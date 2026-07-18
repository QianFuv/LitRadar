//! CNKI metadata source parsing and fixture transport.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::scholarly::{SourceAttempt, SourceError};

const BASE_URL: &str = "https://oversea.cnki.net";
const DEFAULT_PCODE: &str = "CJFD,CCJD";
const CNKI_CHINESE_LANGUAGE: &str = "CHS";
const JOURNAL_PRODUCT_CODE: &str = "BOJHD70J";
const CNKI_RESPONSE_ATTEMPTS: usize = 3;
const CNKI_TRANSPORT_ATTEMPTS: usize = 5;
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DEFAULT_ACCEPT_LANGUAGE: &str = "zh-CN,zh;q=0.9,en;q=0.5";

/// Fixture payload used by CNKI source replay.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CnkiFixtureData {
    /// Journal detail HTML page.
    pub journal_detail_html: String,
    /// Year issue tree HTML.
    pub year_issues_html: String,
    /// Issue article HTML keyed by `year_issue`.
    #[serde(default)]
    pub issue_articles_html: BTreeMap<String, String>,
    /// Article detail HTML keyed by platform id.
    #[serde(default)]
    pub article_detail_html: BTreeMap<String, String>,
    /// Optional endpoint forced to return a parser error.
    #[serde(default)]
    pub fail_endpoint: Option<String>,
}

/// Errors returned by the CNKI source parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CnkiSourceError {
    /// CNKI returned a blocked or verification page.
    Request(String),
    /// HTML could not be parsed into the expected payload.
    Parse(String),
    /// Fixture data is missing a required response.
    MissingFixture(String),
    /// Shared source error.
    Source(SourceError),
}

impl fmt::Display for CnkiSourceError {
    /// Format the CNKI source error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) => formatter.write_str(message),
            Self::Parse(message) => formatter.write_str(message),
            Self::MissingFixture(message) => formatter.write_str(message),
            Self::Source(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for CnkiSourceError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SourceError> for CnkiSourceError {
    /// Convert shared source errors into CNKI source errors.
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

/// CNKI source transport abstraction.
pub trait CnkiTransport {
    /// Fetch one CNKI response body.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - Logical endpoint name.
    /// * `key` - Optional fixture key.
    ///
    /// # Returns
    ///
    /// Response body text.
    fn text(&mut self, endpoint: &str, key: Option<&str>) -> Result<String, CnkiSourceError>;

    /// Resolve one CSV journal row to CNKI journal details.
    ///
    /// # Arguments
    ///
    /// * `row` - Source CSV row.
    ///
    /// # Returns
    ///
    /// Parsed CNKI journal details.
    fn resolve_journal(
        &mut self,
        row: &BTreeMap<String, String>,
    ) -> Result<Option<Value>, CnkiSourceError> {
        let text = self.text("journal_detail", None)?;
        let details = parse_journal_detail(&text)?;
        let title = row.get("title").map(String::as_str).unwrap_or_default();
        let issn = row.get("issn").map(String::as_str).unwrap_or_default();
        if journal_detail_matches(&details, title, issn) {
            Ok(Some(details))
        } else {
            Ok(None)
        }
    }

    /// Fetch publication issues for one journal.
    ///
    /// # Arguments
    ///
    /// * `journal` - CNKI journal details.
    ///
    /// # Returns
    ///
    /// Parsed issue payloads.
    fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, CnkiSourceError> {
        let _ = journal;
        let text = self.text("year_issues", None)?;
        parse_year_issues(&text)
    }

    /// Fetch article summaries for one issue.
    ///
    /// # Arguments
    ///
    /// * `journal` - CNKI journal details.
    /// * `issue` - CNKI issue payload.
    ///
    /// # Returns
    ///
    /// Article summary payloads.
    fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
    ) -> Result<Vec<Value>, CnkiSourceError> {
        let _ = journal;
        let year_issue = json_text(issue.get("year_issue"))
            .ok_or_else(|| CnkiSourceError::Parse("CNKI issue missing year_issue".to_string()))?;
        let text = self.text("issue_articles", Some(&year_issue))?;
        parse_issue_articles(&text, issue)
    }

    /// Fetch one article detail payload.
    ///
    /// # Arguments
    ///
    /// * `article_url` - Article URL from issue summary.
    /// * `platform_id` - Optional platform id from issue summary.
    ///
    /// # Returns
    ///
    /// Article detail payload.
    fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, CnkiSourceError> {
        let key = platform_id.unwrap_or(article_url);
        let text = self.text("article_detail", Some(key))?;
        parse_article_detail(&text, article_url)
    }

    /// Return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured source attempts.
    fn attempts(&self) -> &[SourceAttempt];

    /// Remove and return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured attempts, leaving the transport buffer empty.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt>;

    /// Append attempts captured by cloned worker transports.
    ///
    /// # Arguments
    ///
    /// * `attempts` - Source attempts captured by worker transports.
    fn append_attempts(&mut self, attempts: Vec<SourceAttempt>);
}

/// Deterministic fixture transport for CNKI source tests.
#[derive(Debug, Clone)]
pub struct FixtureCnkiTransport {
    data: CnkiFixtureData,
    attempts: Vec<SourceAttempt>,
}

impl FixtureCnkiTransport {
    /// Build a fixture transport from response data.
    ///
    /// # Arguments
    ///
    /// * `data` - CNKI fixture response payloads.
    ///
    /// # Returns
    ///
    /// Fixture transport.
    pub fn new(data: CnkiFixtureData) -> Self {
        Self {
            data,
            attempts: Vec::new(),
        }
    }

    fn record_attempt(
        &mut self,
        endpoint: &str,
        key: Option<&str>,
        did_succeed: bool,
        error: Option<String>,
    ) {
        self.attempts.push(SourceAttempt {
            service: "cnki".to_string(),
            endpoint: endpoint.to_string(),
            method: if endpoint == "journal_detail" || endpoint == "article_detail" {
                "GET".to_string()
            } else {
                "POST".to_string()
            },
            url: fixture_url(endpoint, key),
            status_code: Some(if did_succeed { 200 } else { 500 }),
            did_succeed,
            did_retry: false,
            error,
        });
    }
}

impl CnkiTransport for FixtureCnkiTransport {
    /// Fetch one CNKI fixture response body.
    fn text(&mut self, endpoint: &str, key: Option<&str>) -> Result<String, CnkiSourceError> {
        if self
            .data
            .fail_endpoint
            .as_deref()
            .is_some_and(|value| value == endpoint)
        {
            let message = format!("CNKI parser fixture failed for {endpoint}");
            self.record_attempt(endpoint, key, false, Some(message.clone()));
            return Err(CnkiSourceError::Parse(message));
        }
        let body = match endpoint {
            "journal_detail" => Some(self.data.journal_detail_html.clone()),
            "year_issues" => Some(self.data.year_issues_html.clone()),
            "issue_articles" => key.and_then(|key| self.data.issue_articles_html.get(key).cloned()),
            "article_detail" => key.and_then(|key| self.data.article_detail_html.get(key).cloned()),
            _ => None,
        }
        .ok_or_else(|| {
            let message = format!("CNKI fixture missing endpoint {endpoint}");
            self.record_attempt(endpoint, key, false, Some(message.clone()));
            CnkiSourceError::MissingFixture(message)
        })?;
        if let Err(error) = checked_text(&body, &fixture_url(endpoint, key)) {
            self.record_attempt(endpoint, key, false, Some(error.to_string()));
            return Err(error);
        }
        self.record_attempt(endpoint, key, true, None);
        Ok(body)
    }

    /// Return captured source attempts.
    fn attempts(&self) -> &[SourceAttempt] {
        &self.attempts
    }

    /// Drain captured fixture attempts.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        std::mem::take(&mut self.attempts)
    }

    /// Append attempts captured by cloned worker transports.
    fn append_attempts(&mut self, attempts: Vec<SourceAttempt>) {
        self.attempts.extend(attempts);
    }
}

/// Live CNKI source transport configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCnkiConfig {
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
}

/// Blocking HTTP transport for live CNKI sources.
#[derive(Debug, Clone)]
pub struct LiveCnkiTransport {
    client: Client,
    attempts: Vec<SourceAttempt>,
}

struct LiveCnkiAttempt<'a> {
    endpoint: &'a str,
    method: &'a str,
    url: &'a str,
    attempt: usize,
    status_code: Option<u16>,
    did_succeed: bool,
    did_retry: bool,
    will_retry: bool,
    error_kind: &'static str,
    duration_ms: u64,
    error: Option<String>,
}

impl LiveCnkiTransport {
    /// Build a live CNKI transport.
    ///
    /// # Arguments
    ///
    /// * `config` - Live source configuration.
    ///
    /// # Returns
    ///
    /// Live CNKI transport.
    pub fn new(config: LiveCnkiConfig) -> Result<Self, CnkiSourceError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.max(1)))
            .redirect(Policy::none())
            .build()
            .map_err(|error| CnkiSourceError::Request(error.to_string()))?;
        Ok(Self {
            client,
            attempts: Vec::new(),
        })
    }

    fn search_journals(
        &mut self,
        field_name: &str,
        value: &str,
        operator: &str,
        search_type: &str,
    ) -> Result<Vec<Value>, CnkiSourceError> {
        let search_state = cnki_journal_search_state(field_name, value, operator);
        let data = vec![
            ("searchStateJson".to_string(), search_state.to_string()),
            ("displaymode".to_string(), "1".to_string()),
            ("pageindex".to_string(), "1".to_string()),
            ("pagecount".to_string(), "21".to_string()),
            ("index".to_string(), String::new()),
            ("searchType".to_string(), search_type.to_string()),
            ("parentcode".to_string(), String::new()),
            ("clickName".to_string(), String::new()),
            ("switchdata".to_string(), "search".to_string()),
        ];
        let text = self.post_text(
            &format!("{BASE_URL}/knavi/journals/searchbaseinfo"),
            &data,
            &[],
            Some(&format!("{BASE_URL}/knavi")),
            "journal_search",
        )?;
        parse_journal_search_results(&text)
    }

    fn get_journal_detail(&mut self, detail_url: &str) -> Result<Option<Value>, CnkiSourceError> {
        let text = self.get_text(
            &with_cnki_chinese_language(detail_url),
            None,
            "journal_detail",
        )?;
        if input_value(&text, "pykm").is_none() {
            return Ok(None);
        }
        parse_journal_detail(&text).map(Some)
    }

    fn get_text(
        &mut self,
        url: &str,
        referer: Option<&str>,
        endpoint: &str,
    ) -> Result<String, CnkiSourceError> {
        self.request_text("GET", url, &[], &[], referer, endpoint)
    }

    fn post_text(
        &mut self,
        url: &str,
        data: &[(String, String)],
        query: &[(String, String)],
        referer: Option<&str>,
        endpoint: &str,
    ) -> Result<String, CnkiSourceError> {
        self.request_text("POST", url, data, query, referer, endpoint)
    }

    fn request_text(
        &mut self,
        method: &str,
        url: &str,
        data: &[(String, String)],
        query: &[(String, String)],
        referer: Option<&str>,
        endpoint: &str,
    ) -> Result<String, CnkiSourceError> {
        let mut response_failure_count = 0;
        let mut transport_failure_count = 0;
        loop {
            let did_retry = response_failure_count + transport_failure_count > 0;
            let attempt = response_failure_count + transport_failure_count + 1;
            let started_at = Instant::now();
            let mut builder = match method {
                "POST" => self.client.post(url).form(data),
                _ => self.client.get(url),
            }
            .query(query)
            .header("User-Agent", DEFAULT_USER_AGENT)
            .header("Accept-Language", DEFAULT_ACCEPT_LANGUAGE);
            if let Some(referer) = referer {
                builder = builder.header("Referer", referer);
            }
            let request = builder
                .build()
                .map_err(|error| CnkiSourceError::Request(error.to_string()))?;
            let request_url = request.url().to_string();
            match self.client.execute(request) {
                Ok(response) => {
                    let status_code = response.status().as_u16();
                    if !(200..300).contains(&status_code) {
                        let message = format!("CNKI request failed with HTTP {status_code}");
                        response_failure_count += 1;
                        let will_retry = response_failure_count < CNKI_RESPONSE_ATTEMPTS;
                        self.record_attempt(LiveCnkiAttempt {
                            endpoint,
                            method,
                            url: &request_url,
                            attempt,
                            status_code: Some(status_code),
                            did_succeed: false,
                            did_retry,
                            will_retry,
                            error_kind: "http_status",
                            duration_ms: elapsed_millis(started_at),
                            error: Some(message.clone()),
                        });
                        if will_retry {
                            thread::sleep(Duration::from_secs(response_failure_count as u64));
                            continue;
                        }
                        return Err(CnkiSourceError::Request(message));
                    }
                    let text = match response.text() {
                        Ok(text) => text,
                        Err(_) => {
                            let message = "CNKI response body decoding failed".to_string();
                            response_failure_count += 1;
                            let will_retry = response_failure_count < CNKI_RESPONSE_ATTEMPTS;
                            self.record_attempt(LiveCnkiAttempt {
                                endpoint,
                                method,
                                url: &request_url,
                                attempt,
                                status_code: Some(status_code),
                                did_succeed: false,
                                did_retry,
                                will_retry,
                                error_kind: "response_body",
                                duration_ms: elapsed_millis(started_at),
                                error: Some(message.clone()),
                            });
                            if will_retry {
                                thread::sleep(Duration::from_secs(response_failure_count as u64));
                                continue;
                            }
                            return Err(CnkiSourceError::Request(message));
                        }
                    };
                    match checked_text(&text, &request_url) {
                        Ok(()) => {
                            self.record_attempt(LiveCnkiAttempt {
                                endpoint,
                                method,
                                url: &request_url,
                                attempt,
                                status_code: Some(status_code),
                                did_succeed: true,
                                did_retry,
                                will_retry: false,
                                error_kind: "none",
                                duration_ms: elapsed_millis(started_at),
                                error: None,
                            });
                            return Ok(text);
                        }
                        Err(error) => {
                            response_failure_count += 1;
                            let will_retry = response_failure_count < CNKI_RESPONSE_ATTEMPTS;
                            self.record_attempt(LiveCnkiAttempt {
                                endpoint,
                                method,
                                url: &request_url,
                                attempt,
                                status_code: Some(status_code),
                                did_succeed: false,
                                did_retry,
                                will_retry,
                                error_kind: "invalid_response",
                                duration_ms: elapsed_millis(started_at),
                                error: Some(error.to_string()),
                            });
                            if will_retry {
                                thread::sleep(Duration::from_secs(response_failure_count as u64));
                                continue;
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) => {
                    transport_failure_count += 1;
                    let will_retry = transport_failure_count < CNKI_TRANSPORT_ATTEMPTS;
                    self.record_attempt(LiveCnkiAttempt {
                        endpoint,
                        method,
                        url,
                        attempt,
                        status_code: None,
                        did_succeed: false,
                        did_retry,
                        will_retry,
                        error_kind: "transport",
                        duration_ms: elapsed_millis(started_at),
                        error: Some(error.to_string()),
                    });
                    if will_retry {
                        thread::sleep(Duration::from_secs(transport_failure_count as u64));
                        continue;
                    }
                    return Err(CnkiSourceError::Request(error.to_string()));
                }
            }
        }
    }

    fn record_attempt(&mut self, attempt: LiveCnkiAttempt<'_>) {
        if !attempt.did_succeed {
            tracing::warn!(
                event = "source.request.failed",
                component = "source",
                provider = "cnki",
                endpoint = attempt.endpoint,
                method = attempt.method,
                attempt = attempt.attempt,
                outcome = "failure",
                error_kind = attempt.error_kind,
                http_status = attempt.status_code.unwrap_or(0),
                has_http_status = attempt.status_code.is_some(),
                is_retry = attempt.did_retry,
                will_retry = attempt.will_retry,
                duration_ms = attempt.duration_ms,
            );
        }
        self.attempts.push(SourceAttempt {
            service: "cnki".to_string(),
            endpoint: attempt.endpoint.to_string(),
            method: attempt.method.to_string(),
            url: attempt.url.to_string(),
            status_code: attempt.status_code,
            did_succeed: attempt.did_succeed,
            did_retry: attempt.did_retry,
            error: attempt.error,
        });
    }
}

impl CnkiTransport for LiveCnkiTransport {
    /// Fetch one CNKI response body by endpoint.
    fn text(&mut self, endpoint: &str, _key: Option<&str>) -> Result<String, CnkiSourceError> {
        Err(CnkiSourceError::Request(format!(
            "live CNKI endpoint {endpoint} requires typed context"
        )))
    }

    /// Resolve one CSV journal row through CNKI search and detail pages.
    fn resolve_journal(
        &mut self,
        row: &BTreeMap<String, String>,
    ) -> Result<Option<Value>, CnkiSourceError> {
        let title = row
            .get("title")
            .map(String::as_str)
            .unwrap_or_default()
            .trim();
        let issn = row
            .get("issn")
            .map(String::as_str)
            .unwrap_or_default()
            .trim();
        if !title.is_empty() {
            for candidate in self.search_journals("TI", title, "%", "刊名(曾用刊名)")? {
                let Some(detail_url) = json_text(candidate.get("detail_url")) else {
                    continue;
                };
                let Some(details) = self.get_journal_detail(&detail_url)? else {
                    continue;
                };
                if journal_detail_matches(&details, title, issn) {
                    return Ok(Some(details));
                }
            }
        }
        if !issn.is_empty() {
            for candidate in self.search_journals("SN", issn, "=", "ISSN")? {
                let Some(detail_url) = json_text(candidate.get("detail_url")) else {
                    continue;
                };
                let Some(details) = self.get_journal_detail(&detail_url)? else {
                    continue;
                };
                if journal_detail_matches(&details, title, issn) {
                    return Ok(Some(details));
                }
            }
        }
        Ok(None)
    }

    /// Fetch publication issues for one CNKI journal.
    fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, CnkiSourceError> {
        let pykm = json_text(journal.get("pykm"))
            .ok_or_else(|| CnkiSourceError::Parse("CNKI journal missing pykm".to_string()))?;
        let data = vec![
            ("pIdx".to_string(), "0".to_string()),
            (
                "time".to_string(),
                json_text(journal.get("time")).unwrap_or_default(),
            ),
            ("isEpublish".to_string(), String::new()),
            (
                "pcode".to_string(),
                json_text(journal.get("pcode")).unwrap_or_else(|| DEFAULT_PCODE.to_string()),
            ),
        ];
        let text = self.post_text(
            &format!("{BASE_URL}/knavi/journals/{pykm}/yearList"),
            &data,
            &[],
            json_text(journal.get("detail_url")).as_deref(),
            "year_issues",
        )?;
        parse_year_issues(&text)
    }

    /// Fetch article summaries for one issue.
    fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
    ) -> Result<Vec<Value>, CnkiSourceError> {
        let pykm = json_text(journal.get("pykm"))
            .ok_or_else(|| CnkiSourceError::Parse("CNKI journal missing pykm".to_string()))?;
        let year_issue = json_text(issue.get("year_issue"))
            .ok_or_else(|| CnkiSourceError::Parse("CNKI issue missing year_issue".to_string()))?;
        let query = vec![
            ("yearIssue".to_string(), year_issue),
            ("pageIdx".to_string(), "0".to_string()),
            (
                "pcode".to_string(),
                json_text(journal.get("pcode")).unwrap_or_else(|| DEFAULT_PCODE.to_string()),
            ),
            ("isEpublish".to_string(), String::new()),
            ("language".to_string(), CNKI_CHINESE_LANGUAGE.to_string()),
        ];
        let text = self.post_text(
            &format!("{BASE_URL}/knavi/journals/{pykm}/papers"),
            &[],
            &query,
            json_text(journal.get("detail_url")).as_deref(),
            "issue_articles",
        )?;
        parse_issue_articles(&text, issue)
    }

    /// Fetch one article detail payload.
    fn article_detail(
        &mut self,
        article_url: &str,
        _platform_id: Option<&str>,
    ) -> Result<Value, CnkiSourceError> {
        let resolved_url = with_cnki_chinese_language(article_url);
        let text = self.get_text(&resolved_url, Some(BASE_URL), "article_detail")?;
        parse_article_detail(&text, &resolved_url)
    }

    /// Return captured source attempts.
    fn attempts(&self) -> &[SourceAttempt] {
        &self.attempts
    }

    /// Drain captured live attempts.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        std::mem::take(&mut self.attempts)
    }

    /// Append attempts captured by cloned worker transports.
    fn append_attempts(&mut self, attempts: Vec<SourceAttempt>) {
        self.attempts.extend(attempts);
    }
}

/// CNKI metadata client using a transport implementation.
#[derive(Debug, Clone)]
pub struct CnkiClient<T> {
    transport: T,
}

impl<T> CnkiClient<T>
where
    T: CnkiTransport,
{
    /// Build a CNKI client from a transport.
    ///
    /// # Arguments
    ///
    /// * `transport` - Source transport.
    ///
    /// # Returns
    ///
    /// CNKI client.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Resolve one CSV journal row to CNKI journal details.
    ///
    /// # Arguments
    ///
    /// * `row` - Source CSV row.
    ///
    /// # Returns
    ///
    /// Parsed CNKI journal details.
    pub fn resolve_journal(
        &mut self,
        row: &BTreeMap<String, String>,
    ) -> Result<Option<Value>, CnkiSourceError> {
        self.transport.resolve_journal(row)
    }

    /// Fetch publication issues for one journal.
    ///
    /// # Arguments
    ///
    /// * `journal` - CNKI journal details.
    ///
    /// # Returns
    ///
    /// Parsed issue payloads.
    pub fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, CnkiSourceError> {
        self.transport.year_issues(journal)
    }

    /// Fetch article summaries for one issue.
    ///
    /// # Arguments
    ///
    /// * `journal` - CNKI journal details.
    /// * `issue` - CNKI issue payload.
    ///
    /// # Returns
    ///
    /// Article summary payloads.
    pub fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
    ) -> Result<Vec<Value>, CnkiSourceError> {
        self.transport.issue_articles(journal, issue)
    }

    /// Fetch one article detail payload.
    ///
    /// # Arguments
    ///
    /// * `article_url` - Article URL from issue summary.
    /// * `platform_id` - Optional platform id from issue summary.
    ///
    /// # Returns
    ///
    /// Article detail payload.
    pub fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, CnkiSourceError> {
        self.transport.article_detail(article_url, platform_id)
    }

    /// Return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured source attempts.
    pub fn attempts(&self) -> &[SourceAttempt] {
        self.transport.attempts()
    }

    /// Remove and return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured attempts, leaving the client buffer empty.
    pub fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        self.transport.drain_attempts()
    }

    /// Append attempts captured by cloned worker clients.
    ///
    /// # Arguments
    ///
    /// * `attempts` - Source attempts captured by worker clients.
    pub fn append_attempts(&mut self, attempts: Vec<SourceAttempt>) {
        self.transport.append_attempts(attempts);
    }
}

/// Parse one CNKI journal detail HTML page.
///
/// # Arguments
///
/// * `text` - Journal detail HTML.
///
/// # Returns
///
/// Journal detail payload.
pub fn parse_journal_detail(text: &str) -> Result<Value, CnkiSourceError> {
    checked_text(text, "journal_detail")?;
    let pykm = input_value(text, "pykm")
        .ok_or_else(|| CnkiSourceError::Parse("CNKI journal detail missing pykm".to_string()))?;
    let pcode = input_value(text, "pCode").unwrap_or_else(|| DEFAULT_PCODE.to_string());
    let visible_text = strip_tags(text);
    Ok(json!({
        "detail_url": with_cnki_chinese_language(&format!("{BASE_URL}/knavi/detail?pykm={pykm}")),
        "pykm": pykm,
        "pcode": pcode,
        "time": input_value(text, "time"),
        "title": input_value(text, "shareChName").or_else(|| title_text(text)),
        "issn": label_value(&visible_text, &["ISSN"]),
        "cn": label_value(&visible_text, &["CN"]),
        "impact_factor": label_value(&visible_text, &["Combined IF", "复合影响因子"]),
        "cover_url": image_url(text),
        "raw_text": visible_text,
    }))
}

/// Parse CNKI year issue tree HTML.
///
/// # Arguments
///
/// * `text` - Year issue HTML.
///
/// # Returns
///
/// Parsed issue payloads.
pub fn parse_year_issues(text: &str) -> Result<Vec<Value>, CnkiSourceError> {
    checked_text(text, "year_issues")?;
    let mut issues = Vec::new();
    for tag in tags(text, "a") {
        let attrs = attrs(&tag);
        let element_id = attrs.get("id").cloned().unwrap_or_default();
        if !element_id.starts_with("yq") {
            continue;
        }
        let key = &element_id[2..];
        let Some(year) = key.get(..4).and_then(|value| value.parse::<i64>().ok()) else {
            continue;
        };
        let label = strip_tags(&tag);
        let Some(year_issue) = attrs.get("value").cloned() else {
            continue;
        };
        issues.push(json!({
            "year": year,
            "number": issue_number(key, &label),
            "title": label,
            "year_issue": decode_html(&year_issue),
        }));
    }
    Ok(issues)
}

/// Parse CNKI article rows for one issue.
///
/// # Arguments
///
/// * `text` - Issue article HTML.
/// * `issue` - Issue payload.
///
/// # Returns
///
/// Article summary payloads.
pub fn parse_issue_articles(text: &str, issue: &Value) -> Result<Vec<Value>, CnkiSourceError> {
    checked_text(text, "issue_articles")?;
    let mut articles = Vec::new();
    let mut current_section = String::new();
    let mut cursor = 0;
    while let Some((start, tag_name)) = next_article_block(text, cursor) {
        if tag_name == "dt" {
            if let Some((block, end)) = tag_block_at(text, "dt", start) {
                current_section = strip_tags(&block);
                cursor = end;
            } else {
                break;
            }
        } else if let Some((block, end)) = tag_block_at(text, "dd", start) {
            if let Some(article) = parse_article_row(&block, issue, &current_section) {
                articles.push(article);
            }
            cursor = end;
        } else {
            break;
        }
    }
    Ok(articles)
}

/// Parse one CNKI article detail HTML page.
///
/// # Arguments
///
/// * `text` - Article detail HTML.
/// * `article_url` - Original article URL.
///
/// # Returns
///
/// Article detail payload.
pub fn parse_article_detail(text: &str, article_url: &str) -> Result<Value, CnkiSourceError> {
    checked_text(text, article_url)?;
    let filename =
        input_value(text, "paramfilename").or_else(|| input_value(text, "param-filename"));
    let dbcode = input_value(text, "paramdbcode").or_else(|| input_value(text, "param-dbcode"));
    let dbname = input_value(text, "paramdbname").or_else(|| input_value(text, "param-dbname"));
    let title = first_block_text(text, "<p", "title-one").or_else(|| title_text(text));
    let online_time =
        row_value(text, "在线公开时间").or_else(|| row_value(text, "Online Release Time"));
    let permalink = article_detail_url(dbcode.as_deref(), dbname.as_deref(), filename.as_deref())
        .unwrap_or_else(|| with_cnki_chinese_language(article_url));
    Ok(json!({
        "article_url": with_cnki_chinese_language(article_url),
        "platform_id": filename,
        "dbcode": dbcode,
        "dbname": dbname,
        "title": title,
        "authors": author_text(text),
        "abstract": input_value(text, "abstract_text"),
        "doi": row_value(text, "DOI"),
        "online_release_date": online_time.and_then(|value| date_part(&value)),
        "pages": label_value(&strip_tags(text), &["页码", "Pages"]),
        "html_read_url": link_with_text(text, "HTML阅读"),
        "permalink": permalink,
        "content_location": permalink,
    }))
}

/// Validate a CNKI response text.
///
/// # Arguments
///
/// * `text` - Response text.
/// * `url` - Request URL or fixture key.
///
/// # Returns
///
/// Ok when the response appears usable.
pub fn checked_text(text: &str, url: &str) -> Result<(), CnkiSourceError> {
    let lowered = text.to_lowercase();
    if (lowered.contains("captcha") || text.contains("访问异常") || text.contains("安全验证"))
        && !looks_like_cnki_content(text)
    {
        return Err(CnkiSourceError::Request(format!(
            "CNKI verification required: {url}"
        )));
    }
    Ok(())
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn cnki_journal_search_state(field_name: &str, value: &str, operator: &str) -> Value {
    json!({
        "StateID": "",
        "Platfrom": "",
        "QueryTime": "",
        "Account": "knavi",
        "ClientToken": "",
        "Language": "",
        "CNode": {
            "PCode": JOURNAL_PRODUCT_CODE,
            "SMode": "",
            "OperateT": 0
        },
        "QNode": {
            "SelectT": "",
            "Select_Fields": "",
            "S_DBCodes": "",
            "Subscribed": "",
            "QGroup": [{
                "Key": "subject",
                "Logic": 1,
                "Items": [{
                    "Key": "txt_1",
                    "Title": "",
                    "Logic": 1,
                    "Name": field_name,
                    "Operate": operator,
                    "Value": value,
                    "ExtendType": 0,
                    "ExtendValue": "",
                    "Value2": ""
                }],
                "ChildItems": []
            }],
            "OrderBy": "OTA|DESC",
            "GroupBy": "",
            "Additon": ""
        }
    })
}

fn parse_journal_search_results(text: &str) -> Result<Vec<Value>, CnkiSourceError> {
    checked_text(text, "journal_search")?;
    let mut candidates = Vec::new();
    let mut seen = Vec::<String>::new();
    for tag in tags(text, "a") {
        let attrs = attrs(&tag);
        let Some(href) = attrs.get("href") else {
            continue;
        };
        if !href.contains("/knavi/detail?") {
            continue;
        }
        let detail_url = absolute_url(href);
        if seen.iter().any(|value| value == &detail_url) {
            continue;
        }
        seen.push(detail_url.clone());
        candidates.push(json!({
            "detail_url": detail_url,
            "title": strip_tags(&tag),
        }));
    }
    Ok(candidates)
}

fn parse_article_row(row_html: &str, issue: &Value, section: &str) -> Option<Value> {
    let anchor = tags(row_html, "a").into_iter().find(|tag| {
        attrs(tag)
            .get("href")
            .is_some_and(|href| href.contains("/kcms2/article/abstract?"))
    })?;
    let anchor_attrs = attrs(&anchor);
    let href = anchor_attrs.get("href")?;
    let article_url = with_cnki_chinese_language(&absolute_url(href));
    let platform_id = tags(row_html, "b").into_iter().find_map(|tag| {
        let attrs = attrs(&tag);
        (attrs.get("name").is_some_and(|value| value == "encrypt"))
            .then(|| attrs.get("id").cloned())
            .flatten()
    });
    let year = issue
        .get("year")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    Some(json!({
        "article_url": article_url,
        "platform_id": platform_id,
        "title": strip_tags(&anchor),
        "authors": span_title(row_html, "author"),
        "pages": span_title(row_html, "company"),
        "section": section,
        "is_free": if strip_tags(row_html).contains("免费") || row_html.contains("Free") { 1 } else { 0 },
        "date": format!("{year:04}-01-01"),
    }))
}

fn journal_detail_matches(details: &Value, title: &str, issn: &str) -> bool {
    let detail_title = json_text(details.get("title")).unwrap_or_default();
    if !title.trim().is_empty() {
        normalize_title(title) == normalize_title(&detail_title)
            || json_text(details.get("raw_text"))
                .map(|text| normalize_title(&text).contains(&normalize_title(title)))
                .unwrap_or(false)
    } else {
        !issn.trim().is_empty()
            && normalize_issn(issn)
                == normalize_issn(&json_text(details.get("issn")).unwrap_or_default())
    }
}

fn tags(text: &str, tag_name: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut cursor = 0;
    while let Some((block, end)) = find_tag_block(text, tag_name, cursor) {
        tags.push(block);
        cursor = end;
    }
    tags
}

fn find_tag_block(text: &str, tag_name: &str, from: usize) -> Option<(String, usize)> {
    let start = text[from..].find(&format!("<{tag_name}"))? + from;
    tag_block_at(text, tag_name, start)
}

fn tag_block_at(text: &str, tag_name: &str, start: usize) -> Option<(String, usize)> {
    let open_end = text[start..].find('>')? + start + 1;
    let close_marker = format!("</{tag_name}>");
    let close_start = text[open_end..].find(&close_marker)? + open_end;
    let end = close_start + close_marker.len();
    Some((text[start..end].to_string(), end))
}

fn next_article_block(text: &str, from: usize) -> Option<(usize, &'static str)> {
    let dt = text[from..].find("<dt").map(|index| (from + index, "dt"));
    let dd = text[from..].find("<dd").map(|index| (from + index, "dd"));
    match (dt, dd) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn attrs(tag: &str) -> BTreeMap<String, String> {
    let header = tag.split('>').next().unwrap_or(tag);
    let mut output = BTreeMap::new();
    for quote in ['"', '\''] {
        let mut cursor = 0;
        while let Some(equals_index) = header[cursor..].find('=') {
            let equals_index = cursor + equals_index;
            if !header[equals_index + 1..].starts_with(quote) {
                cursor = equals_index + 1;
                continue;
            }
            let key_start = header[..equals_index]
                .rfind(|character: char| character.is_whitespace() || character == '<')
                .map(|index| index + 1)
                .unwrap_or(0);
            let key = header[key_start..equals_index].trim().to_lowercase();
            let value_start = equals_index + 2;
            let Some(value_end) = header[value_start..]
                .find(quote)
                .map(|index| value_start + index)
            else {
                break;
            };
            if !key.is_empty() {
                output.insert(key, decode_html(&header[value_start..value_end]));
            }
            cursor = value_end + 1;
        }
    }
    output
}

fn input_value(text: &str, element_id: &str) -> Option<String> {
    start_tags(text, "input").into_iter().find_map(|tag| {
        let attrs = attrs(&tag);
        (attrs.get("id").is_some_and(|value| value == element_id))
            .then(|| attrs.get("value").cloned())
            .flatten()
            .and_then(|value| non_empty(&value))
    })
}

fn span_title(text: &str, class_name: &str) -> Option<String> {
    tags(text, "span").into_iter().find_map(|tag| {
        let attrs = attrs(&tag);
        attrs
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class_name))
            .then(|| attrs.get("title").cloned())
            .flatten()
            .and_then(|value| clean_text(&value))
    })
}

fn author_text(text: &str) -> Option<String> {
    let block = tags(text, "h3").into_iter().find(|tag| {
        let attrs = attrs(tag);
        attrs.get("id").is_some_and(|value| value == "authorpart")
            && attrs
                .get("class")
                .is_some_and(|value| value.split_whitespace().any(|item| item == "author"))
    })?;
    let names = tags(&block, "span")
        .into_iter()
        .filter_map(|tag| non_empty(&strip_tags(&tag)))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("; "))
}

fn row_value(text: &str, label: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find("<span").map(|index| cursor + index) {
        let Some((span, end)) = tag_block_at(text, "span", start) else {
            break;
        };
        let span_attrs = attrs(&span);
        if span_attrs
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == "rowtit"))
            && strip_tags(&span)
                .trim()
                .trim_end_matches([':', '：'])
                .trim()
                == label
        {
            if let Some((paragraph, _)) = find_tag_block(text, "p", end) {
                return non_empty(&strip_tags(&paragraph));
            }
        }
        cursor = end;
    }
    None
}

fn first_block_text(text: &str, tag_prefix: &str, class_name: &str) -> Option<String> {
    let tag_name = tag_prefix.trim_start_matches('<');
    tags(text, tag_name).into_iter().find_map(|tag| {
        attrs(&tag)
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class_name))
            .then(|| non_empty(&strip_tags(&tag)))
            .flatten()
    })
}

fn link_with_text(text: &str, label: &str) -> Option<String> {
    tags(text, "a").into_iter().find_map(|tag| {
        strip_tags(&tag).contains(label).then(|| {
            attrs(&tag)
                .get("href")
                .map(|href| with_cnki_chinese_language(&absolute_url(href)))
        })?
    })
}

fn article_detail_url(
    dbcode: Option<&str>,
    dbname: Option<&str>,
    filename: Option<&str>,
) -> Option<String> {
    Some(with_cnki_chinese_language(&format!(
        "{BASE_URL}/openlink/detail?dbcode={}&dbname={}&filename={}&uniplatform=OVERSEA&language={CNKI_CHINESE_LANGUAGE}",
        dbcode?,
        dbname?,
        filename?
    )))
}

fn with_cnki_chinese_language(url: &str) -> String {
    if !url.contains("oversea.cnki.net")
        && !url.starts_with("/kcms")
        && !url.starts_with("/knavi")
        && !url.starts_with("/openlink")
    {
        return url.to_string();
    }
    let absolute = absolute_url(url);
    let mut parts = absolute.splitn(2, '?');
    let path = parts.next().unwrap_or_default();
    let query = parts.next().unwrap_or_default();
    let mut pairs = query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter(|part| {
            let key = part.split('=').next().unwrap_or_default().to_lowercase();
            key != "language" && key != "uniplatform"
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    pairs.push("uniplatform=OVERSEA".to_string());
    pairs.push(format!("language={CNKI_CHINESE_LANGUAGE}"));
    format!("{path}?{}", pairs.join("&"))
}

fn checked_marker_text(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

fn looks_like_cnki_content(text: &str) -> bool {
    checked_marker_text(
        text,
        &[
            "id=\"abstract_text\"",
            "id=\"pykm\"",
            "id=\"YearIssueTree\"",
            "class=\"name\"",
            "/knavi/detail?",
        ],
    )
}

fn image_url(text: &str) -> Option<String> {
    start_tags(text, "img").into_iter().find_map(|tag| {
        attrs(&tag).get("src").and_then(|source| {
            (source.to_lowercase().contains("cover") || source.to_lowercase().contains("journal"))
                .then(|| absolute_url(source))
        })
    })
}

fn start_tags(text: &str, tag_name: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut cursor = 0;
    let marker = format!("<{tag_name}");
    while let Some(start) = text[cursor..].find(&marker).map(|index| cursor + index) {
        let Some(end) = text[start..].find('>').map(|index| start + index + 1) else {
            break;
        };
        output.push(text[start..end].to_string());
        cursor = end;
    }
    output
}

fn title_text(text: &str) -> Option<String> {
    let title = tags(text, "title")
        .into_iter()
        .find_map(|tag| non_empty(&strip_tags(&tag)))?;
    non_empty(title.trim_end_matches(" - 中国知网")).or(Some(title))
}

fn label_value(text: &str, labels: &[&str]) -> Option<String> {
    for label in labels {
        for separator in [":", "："] {
            let marker = format!("{label}{separator}");
            if let Some(index) = text.find(&marker) {
                let start = index + marker.len();
                let value = text[start..]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .trim_matches([';', ',', '，', '；']);
                if let Some(value) = non_empty(value) {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn issue_number(key: &str, label: &str) -> String {
    let suffix = key.get(4..).unwrap_or_default();
    if !suffix.is_empty() {
        let trimmed = suffix.trim_start_matches('0');
        return if trimmed.is_empty() { "0" } else { trimmed }.to_string();
    }
    label
        .split_whitespace()
        .find(|part| part.chars().any(|character| character.is_ascii_digit()))
        .unwrap_or(label)
        .to_string()
}

fn date_part(value: &str) -> Option<String> {
    non_empty(value).map(|value| value.chars().take(10).collect())
}

fn strip_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => {
                inside_tag = true;
                output.push(' ');
            }
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    clean_text(&decode_html(&output)).unwrap_or_default()
}

fn clean_text(value: &str) -> Option<String> {
    non_empty(
        &decode_html(value)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn decode_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn json_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(value) => non_empty(value),
        other => non_empty(&other.to_string()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn absolute_url(value: &str) -> String {
    if value.starts_with("http://") || value.starts_with("https://") {
        value.to_string()
    } else if value.starts_with('/') {
        format!("{BASE_URL}{value}")
    } else {
        format!("{BASE_URL}/{value}")
    }
}

fn normalize_title(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_issn(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_digit() || *character == 'X' || *character == 'x')
        .flat_map(char::to_uppercase)
        .collect()
}

fn fixture_url(endpoint: &str, key: Option<&str>) -> String {
    match (endpoint, key) {
        ("issue_articles", Some(key)) => {
            format!("{BASE_URL}/knavi/journals/TEST/papers?yearIssue={key}")
        }
        ("article_detail", Some(key)) => {
            format!("{BASE_URL}/kcms2/article/abstract?filename={key}")
        }
        ("year_issues", _) => format!("{BASE_URL}/knavi/journals/TEST/yearList"),
        ("journal_detail", _) => format!("{BASE_URL}/knavi/detail?pykm=TEST"),
        _ => format!("{BASE_URL}/{endpoint}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use serde_json::{json, Value};

    use crate::scholarly::test_support::CapturedLogs;

    use super::{
        absolute_url, checked_text, decode_html, journal_detail_matches, parse_article_detail,
        parse_issue_articles, parse_journal_detail, parse_journal_search_results,
        parse_year_issues, with_cnki_chinese_language, CnkiClient, CnkiFixtureData,
        CnkiSourceError, FixtureCnkiTransport, LiveCnkiAttempt, LiveCnkiConfig, LiveCnkiTransport,
    };

    const TEST_SERVER_EVENT_TIMEOUT: Duration = Duration::from_secs(3);

    #[test]
    fn cnki_attempt_events_keep_worker_context_and_omit_request_material() {
        let sentinel = "cnki-url-cookie-body-sentinel";
        let mut transport = live_cnki_transport();
        let logs = CapturedLogs::default();

        tracing::subscriber::with_default(logs.subscriber(), || {
            let span = tracing::info_span!(
                "index.worker",
                run_id = "run-cnki-correlation",
                worker_id = 4,
            );
            span.in_scope(|| {
                transport.record_attempt(LiveCnkiAttempt {
                    endpoint: "article_detail",
                    method: "GET",
                    url: sentinel,
                    attempt: 2,
                    status_code: Some(503),
                    did_succeed: false,
                    did_retry: true,
                    will_retry: true,
                    error_kind: "http_status",
                    duration_ms: 17,
                    error: Some(sentinel.to_string()),
                });
            });
        });

        let events = logs.events();
        assert_eq!(events.len(), 1);
        let failed = &events[0];
        assert_eq!(failed["event"], "source.request.failed");
        assert_eq!(failed["provider"], "cnki");
        assert_eq!(failed["endpoint"], "article_detail");
        assert_eq!(failed["attempt"], 2);
        assert_eq!(failed["duration_ms"], 17);
        assert_eq!(failed["span"]["run_id"], "run-cnki-correlation");
        assert_eq!(failed["span"]["worker_id"], 4);
        assert!(!logs.text().contains(sentinel));
    }

    #[test]
    fn parses_cnki_journal_issue_and_article_html() {
        let journal = parse_journal_detail(
            r#"
            <html><head><title>CNKI Test Journal - 中国知网</title></head>
            <body>
              <input id="pykm" value="TEST" />
              <input id="pCode" value="CJFD" />
              <input id="time" value="token" />
              <input id="shareChName" value="CNKI Test Journal" />
              <p>ISSN: 1234-5678</p><p>Combined IF: 1.5</p>
              <img src="/images/journal-cover.jpg" />
            </body></html>
            "#,
        )
        .expect("journal detail should parse");
        let issues = parse_year_issues(
            r#"<div id="YearIssueTree"><a id="yq202601" value="202601">2026 No.01</a></div>"#,
        )
        .expect("issues should parse");
        let articles = parse_issue_articles(
            r#"
            <dt class="tit">Articles</dt>
            <dd class="row">
              <a href="/kcms2/article/abstract?v=1&filename=CNKI202601001">CNKI article CNKI202601001</a>
              <b name="encrypt" id="CNKI202601001"></b>
              <span class="author" title="Test Author"></span>
              <span class="company" title="1-2"></span>
              Free
            </dd>
            "#,
            &issues[0],
        )
        .expect("article summaries should parse");
        let detail = parse_article_detail(
            r#"
            <html><head><title>CNKI article CNKI202601001</title></head>
            <body>
              <input id="paramfilename" value="CNKI202601001" />
              <input id="paramdbcode" value="CJFD" />
              <input id="paramdbname" value="CJFDLAST2026" />
              <input id="abstract_text" value="Test abstract." />
              <p class="title-one">CNKI article CNKI202601001</p>
              <h3 class="author" id="authorpart"><span>Test Author</span></h3>
              <span class="rowtit">Online Release Time:</span><p>2026-01-02</p>
              <span class="rowtit">DOI:</span><p>10.1/cnki</p>
              <span class="rowtit">Pages:</span><p>1-2</p>
              <a href="/barnew/download/order?id=abc">HTML阅读</a>
            </body></html>
            "#,
            "https://oversea.cnki.net/kcms2/article/abstract?v=1&filename=CNKI202601001",
        )
        .expect("article detail should parse");

        assert_eq!(journal["pykm"], "TEST");
        assert_eq!(issues[0]["year"], 2026);
        assert_eq!(articles[0]["is_free"], 1);
        assert_eq!(detail["platform_id"], "CNKI202601001");
        assert_eq!(detail["authors"], "Test Author");
    }

    #[test]
    fn cnki_url_and_text_helpers_cover_language_normalization_and_decoding() {
        assert_eq!(
            with_cnki_chinese_language("https://example.test/article?language=en"),
            "https://example.test/article?language=en"
        );
        assert_eq!(
            with_cnki_chinese_language("/kcms2/article/abstract?v=1&language=en&uniplatform=OLD"),
            "https://oversea.cnki.net/kcms2/article/abstract?v=1&uniplatform=OVERSEA&language=CHS"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/knavi/detail?pykm=TEST"),
            "https://oversea.cnki.net/knavi/detail?pykm=TEST&uniplatform=OVERSEA&language=CHS"
        );
        assert_eq!(
            absolute_url("kcms/detail"),
            "https://oversea.cnki.net/kcms/detail"
        );
        assert_eq!(decode_html("&lt;A&amp;B&gt;&quot;&#39;"), "<A&B>\"'");
    }

    #[test]
    fn cnki_search_and_journal_matching_cover_dedup_and_issn_fallbacks() {
        let search_results = parse_journal_search_results(
            r#"
            <a href="/knavi/detail?pykm=TEST">CNKI &amp; Test Journal</a>
            <a href="/knavi/detail?pykm=TEST">Duplicate</a>
            <a href="/other">Ignored</a>
            "#,
        )
        .expect("search results should parse");
        let detail = parse_journal_detail(
            r#"
            <html><head><title>CNKI Test Journal - 中国知网</title></head>
            <body>
              <input id="pykm" value="TEST" />
              <input id="shareChName" value="CNKI Test Journal" />
              <p>ISSN: 1234-567X</p>
            </body></html>
            "#,
        )
        .expect("journal detail should parse");

        assert_eq!(search_results.len(), 1);
        assert_eq!(
            search_results[0]["detail_url"],
            "https://oversea.cnki.net/knavi/detail?pykm=TEST"
        );
        assert_eq!(search_results[0]["title"], "CNKI & Test Journal");
        assert!(journal_detail_matches(&detail, "CNKI Test Journal", ""));
        assert!(journal_detail_matches(&detail, "", "1234-567x"));
        assert!(!journal_detail_matches(&detail, "Other Journal", ""));
    }

    #[test]
    fn cnki_year_issue_and_detail_parsers_cover_fallback_variants() {
        let issues = parse_year_issues(
            r#"
            <div id="YearIssueTree">
              <a id="bad" value="ignored">Bad</a>
              <a id="yq202600" value="2026&amp;00">Supplement</a>
              <a id="yq202612" value="202612">2026 No.12</a>
            </div>
            "#,
        )
        .expect("year issues should parse");
        let detail = parse_article_detail(
            r#"
            <html><head><title>Fallback Detail Title</title></head>
            <body>
              <h3 class="author" id="authorpart">
                <span>Alice &amp; Bob</span><span>Carol</span>
              </h3>
              <span class="rowtit">在线公开时间：</span><p>2026-02-03 12:00</p>
              <span class="rowtit">页码：</span><p>5-6</p>
            </body></html>
            "#,
            "/kcms2/article/abstract?v=1&filename=FALLBACK&language=en",
        )
        .expect("fallback detail should parse");

        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0]["number"], "0");
        assert_eq!(issues[0]["year_issue"], "2026&00");
        assert_eq!(issues[1]["number"], "12");
        assert_eq!(detail["platform_id"], Value::Null);
        assert_eq!(detail["authors"], "Alice & Bob; Carol");
        assert_eq!(detail["online_release_date"], "2026-02-03");
        assert_eq!(
            detail["permalink"],
            "https://oversea.cnki.net/kcms2/article/abstract?v=1&filename=FALLBACK&uniplatform=OVERSEA&language=CHS"
        );
    }

    #[test]
    fn verification_pages_fail_loud() {
        let error = checked_text("<html>captcha 安全验证</html>", "blocked")
            .expect_err("verification page should fail");

        assert!(error.to_string().contains("verification required"));
    }

    #[test]
    fn live_cnki_extends_transport_retries_beyond_response_failures() {
        let server = TestHttpServer::start(vec![
            String::new(),
            String::new(),
            String::new(),
            ok_response(),
        ]);
        let mut transport = live_cnki_transport();

        let result = transport.get_text(server.url(), None, "transport_test");
        let served_count = server.finish();
        assert!(
            result.is_ok(),
            "fourth response should recover after three transport failures: {result:?}; served: {served_count}; attempts: {:?}",
            transport.attempts
        );
        let text = result.expect("successful transport retry should contain text");

        assert_eq!(text, "ok");
        assert_eq!(served_count, 4);
        assert_eq!(transport.attempts.len(), 4);
        assert!(transport.attempts[..3]
            .iter()
            .all(|attempt| { attempt.status_code.is_none() && !attempt.did_succeed }));
        assert_eq!(
            transport
                .attempts
                .iter()
                .map(|attempt| attempt.did_retry)
                .collect::<Vec<_>>(),
            [false, true, true, true]
        );
        assert!(transport
            .attempts
            .last()
            .is_some_and(|attempt| attempt.status_code == Some(200) && attempt.did_succeed));
    }

    #[test]
    fn live_cnki_stops_after_five_transport_failures() {
        let server = TestHttpServer::start(vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]);
        let mut transport = live_cnki_transport();

        let error = transport
            .get_text(server.url(), None, "transport_test")
            .expect_err("five transport failures should fail loud");
        let served_count = server.finish();

        assert_eq!(served_count, 5);
        assert_eq!(transport.attempts.len(), 5);
        assert!(transport
            .attempts
            .iter()
            .all(|attempt| { attempt.status_code.is_none() && !attempt.did_succeed }));
        assert_eq!(
            transport
                .attempts
                .iter()
                .map(|attempt| attempt.did_retry)
                .collect::<Vec<_>>(),
            [false, true, true, true, true]
        );
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn live_cnki_retries_a_2xx_decode_failure_then_records_success() {
        let server = TestHttpServer::start(vec![malformed_gzip_response(200), ok_response()]);
        let request_url = format!("{}?api_key=decode-secret", server.url());
        let mut transport = live_cnki_transport();

        let result = transport.get_text(&request_url, None, "decode_test");
        let served_count = server.finish();
        assert!(
            result.is_ok(),
            "second response should recover from decoding failure: {result:?}; served: {served_count}; attempts: {:?}",
            transport.attempts
        );
        let text = result.expect("successful decode retry should contain text");

        assert_eq!(text, "ok");
        assert_eq!(served_count, 2);
        assert_eq!(transport.attempts.len(), 2);
        assert_eq!(transport.attempts[0].status_code, Some(200));
        assert!(!transport.attempts[0].did_succeed);
        assert!(!transport.attempts[0].did_retry);
        assert!(transport.attempts[1].did_succeed);
        assert!(transport.attempts[1].did_retry);
        assert_decode_errors_are_safe(&transport);
    }

    #[test]
    fn live_cnki_stops_after_three_recorded_2xx_decode_failures() {
        let server = TestHttpServer::start(vec![
            malformed_gzip_response(200),
            malformed_gzip_response(200),
            malformed_gzip_response(200),
        ]);
        let request_url = format!("{}?api_key=persistent-secret", server.url());
        let mut transport = live_cnki_transport();

        let error = transport
            .get_text(&request_url, None, "decode_test")
            .expect_err("three decoding failures should fail loud");
        let served_count = server.finish();

        assert_eq!(served_count, 3);
        assert_eq!(transport.attempts.len(), 3);
        assert!(transport
            .attempts
            .iter()
            .all(|attempt| attempt.status_code == Some(200) && !attempt.did_succeed));
        assert_eq!(
            transport
                .attempts
                .iter()
                .map(|attempt| attempt.did_retry)
                .collect::<Vec<_>>(),
            [false, true, true]
        );
        assert!(error.to_string().contains("response body decoding failed"));
        assert!(!error.to_string().contains("persistent-secret"));
        assert_decode_errors_are_safe(&transport);
    }

    #[test]
    fn live_cnki_checks_non_success_status_before_decoding_body() {
        let server = TestHttpServer::start(vec![malformed_gzip_response(503), ok_response()]);
        let mut transport = live_cnki_transport();

        let result = transport.get_text(server.url(), None, "status_test");
        let served_count = server.finish();
        assert!(
            result.is_ok(),
            "HTTP retry should recover: {result:?}; attempts: {:?}",
            transport.attempts
        );
        let text = result.expect("checked successful HTTP retry should contain text");

        assert_eq!(text, "ok");
        assert_eq!(served_count, 2);
        assert_eq!(transport.attempts[0].status_code, Some(503));
        assert!(transport.attempts[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("HTTP 503")));
        assert!(transport
            .attempts
            .last()
            .is_some_and(|attempt| attempt.did_succeed));
    }

    #[test]
    fn issue_article_parser_returns_empty_for_missing_rows() {
        let articles = parse_issue_articles(
            "<dt class=\"tit\">Articles</dt>",
            &json!({"year": 2026, "number": "1"}),
        )
        .expect("empty section should parse");

        assert!(articles.is_empty());
    }

    #[test]
    fn fixture_client_records_successful_cnki_attempts() {
        let mut client = CnkiClient::new(FixtureCnkiTransport::new(cnki_fixture_data(None)));
        let journal = client
            .resolve_journal(&cnki_row())
            .expect("journal should resolve")
            .expect("journal should exist");
        let issues = client.year_issues(&journal).expect("issues should resolve");
        let articles = client
            .issue_articles(&journal, &issues[0])
            .expect("issue articles should resolve");
        let detail = client
            .article_detail(
                articles[0]["article_url"]
                    .as_str()
                    .expect("article url should exist"),
                articles[0]["platform_id"].as_str(),
            )
            .expect("article detail should resolve");

        assert_eq!(detail["platform_id"], "CNKI202601001");
        assert_eq!(
            client
                .attempts()
                .iter()
                .map(|attempt| attempt.endpoint.as_str())
                .collect::<Vec<_>>(),
            vec![
                "journal_detail",
                "year_issues",
                "issue_articles",
                "article_detail"
            ]
        );
        assert!(client.attempts().iter().all(|attempt| attempt.did_succeed));
    }

    #[test]
    fn fixture_client_records_missing_and_forced_failure_attempts() {
        let mut missing_client =
            CnkiClient::new(FixtureCnkiTransport::new(cnki_fixture_data(None)));
        let missing_error = missing_client
            .article_detail("https://example.test/missing", Some("missing"))
            .expect_err("missing article detail fixture should fail");

        assert!(matches!(missing_error, CnkiSourceError::MissingFixture(_)));
        assert_eq!(missing_client.attempts()[0].status_code, Some(500));
        assert!(!missing_client.attempts()[0].did_succeed);

        let mut forced_client = CnkiClient::new(FixtureCnkiTransport::new(cnki_fixture_data(
            Some("year_issues".to_string()),
        )));
        let journal = forced_client
            .resolve_journal(&cnki_row())
            .expect("journal should resolve")
            .expect("journal should exist");
        let forced_error = forced_client
            .year_issues(&journal)
            .expect_err("forced parser failure should fail");

        assert!(matches!(forced_error, CnkiSourceError::Parse(_)));
        assert_eq!(forced_client.attempts().len(), 2);
        assert!(!forced_client.attempts()[1].did_succeed);
    }

    #[test]
    fn malformed_cnki_pages_fail_loud() {
        let error = parse_journal_detail("<html><title>Missing Pykm</title></html>")
            .expect_err("journal detail missing pykm should fail");

        assert!(error.to_string().contains("missing pykm"));
    }

    fn cnki_fixture_data(fail_endpoint: Option<String>) -> CnkiFixtureData {
        CnkiFixtureData {
            journal_detail_html: r#"
                <html><head><title>CNKI Test Journal - 中国知网</title></head>
                <body>
                  <input id="pykm" value="TEST" />
                  <input id="pCode" value="CJFD" />
                  <input id="shareChName" value="CNKI Test Journal" />
                </body></html>
            "#
            .to_string(),
            year_issues_html:
                r#"<div id="YearIssueTree"><a id="yq202601" value="202601">2026 No.01</a></div>"#
                    .to_string(),
            issue_articles_html: BTreeMap::from([(
                "202601".to_string(),
                r#"
                <dt class="tit">Articles</dt>
                <dd class="row">
                  <a href="/kcms2/article/abstract?v=1&filename=CNKI202601001">CNKI article</a>
                  <b name="encrypt" id="CNKI202601001"></b>
                </dd>
                "#
                .to_string(),
            )]),
            article_detail_html: BTreeMap::from([(
                "CNKI202601001".to_string(),
                r#"
                <html><head><title>CNKI article</title></head>
                <body>
                  <input id="paramfilename" value="CNKI202601001" />
                  <input id="paramdbcode" value="CJFD" />
                  <input id="paramdbname" value="CJFDLAST2026" />
                  <p class="title-one">CNKI article</p>
                </body></html>
                "#
                .to_string(),
            )]),
            fail_endpoint,
        }
    }

    fn cnki_row() -> BTreeMap<String, String> {
        BTreeMap::from([("title".to_string(), "CNKI Test Journal".to_string())])
    }

    fn live_cnki_transport() -> LiveCnkiTransport {
        LiveCnkiTransport::new(LiveCnkiConfig { timeout_seconds: 5 })
            .expect("live CNKI test transport should build")
    }

    fn assert_decode_errors_are_safe(transport: &LiveCnkiTransport) {
        for attempt in transport
            .attempts
            .iter()
            .filter(|attempt| !attempt.did_succeed)
        {
            let error = attempt
                .error
                .as_deref()
                .expect("failure should have an error");
            assert!(error.contains("response body decoding failed"));
            assert!(!error.contains("secret"));
            assert!(!error.contains("not-gzip"));
        }
    }

    fn malformed_gzip_response(status_code: u16) -> String {
        let reason = if status_code == 200 {
            "OK"
        } else {
            "Service Unavailable"
        };
        format!(
            "HTTP/1.1 {status_code} {reason}\r\nContent-Encoding: gzip\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-gzip"
        )
    }

    fn ok_response() -> String {
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string()
    }

    #[test]
    fn test_http_server_finish_does_not_preempt_a_queued_response() {
        let server = TestHttpServer::start_with_accept_delay(
            vec![ok_response()],
            Duration::from_millis(100),
        );
        let address = server
            .url()
            .strip_prefix("http://")
            .and_then(|url| url.strip_suffix("/body"))
            .expect("test server URL should expose its socket address");
        let mut stream = TcpStream::connect(address).expect("queued test connection should open");
        stream
            .write_all(b"GET /body HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .expect("queued test request should write");

        assert_eq!(server.finish(), 1);
    }

    struct TestHttpServer {
        url: String,
        stop_sender: Sender<()>,
        completion_receiver: Receiver<usize>,
        handle: Option<JoinHandle<usize>>,
    }

    impl TestHttpServer {
        fn start(responses: Vec<String>) -> Self {
            Self::start_with_accept_delay(responses, Duration::ZERO)
        }

        fn start_with_accept_delay(responses: Vec<String>, accept_delay: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
            listener
                .set_nonblocking(true)
                .expect("test server should be nonblocking");
            let address = listener
                .local_addr()
                .expect("test server address should load");
            let (stop_sender, stop_receiver) = mpsc::channel();
            let (ready_sender, ready_receiver) = mpsc::channel();
            let (completion_sender, completion_receiver) = mpsc::channel();
            let handle = thread::spawn(move || {
                let planned_count = responses.len();
                ready_sender
                    .send(())
                    .expect("test server readiness should signal");
                thread::sleep(accept_delay);
                let mut responses = responses.into_iter();
                let mut served_count = 0;
                if planned_count == 0 {
                    let _ = completion_sender.send(0);
                    return 0;
                }
                loop {
                    if stop_receiver.try_recv().is_ok() {
                        return served_count;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_nonblocking(false)
                                .expect("test stream should be blocking");
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .expect("test stream timeout should set");
                            if !read_http_request(&mut stream) {
                                continue;
                            }
                            let Some(response) = responses.next() else {
                                return served_count;
                            };
                            if response.contains("Content-Encoding: gzip") {
                                let (headers, body) = response
                                    .split_once("\r\n\r\n")
                                    .expect("test response should contain headers");
                                stream
                                    .write_all(format!("{headers}\r\n\r\n").as_bytes())
                                    .expect("test response headers should write");
                                stream.flush().expect("test response headers should flush");
                                thread::sleep(Duration::from_millis(500));
                                let _ = stream.write_all(body.as_bytes());
                                let _ = stream.flush();
                            } else {
                                stream
                                    .write_all(response.as_bytes())
                                    .expect("test response should write");
                                stream.flush().expect("test response should flush");
                            }
                            served_count += 1;
                            drop(stream);
                            if served_count == planned_count {
                                let _ = completion_sender.send(served_count);
                                return served_count;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("test server accept failed: {error}"),
                    }
                }
            });
            ready_receiver
                .recv_timeout(TEST_SERVER_EVENT_TIMEOUT)
                .expect("test server should signal readiness");
            Self {
                url: format!("http://{address}/body"),
                stop_sender,
                completion_receiver,
                handle: Some(handle),
            }
        }

        fn url(&self) -> &str {
            &self.url
        }

        fn finish(mut self) -> usize {
            let served_count = match self
                .completion_receiver
                .recv_timeout(TEST_SERVER_EVENT_TIMEOUT)
            {
                Ok(served_count) => served_count,
                Err(error) => {
                    let _ = self.stop_sender.send(());
                    let joined_count = self
                        .handle
                        .take()
                        .expect("test server thread should exist")
                        .join()
                        .expect("failed test server thread should stop");
                    panic!(
                        "test server did not complete its planned responses: {error}; served {joined_count}"
                    );
                }
            };
            let joined_count = self
                .handle
                .take()
                .expect("test server thread should exist")
                .join()
                .expect("test server thread should finish");
            assert_eq!(joined_count, served_count);
            served_count
        }
    }

    impl Drop for TestHttpServer {
        fn drop(&mut self) {
            let _ = self.stop_sender.send(());
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> bool {
        const MAX_REQUEST_BYTES: usize = 16 * 1024;

        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        while request.len() < MAX_REQUEST_BYTES {
            match stream.read(&mut chunk) {
                Ok(0) => return false,
                Ok(read_count) => {
                    request.extend_from_slice(&chunk[..read_count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        return true;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return false;
                }
                Err(error) => panic!("test request headers failed: {error}"),
            }
        }
        panic!("test request headers exceeded {MAX_REQUEST_BYTES} bytes");
    }
}
