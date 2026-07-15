//! Scholarly source clients backed by deterministic fixture transports.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum DOI IDs accepted by one Semantic Scholar batch request.
pub const SEMANTIC_SCHOLAR_BATCH_SIZE: usize = 500;

const CROSSREF_BASE_URL: &str = "https://api.crossref.org/v1";
const CROSSREF_SOURCE: &str = "crossref";
const OPENALEX_BASE_URL: &str = "https://api.openalex.org";
const OPENALEX_SOURCE: &str = "openalex";
const SEMANTIC_SCHOLAR_BASE_URL: &str = "https://api.semanticscholar.org/graph/v1";
const SEMANTIC_SCHOLAR_SOURCE: &str = "semantic_scholar";
const SEMANTIC_SCHOLAR_FIELDS: &str = "externalIds,url,isOpenAccess,openAccessPdf,abstract";
const OPENALEX_SOURCE_FIELDS: &str = "id,display_name,issn_l,issn,works_count";
const OPENALEX_WORK_FIELDS: &str = "id,doi,title,display_name,publication_year,publication_date,language,cited_by_count,is_retracted,primary_location,locations,open_access,best_oa_location,authorships,ids,biblio,abstract_inverted_index,topics,primary_topic,funders,awards";
const DEFAULT_USER_AGENT: &str = "LitRadar/0.1 (mailto:litradar@example.invalid)";
const CROSSREF_ROWS: usize = 225;
const OPENALEX_SOURCE_WORK_ROWS: usize = 200;
const DEFAULT_MAX_RETRIES: usize = 2;
const RETRY_STATUS_CODES: [u16; 5] = [429, 500, 502, 503, 504];

fn crossref_journal_filter(from_sync_date: Option<&str>) -> String {
    let mut filters = vec!["type:journal-article".to_string()];
    if let Some(value) = from_sync_date.filter(|value| !value.trim().is_empty()) {
        filters.push(format!("from-update-date:{value}"));
    }
    filters.join(",")
}

fn openalex_source_work_filter(source_id: &str, from_sync_date: Option<&str>) -> String {
    let mut filters = vec![
        format!("primary_location.source.id:{source_id}"),
        "type:article".to_string(),
    ];
    if let Some(value) = from_sync_date.filter(|value| !value.trim().is_empty()) {
        filters.push(format!("from_created_date:{value}"));
    }
    filters.join(",")
}

fn is_openalex_created_date_plan_error(error: &SourceError) -> bool {
    let SourceError::HttpStatus {
        service,
        endpoint,
        status_code: 429,
        body,
    } = error
    else {
        return false;
    };
    service == OPENALEX_SOURCE
        && endpoint == "source_works"
        && body.get("error").and_then(Value::as_str) == Some("Plan upgrade required")
        && body
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("from_created_date"))
}

fn openalex_source_works_request(
    source_id: &str,
    from_sync_date: Option<&str>,
    cursor: Option<&str>,
) -> ScholarlyRequest {
    ScholarlyRequest {
        service: OPENALEX_SOURCE.to_string(),
        endpoint: "source_works".to_string(),
        method: "GET".to_string(),
        url: format!("https://api.openalex.org/works?filter=primary_location.source.id:{source_id}&api_key=SECRET"),
        kind: ScholarlyRequestKind::OpenAlexWorksBySource {
            source_id: source_id.to_string(),
            from_sync_date: from_sync_date.map(str::to_string),
            cursor: cursor.map(str::to_string),
        },
    }
}

/// One source transport attempt captured for index statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceAttempt {
    /// Upstream service identifier.
    pub service: String,
    /// Logical endpoint identifier.
    pub endpoint: String,
    /// HTTP method.
    pub method: String,
    /// Request URL or fixture URL equivalent.
    pub url: String,
    /// HTTP status code when available.
    pub status_code: Option<u16>,
    /// Whether the attempt succeeded.
    pub did_succeed: bool,
    /// Whether the attempt is part of retry accounting.
    pub did_retry: bool,
    /// Attempt error sample.
    pub error: Option<String>,
}

/// One bounded page of scholarly work payloads.
#[derive(Debug, Clone, PartialEq)]
pub struct ScholarlyWorksPage {
    /// Work payloads in upstream order.
    pub items: Vec<Value>,
    /// Cursor for the next page when one is available.
    pub next_cursor: Option<String>,
}

/// Request shape sent through a scholarly transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScholarlyRequest {
    /// Upstream service identifier.
    pub service: String,
    /// Logical endpoint identifier.
    pub endpoint: String,
    /// HTTP method.
    pub method: String,
    /// Request URL or fixture URL equivalent.
    pub url: String,
    /// Parsed request kind used by fixture transports.
    pub kind: ScholarlyRequestKind,
}

/// Typed scholarly request kind for fixture transports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScholarlyRequestKind {
    /// Fetch Crossref journal works by ISSN.
    CrossrefJournalWorks {
        /// ISSN lookup candidate.
        issn: String,
        /// Optional lower synchronization-date filter.
        from_sync_date: Option<String>,
        /// Cursor returned by the previous page.
        cursor: Option<String>,
    },
    /// Fetch an OpenAlex source by ISSN.
    OpenAlexSourceByIssn {
        /// ISSN lookup candidate.
        issn: String,
    },
    /// Fetch an OpenAlex source by title search.
    OpenAlexSourceByTitle {
        /// Journal title search value.
        title: String,
    },
    /// Fetch OpenAlex works for a source.
    OpenAlexWorksBySource {
        /// OpenAlex source id or URL.
        source_id: String,
        /// Optional lower synchronization-date filter.
        from_sync_date: Option<String>,
        /// Cursor returned by the previous page.
        cursor: Option<String>,
    },
    /// Fetch OpenAlex works by DOI filters.
    OpenAlexWorksByDoi {
        /// DOI batch.
        dois: Vec<String>,
    },
    /// Fetch Semantic Scholar papers by DOI batch.
    SemanticScholarBatch {
        /// DOI batch.
        dois: Vec<String>,
    },
}

/// Fixture payload used by the scholarly transport.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScholarlyFixtureData {
    /// Optional Crossref journal works status code.
    #[serde(default)]
    pub crossref_status: Option<u16>,
    /// Crossref works returned by journal ISSN lookup.
    #[serde(default)]
    pub crossref_works: Vec<Value>,
    /// Optional Crossref pages used by bounded pagination tests.
    #[serde(default)]
    pub crossref_work_pages: Vec<Vec<Value>>,
    /// OpenAlex source returned by ISSN lookup.
    #[serde(default)]
    pub openalex_source_by_issns: Option<Value>,
    /// OpenAlex source returned by title lookup.
    #[serde(default)]
    pub openalex_source_by_title: Option<Value>,
    /// OpenAlex works returned by source lookup.
    #[serde(default)]
    pub openalex_source_works: Vec<Value>,
    /// Optional OpenAlex source-work pages used by bounded pagination tests.
    #[serde(default)]
    pub openalex_source_work_pages: Vec<Vec<Value>>,
    /// Whether dated OpenAlex source-work requests require a paid plan.
    #[serde(default)]
    pub openalex_source_works_plan_restricted: bool,
    /// Optional OpenAlex source-work status code.
    #[serde(default)]
    pub openalex_source_works_status: Option<u16>,
    /// OpenAlex works returned by DOI enrichment.
    #[serde(default)]
    pub openalex_by_doi: BTreeMap<String, Value>,
    /// Optional Semantic Scholar status code.
    #[serde(default)]
    pub semantic_scholar_status: Option<u16>,
    /// Optional Semantic Scholar error text.
    #[serde(default)]
    pub semantic_scholar_error: Option<String>,
    /// Semantic Scholar payloads returned by DOI enrichment.
    #[serde(default)]
    pub semantic_scholar_by_doi: BTreeMap<String, Value>,
}

/// Errors returned by source clients and fixture transports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// HTTP-like status failure.
    HttpStatus {
        /// Upstream service identifier.
        service: String,
        /// Logical endpoint identifier.
        endpoint: String,
        /// HTTP status code.
        status_code: u16,
        /// Error response body.
        body: Value,
    },
    /// Fixture payload shape is invalid.
    InvalidFixture(String),
    /// Required client configuration is missing.
    Configuration(String),
    /// HTTP request failed before a usable source response was available.
    Request {
        /// Upstream service identifier.
        service: String,
        /// Logical endpoint identifier.
        endpoint: String,
        /// Request or transport failure message.
        message: String,
    },
}

impl fmt::Display for SourceError {
    /// Format the source error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpStatus {
                service,
                endpoint,
                status_code,
                ..
            } => write!(
                formatter,
                "{service} {endpoint} failed with HTTP {status_code}"
            ),
            Self::InvalidFixture(message) => formatter.write_str(message),
            Self::Configuration(message) => formatter.write_str(message),
            Self::Request {
                service,
                endpoint,
                message,
            } => write!(formatter, "{service} {endpoint} request failed: {message}"),
        }
    }
}

impl Error for SourceError {}

/// Scholarly source transport abstraction.
pub trait ScholarlyTransport {
    /// Execute one scholarly request.
    ///
    /// # Arguments
    ///
    /// * `request` - Typed scholarly request.
    ///
    /// # Returns
    ///
    /// JSON response payload.
    fn request(&mut self, request: ScholarlyRequest) -> Result<Value, SourceError>;

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
}

/// Deterministic fixture transport for scholarly source tests.
#[derive(Debug, Clone)]
pub struct FixtureScholarlyTransport {
    data: ScholarlyFixtureData,
    attempts: Vec<SourceAttempt>,
    crossref_page_index: usize,
    semantic_scholar_batches: Vec<Vec<String>>,
    openalex_doi_batches: Vec<Vec<String>>,
    source_lookup_issns: Vec<String>,
    source_lookup_titles: Vec<String>,
    journal_work_requests: Vec<(String, Option<String>)>,
    source_work_requests: Vec<(String, Option<String>)>,
}

impl FixtureScholarlyTransport {
    /// Build a fixture transport from response data.
    ///
    /// # Arguments
    ///
    /// * `data` - Scholarly fixture response payloads.
    ///
    /// # Returns
    ///
    /// Fixture transport.
    pub fn new(data: ScholarlyFixtureData) -> Self {
        Self {
            data,
            attempts: Vec::new(),
            crossref_page_index: 0,
            semantic_scholar_batches: Vec::new(),
            openalex_doi_batches: Vec::new(),
            source_lookup_issns: Vec::new(),
            source_lookup_titles: Vec::new(),
            journal_work_requests: Vec::new(),
            source_work_requests: Vec::new(),
        }
    }

    /// Return captured Semantic Scholar DOI batches.
    ///
    /// # Returns
    ///
    /// Captured DOI batches.
    pub fn semantic_scholar_batches(&self) -> &[Vec<String>] {
        &self.semantic_scholar_batches
    }

    /// Return captured OpenAlex DOI batches.
    ///
    /// # Returns
    ///
    /// Captured DOI batches.
    pub fn openalex_doi_batches(&self) -> &[Vec<String>] {
        &self.openalex_doi_batches
    }

    /// Return captured OpenAlex source ISSN lookups.
    ///
    /// # Returns
    ///
    /// Captured ISSN candidates.
    pub fn source_lookup_issns(&self) -> &[String] {
        &self.source_lookup_issns
    }

    /// Return captured OpenAlex source title lookups.
    ///
    /// # Returns
    ///
    /// Captured title candidates.
    pub fn source_lookup_titles(&self) -> &[String] {
        &self.source_lookup_titles
    }

    /// Return captured Crossref journal work requests.
    ///
    /// # Returns
    ///
    /// Captured ISSN and synchronization-date pairs.
    pub fn journal_work_requests(&self) -> &[(String, Option<String>)] {
        &self.journal_work_requests
    }

    /// Return captured OpenAlex source work requests.
    ///
    /// # Returns
    ///
    /// Captured source work requests.
    pub fn source_work_requests(&self) -> &[(String, Option<String>)] {
        &self.source_work_requests
    }

    fn record_attempt(
        &mut self,
        request: &ScholarlyRequest,
        status_code: Option<u16>,
        did_succeed: bool,
        error: Option<String>,
    ) {
        self.attempts.push(SourceAttempt {
            service: request.service.clone(),
            endpoint: request.endpoint.clone(),
            method: request.method.clone(),
            url: request.url.clone(),
            status_code,
            did_succeed,
            did_retry: false,
            error,
        });
    }

    fn http_error(
        &mut self,
        request: &ScholarlyRequest,
        status_code: u16,
        body: Value,
    ) -> SourceError {
        self.record_attempt(
            request,
            Some(status_code),
            false,
            Some(format!("HTTP {status_code}: {}", body)),
        );
        SourceError::HttpStatus {
            service: request.service.clone(),
            endpoint: request.endpoint.clone(),
            status_code,
            body,
        }
    }
}

/// Live Scholarly source transport configuration.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct LiveScholarlyConfig {
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// OpenAlex API key candidates.
    pub openalex_api_keys: Vec<String>,
    /// Semantic Scholar API key candidates.
    pub semantic_scholar_api_keys: Vec<String>,
    /// Crossref mailto candidates.
    pub crossref_mailtos: Vec<String>,
    /// Current journal worker id for process-aware Semantic Scholar throttling.
    pub semantic_scholar_worker_id: usize,
    /// Journal worker process count for process-aware Semantic Scholar throttling.
    pub semantic_scholar_process_count: usize,
    /// Base Semantic Scholar global interval in milliseconds.
    pub semantic_scholar_base_interval_ms: u64,
}

impl fmt::Debug for LiveScholarlyConfig {
    /// Format source configuration without exposing key or mailto values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveScholarlyConfig")
            .field("timeout_seconds", &self.timeout_seconds)
            .field("openalex_api_key_count", &self.openalex_api_keys.len())
            .field(
                "semantic_scholar_api_key_count",
                &self.semantic_scholar_api_keys.len(),
            )
            .field("crossref_mailto_count", &self.crossref_mailtos.len())
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

impl LiveScholarlyConfig {
    /// Build live Scholarly configuration from explicit value pools.
    ///
    /// # Arguments
    ///
    /// * `timeout_seconds` - HTTP request timeout in seconds.
    /// * `openalex_api_key_pool` - OpenAlex API key pool text.
    /// * `semantic_scholar_api_key_pool` - Semantic Scholar API key pool text.
    /// * `crossref_mailto_pool` - Crossref mailto pool text.
    ///
    /// # Returns
    ///
    /// Live Scholarly configuration.
    pub fn from_value_pools(
        timeout_seconds: u64,
        openalex_api_key_pool: &str,
        semantic_scholar_api_key_pool: &str,
        crossref_mailto_pool: &str,
    ) -> Self {
        Self {
            timeout_seconds,
            openalex_api_keys: value_pool_from_text(openalex_api_key_pool),
            semantic_scholar_api_keys: value_pool_from_text(semantic_scholar_api_key_pool),
            crossref_mailtos: value_pool_from_text(crossref_mailto_pool),
            semantic_scholar_worker_id: 0,
            semantic_scholar_process_count: 1,
            semantic_scholar_base_interval_ms: 1_000,
        }
    }

    /// Return a config with Semantic Scholar worker throttle context.
    ///
    /// # Arguments
    ///
    /// * `worker_id` - Current journal worker id.
    /// * `process_count` - Total journal worker process count.
    ///
    /// # Returns
    ///
    /// Updated live Scholarly configuration.
    pub fn with_worker_context(mut self, worker_id: usize, process_count: usize) -> Self {
        self.semantic_scholar_worker_id = worker_id;
        self.semantic_scholar_process_count = process_count.max(1);
        self
    }

    /// Return whether Semantic Scholar enrichment can be authenticated.
    ///
    /// # Returns
    ///
    /// True when at least one Semantic Scholar key is configured.
    pub fn has_semantic_scholar_key(&self) -> bool {
        !self.semantic_scholar_api_keys.is_empty()
    }
}

fn semantic_scholar_worker_offset(config: &LiveScholarlyConfig) -> Duration {
    let process_count = config.semantic_scholar_process_count.max(1);
    let worker_id = config
        .semantic_scholar_worker_id
        .min(process_count.saturating_sub(1));
    Duration::from_millis(
        config
            .semantic_scholar_base_interval_ms
            .saturating_mul(worker_id as u64),
    )
}

fn semantic_scholar_worker_interval(config: &LiveScholarlyConfig) -> Duration {
    Duration::from_millis(
        config
            .semantic_scholar_base_interval_ms
            .saturating_mul(config.semantic_scholar_process_count.max(1) as u64),
    )
}

/// Blocking HTTP transport for live Scholarly sources.
#[derive(Debug, Clone)]
pub struct LiveScholarlyTransport {
    client: Client,
    config: LiveScholarlyConfig,
    attempts: Vec<SourceAttempt>,
    next_semantic_scholar_at: Option<Instant>,
}

struct JsonRequest<'a> {
    service: &'a str,
    endpoint: &'a str,
    method: &'a str,
    url: &'a str,
    query: &'a [(String, String)],
    body: Option<&'a Value>,
    header: Option<(&'a str, String)>,
}

struct LiveAttempt<'a> {
    service: &'a str,
    endpoint: &'a str,
    method: &'a str,
    url: &'a str,
    status_code: Option<u16>,
    did_succeed: bool,
    did_retry: bool,
    error: Option<String>,
}

impl LiveScholarlyTransport {
    /// Build a live Scholarly transport.
    ///
    /// # Arguments
    ///
    /// * `config` - Live source configuration.
    ///
    /// # Returns
    ///
    /// Live transport or a request configuration error.
    pub fn new(config: LiveScholarlyConfig) -> Result<Self, SourceError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.max(1)))
            .user_agent(DEFAULT_USER_AGENT)
            .build()
            .map_err(|error| SourceError::Request {
                service: "http".to_string(),
                endpoint: "client".to_string(),
                message: error.to_string(),
            })?;
        Ok(Self {
            client,
            next_semantic_scholar_at: Some(
                Instant::now() + semantic_scholar_worker_offset(&config),
            ),
            config,
            attempts: Vec::new(),
        })
    }

    fn crossref_journal_works(
        &mut self,
        issn: &str,
        from_sync_date: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Value, SourceError> {
        let mut query = vec![
            ("rows".to_string(), CROSSREF_ROWS.to_string()),
            ("cursor".to_string(), cursor.unwrap_or("*").to_string()),
            (
                "filter".to_string(),
                crossref_journal_filter(from_sync_date),
            ),
            ("sort".to_string(), "published".to_string()),
            ("order".to_string(), "asc".to_string()),
        ];
        if let Some(mailto) = self.config.crossref_mailtos.first() {
            query.push(("mailto".to_string(), mailto.clone()));
        }
        let mut payload = self.get_json(
            CROSSREF_SOURCE,
            "journal_works",
            &format!("{CROSSREF_BASE_URL}/journals/{issn}/works"),
            &query,
        )?;
        let item_count = payload
            .get("message")
            .and_then(|message| message.get("items"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if item_count < CROSSREF_ROWS {
            if let Some(message) = payload.get_mut("message").and_then(Value::as_object_mut) {
                message.remove("next-cursor");
            }
        }
        Ok(payload)
    }

    fn openalex_source_by_issn(&mut self, issn: &str) -> Result<Value, SourceError> {
        let mut query = vec![
            ("filter".to_string(), format!("issn:{issn}")),
            ("per-page".to_string(), "5".to_string()),
            ("select".to_string(), OPENALEX_SOURCE_FIELDS.to_string()),
        ];
        self.append_openalex_config(&mut query);
        self.get_json(
            OPENALEX_SOURCE,
            "sources",
            &format!("{OPENALEX_BASE_URL}/sources"),
            &query,
        )
    }

    fn openalex_source_by_title(&mut self, title: &str) -> Result<Value, SourceError> {
        let mut query = vec![
            ("search".to_string(), title.to_string()),
            ("per-page".to_string(), "5".to_string()),
            ("select".to_string(), OPENALEX_SOURCE_FIELDS.to_string()),
        ];
        self.append_openalex_config(&mut query);
        self.get_json(
            OPENALEX_SOURCE,
            "source_search",
            &format!("{OPENALEX_BASE_URL}/sources"),
            &query,
        )
    }

    fn openalex_works_by_source(
        &mut self,
        source_id: &str,
        from_sync_date: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Value, SourceError> {
        let Some(source_key) = openalex_short_source_id(source_id) else {
            return Ok(json!({ "results": [] }));
        };
        let mut query = vec![
            (
                "filter".to_string(),
                openalex_source_work_filter(&source_key, from_sync_date),
            ),
            (
                "per-page".to_string(),
                OPENALEX_SOURCE_WORK_ROWS.to_string(),
            ),
            ("cursor".to_string(), cursor.unwrap_or("*").to_string()),
            ("sort".to_string(), "publication_date:asc".to_string()),
            ("select".to_string(), OPENALEX_WORK_FIELDS.to_string()),
        ];
        self.append_openalex_config(&mut query);
        let mut payload = self.get_json(
            OPENALEX_SOURCE,
            "source_works",
            &format!("{OPENALEX_BASE_URL}/works"),
            &query,
        )?;
        let item_count = payload
            .get("results")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if item_count < OPENALEX_SOURCE_WORK_ROWS {
            if let Some(meta) = payload.get_mut("meta").and_then(Value::as_object_mut) {
                meta.insert("next_cursor".to_string(), Value::Null);
            }
        }
        Ok(payload)
    }

    fn openalex_works_by_doi(&mut self, dois: &[String]) -> Result<Value, SourceError> {
        let filter_value = dois
            .iter()
            .map(|doi| format!("https://doi.org/{doi}"))
            .collect::<Vec<_>>()
            .join("|");
        let mut query = vec![
            ("filter".to_string(), format!("doi:{filter_value}")),
            ("per-page".to_string(), dois.len().max(1).to_string()),
            ("select".to_string(), OPENALEX_WORK_FIELDS.to_string()),
        ];
        self.append_openalex_config(&mut query);
        self.get_json(
            OPENALEX_SOURCE,
            "works",
            &format!("{OPENALEX_BASE_URL}/works"),
            &query,
        )
    }

    fn semantic_scholar_batch(&mut self, dois: &[String]) -> Result<Value, SourceError> {
        let Some(api_key) = self.config.semantic_scholar_api_keys.first().cloned() else {
            return Err(SourceError::Configuration(
                "Semantic Scholar API key is required for DOI enrichment.".to_string(),
            ));
        };
        self.wait_for_semantic_scholar_slot();
        let query = vec![("fields".to_string(), SEMANTIC_SCHOLAR_FIELDS.to_string())];
        let body = json!({
            "ids": dois.iter().map(|doi| format!("DOI:{doi}")).collect::<Vec<_>>()
        });
        self.post_json(
            SEMANTIC_SCHOLAR_SOURCE,
            "paper_batch",
            &format!("{SEMANTIC_SCHOLAR_BASE_URL}/paper/batch"),
            &query,
            &body,
            Some(("x-api-key", api_key)),
        )
    }

    fn wait_for_semantic_scholar_slot(&mut self) {
        let interval = semantic_scholar_worker_interval(&self.config);
        if interval.is_zero() {
            return;
        }
        if let Some(next_at) = self.next_semantic_scholar_at {
            let now = Instant::now();
            if next_at > now {
                thread::sleep(next_at - now);
            }
        }
        self.next_semantic_scholar_at = Some(Instant::now() + interval);
    }

    fn append_openalex_config(&self, query: &mut Vec<(String, String)>) {
        if let Some(api_key) = self.config.openalex_api_keys.first() {
            query.push(("api_key".to_string(), api_key.clone()));
        }
        if let Some(mailto) = self.config.crossref_mailtos.first() {
            query.push(("mailto".to_string(), mailto.clone()));
        }
    }

    fn get_json(
        &mut self,
        service: &str,
        endpoint: &str,
        url: &str,
        query: &[(String, String)],
    ) -> Result<Value, SourceError> {
        self.request_json(JsonRequest {
            service,
            endpoint,
            method: "GET",
            url,
            query,
            body: None,
            header: None,
        })
    }

    fn post_json(
        &mut self,
        service: &str,
        endpoint: &str,
        url: &str,
        query: &[(String, String)],
        body: &Value,
        header: Option<(&str, String)>,
    ) -> Result<Value, SourceError> {
        self.request_json(JsonRequest {
            service,
            endpoint,
            method: "POST",
            url,
            query,
            body: Some(body),
            header,
        })
    }

    fn request_json(&mut self, live_request: JsonRequest<'_>) -> Result<Value, SourceError> {
        for attempt in 0..=DEFAULT_MAX_RETRIES {
            let mut builder = match live_request.method {
                "POST" => self.client.post(live_request.url),
                _ => self.client.get(live_request.url),
            }
            .query(live_request.query);
            if let Some(body) = live_request.body {
                builder = builder.json(body);
            }
            if let Some((name, value)) = &live_request.header {
                builder = builder.header(*name, value);
            }
            let request = builder.build().map_err(|error| SourceError::Request {
                service: live_request.service.to_string(),
                endpoint: live_request.endpoint.to_string(),
                message: error.to_string(),
            })?;
            let request_url = redact_url(request.url().as_ref());
            match self.client.execute(request) {
                Ok(response) => {
                    let status_code = response.status().as_u16();
                    let text = response.text().map_err(|error| SourceError::Request {
                        service: live_request.service.to_string(),
                        endpoint: live_request.endpoint.to_string(),
                        message: error.to_string(),
                    })?;
                    let payload = serde_json::from_str::<Value>(&text)
                        .unwrap_or_else(|_| json!({ "error": text }));
                    if !(200..300).contains(&status_code) {
                        self.record_attempt(LiveAttempt {
                            service: live_request.service,
                            endpoint: live_request.endpoint,
                            method: live_request.method,
                            url: &request_url,
                            status_code: Some(status_code),
                            did_succeed: false,
                            did_retry: attempt > 0,
                            error: payload
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                        if RETRY_STATUS_CODES.contains(&status_code)
                            && attempt < DEFAULT_MAX_RETRIES
                        {
                            thread::sleep(Duration::from_secs((attempt + 1) as u64));
                            continue;
                        }
                        return Err(SourceError::HttpStatus {
                            service: live_request.service.to_string(),
                            endpoint: live_request.endpoint.to_string(),
                            status_code,
                            body: payload,
                        });
                    }
                    self.record_attempt(LiveAttempt {
                        service: live_request.service,
                        endpoint: live_request.endpoint,
                        method: live_request.method,
                        url: &request_url,
                        status_code: Some(status_code),
                        did_succeed: true,
                        did_retry: attempt > 0,
                        error: None,
                    });
                    return Ok(payload);
                }
                Err(error) => {
                    self.record_attempt(LiveAttempt {
                        service: live_request.service,
                        endpoint: live_request.endpoint,
                        method: live_request.method,
                        url: &redact_url(live_request.url),
                        status_code: None,
                        did_succeed: false,
                        did_retry: attempt > 0,
                        error: Some(error.to_string()),
                    });
                    if attempt < DEFAULT_MAX_RETRIES {
                        thread::sleep(Duration::from_secs((attempt + 1) as u64));
                        continue;
                    }
                    return Err(SourceError::Request {
                        service: live_request.service.to_string(),
                        endpoint: live_request.endpoint.to_string(),
                        message: error.to_string(),
                    });
                }
            }
        }
        Err(SourceError::Request {
            service: live_request.service.to_string(),
            endpoint: live_request.endpoint.to_string(),
            message: "request retry loop exhausted".to_string(),
        })
    }

    fn record_attempt(&mut self, attempt: LiveAttempt<'_>) {
        self.attempts.push(SourceAttempt {
            service: attempt.service.to_string(),
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

impl ScholarlyTransport for FixtureScholarlyTransport {
    /// Execute one scholarly fixture request.
    fn request(&mut self, request: ScholarlyRequest) -> Result<Value, SourceError> {
        match &request.kind {
            ScholarlyRequestKind::CrossrefJournalWorks {
                issn,
                from_sync_date,
                cursor,
            } => {
                self.journal_work_requests
                    .push((issn.clone(), from_sync_date.clone()));
                let status_code = self.data.crossref_status.unwrap_or(200);
                if status_code != 200 {
                    return Err(self.http_error(
                        &request,
                        status_code,
                        json!({"message": "fixture crossref failure"}),
                    ));
                }
                self.record_attempt(&request, Some(200), true, None);
                if cursor.is_none() {
                    self.crossref_page_index = 0;
                }
                let (items, next_cursor) = fixture_page(
                    &self.data.crossref_work_pages,
                    &self.data.crossref_works,
                    self.crossref_page_index,
                );
                self.crossref_page_index += 1;
                let next_cursor = next_cursor.map(|_| "stateful-crossref-cursor".to_string());
                Ok(json!({
                    "message": {
                        "items": items,
                        "next-cursor": next_cursor,
                    }
                }))
            }
            ScholarlyRequestKind::OpenAlexSourceByIssn { issn } => {
                self.source_lookup_issns.push(issn.clone());
                self.record_attempt(&request, Some(200), true, None);
                let results = self
                    .data
                    .openalex_source_by_issns
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>();
                Ok(json!({"results": results}))
            }
            ScholarlyRequestKind::OpenAlexSourceByTitle { title } => {
                self.source_lookup_titles.push(title.clone());
                self.record_attempt(&request, Some(200), true, None);
                let results = self
                    .data
                    .openalex_source_by_title
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>();
                Ok(json!({"results": results}))
            }
            ScholarlyRequestKind::OpenAlexWorksBySource {
                source_id,
                from_sync_date,
                cursor,
            } => {
                self.source_work_requests
                    .push((source_id.clone(), from_sync_date.clone()));
                if from_sync_date.is_some() && self.data.openalex_source_works_plan_restricted {
                    return Err(self.http_error(
                        &request,
                        429,
                        json!({
                            "error": "Plan upgrade required",
                            "message": "The from_created_date filter requires a Premium plan."
                        }),
                    ));
                }
                let status_code = self.data.openalex_source_works_status.unwrap_or(200);
                if status_code != 200 {
                    return Err(self.http_error(
                        &request,
                        status_code,
                        json!({"error": "fixture OpenAlex source works failure"}),
                    ));
                }
                self.record_attempt(&request, Some(200), true, None);
                let page_index = fixture_page_index(cursor.as_deref());
                let (items, next_cursor) = fixture_page(
                    &self.data.openalex_source_work_pages,
                    &self.data.openalex_source_works,
                    page_index,
                );
                Ok(json!({
                    "results": items,
                    "meta": {"next_cursor": next_cursor},
                }))
            }
            ScholarlyRequestKind::OpenAlexWorksByDoi { dois } => {
                self.openalex_doi_batches.push(dois.clone());
                self.record_attempt(&request, Some(200), true, None);
                let results = dois
                    .iter()
                    .filter_map(|doi| self.data.openalex_by_doi.get(doi).cloned())
                    .collect::<Vec<_>>();
                Ok(json!({"results": results}))
            }
            ScholarlyRequestKind::SemanticScholarBatch { dois } => {
                self.semantic_scholar_batches.push(dois.clone());
                let status_code = self.data.semantic_scholar_status.unwrap_or(200);
                if status_code != 200 {
                    let body = json!({
                        "error": self
                            .data
                            .semantic_scholar_error
                            .as_deref()
                            .unwrap_or("fixture semantic scholar failure")
                    });
                    return Err(self.http_error(&request, status_code, body));
                }
                self.record_attempt(&request, Some(200), true, None);
                let results = dois
                    .iter()
                    .filter_map(|doi| self.data.semantic_scholar_by_doi.get(doi).cloned())
                    .collect::<Vec<_>>();
                Ok(Value::Array(results))
            }
        }
    }

    /// Return captured source attempts.
    fn attempts(&self) -> &[SourceAttempt] {
        &self.attempts
    }

    /// Drain captured source attempts.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        std::mem::take(&mut self.attempts)
    }
}

impl ScholarlyTransport for LiveScholarlyTransport {
    /// Execute one live Scholarly source request.
    fn request(&mut self, request: ScholarlyRequest) -> Result<Value, SourceError> {
        match request.kind {
            ScholarlyRequestKind::CrossrefJournalWorks {
                issn,
                from_sync_date,
                cursor,
            } => self.crossref_journal_works(&issn, from_sync_date.as_deref(), cursor.as_deref()),
            ScholarlyRequestKind::OpenAlexSourceByIssn { issn } => {
                self.openalex_source_by_issn(&issn)
            }
            ScholarlyRequestKind::OpenAlexSourceByTitle { title } => {
                self.openalex_source_by_title(&title)
            }
            ScholarlyRequestKind::OpenAlexWorksBySource {
                source_id,
                from_sync_date,
                cursor,
            } => self.openalex_works_by_source(
                &source_id,
                from_sync_date.as_deref(),
                cursor.as_deref(),
            ),
            ScholarlyRequestKind::OpenAlexWorksByDoi { dois } => self.openalex_works_by_doi(&dois),
            ScholarlyRequestKind::SemanticScholarBatch { dois } => {
                self.semantic_scholar_batch(&dois)
            }
        }
    }

    /// Return captured live source attempts.
    fn attempts(&self) -> &[SourceAttempt] {
        &self.attempts
    }

    /// Drain captured live source attempts.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        std::mem::take(&mut self.attempts)
    }
}

fn fixture_page_index(cursor: Option<&str>) -> usize {
    cursor
        .and_then(|value| value.strip_prefix("fixture-page-"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

fn fixture_page(
    pages: &[Vec<Value>],
    fallback: &[Value],
    page_index: usize,
) -> (Vec<Value>, Option<String>) {
    if pages.is_empty() {
        return if page_index == 0 {
            (fallback.to_vec(), None)
        } else {
            (Vec::new(), None)
        };
    }
    let items = pages.get(page_index).cloned().unwrap_or_default();
    let next_cursor =
        (page_index + 1 < pages.len()).then(|| format!("fixture-page-{}", page_index + 1));
    (items, next_cursor)
}

/// Scholarly metadata client using a transport implementation.
#[derive(Debug, Clone)]
pub struct ScholarlyClient<T> {
    transport: T,
    has_semantic_scholar_key: bool,
    is_openalex_created_date_filter_unavailable: bool,
}

impl<T> ScholarlyClient<T>
where
    T: ScholarlyTransport,
{
    /// Build a scholarly client from a transport.
    ///
    /// # Arguments
    ///
    /// * `transport` - Source transport.
    /// * `has_semantic_scholar_key` - Whether Semantic Scholar enrichment is configured.
    ///
    /// # Returns
    ///
    /// Scholarly client.
    pub fn new(transport: T, has_semantic_scholar_key: bool) -> Self {
        Self {
            transport,
            has_semantic_scholar_key,
            is_openalex_created_date_filter_unavailable: false,
        }
    }

    /// Fetch one Crossref journal-work page by ISSN.
    ///
    /// # Arguments
    ///
    /// * `issn` - ISSN lookup candidate.
    /// * `from_sync_date` - Optional lower synchronization-date filter.
    /// * `cursor` - Cursor returned by the previous page.
    ///
    /// # Returns
    ///
    /// Bounded Crossref works page.
    pub fn fetch_journal_works_page(
        &mut self,
        issn: &str,
        from_sync_date: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<ScholarlyWorksPage, SourceError> {
        let url = format!("https://api.crossref.org/journals/{issn}/works");
        let payload = self.transport.request(ScholarlyRequest {
            service: CROSSREF_SOURCE.to_string(),
            endpoint: "journal_works".to_string(),
            method: "GET".to_string(),
            url,
            kind: ScholarlyRequestKind::CrossrefJournalWorks {
                issn: issn.to_string(),
                from_sync_date: from_sync_date.map(str::to_string),
                cursor: cursor.map(str::to_string),
            },
        })?;
        let message = payload.get("message");
        let items = message
            .and_then(|message| message.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let next_cursor = message
            .and_then(|message| message.get("next-cursor"))
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(ScholarlyWorksPage { items, next_cursor })
    }

    /// Fetch an OpenAlex source matching the provided ISSNs.
    ///
    /// # Arguments
    ///
    /// * `issns` - ISSN candidates.
    ///
    /// # Returns
    ///
    /// Matching OpenAlex source payload.
    pub fn fetch_openalex_source_by_issns(
        &mut self,
        issns: &[String],
    ) -> Result<Option<Value>, SourceError> {
        for issn in issns {
            let payload = self.transport.request(ScholarlyRequest {
                service: OPENALEX_SOURCE.to_string(),
                endpoint: "sources".to_string(),
                method: "GET".to_string(),
                url: format!("https://api.openalex.org/sources?filter=issn:{issn}&api_key=SECRET"),
                kind: ScholarlyRequestKind::OpenAlexSourceByIssn { issn: issn.clone() },
            })?;
            for item in json_array(&payload, "results") {
                if openalex_source_matches_issn(&item, issn) {
                    return Ok(Some(item));
                }
            }
        }
        Ok(None)
    }

    /// Fetch an OpenAlex source matching a title exactly.
    ///
    /// # Arguments
    ///
    /// * `title` - Journal title.
    ///
    /// # Returns
    ///
    /// Matching OpenAlex source payload.
    pub fn fetch_openalex_source_by_title(
        &mut self,
        title: &str,
    ) -> Result<Option<Value>, SourceError> {
        let normalized_title = normalize_source_title(title);
        if normalized_title.is_empty() {
            return Ok(None);
        }
        let payload = self.transport.request(ScholarlyRequest {
            service: OPENALEX_SOURCE.to_string(),
            endpoint: "source_search".to_string(),
            method: "GET".to_string(),
            url: format!("https://api.openalex.org/sources?search={title}&api_key=SECRET"),
            kind: ScholarlyRequestKind::OpenAlexSourceByTitle {
                title: title.to_string(),
            },
        })?;
        for item in json_array(&payload, "results") {
            if openalex_source_matches_title(&item, &normalized_title) {
                return Ok(Some(item));
            }
        }
        Ok(None)
    }

    /// Fetch one OpenAlex work page for a source identifier.
    ///
    /// # Arguments
    ///
    /// * `source_id` - OpenAlex source id or URL.
    /// * `from_sync_date` - Optional lower synchronization-date filter.
    /// * `cursor` - Cursor returned by the previous page.
    ///
    /// # Returns
    ///
    /// Bounded OpenAlex works page.
    pub fn fetch_openalex_works_by_source_page(
        &mut self,
        source_id: &str,
        from_sync_date: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<ScholarlyWorksPage, SourceError> {
        let effective_sync_date = if self.is_openalex_created_date_filter_unavailable {
            None
        } else {
            from_sync_date
        };
        let request = openalex_source_works_request(source_id, effective_sync_date, cursor);
        let payload = match self.transport.request(request) {
            Err(error)
                if effective_sync_date.is_some() && is_openalex_created_date_plan_error(&error) =>
            {
                self.is_openalex_created_date_filter_unavailable = true;
                self.transport
                    .request(openalex_source_works_request(source_id, None, cursor))?
            }
            result => result?,
        };
        let items = json_array(&payload, "results");
        let next_cursor = payload
            .get("meta")
            .and_then(|meta| meta.get("next_cursor"))
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(ScholarlyWorksPage { items, next_cursor })
    }

    /// Fetch OpenAlex enrichment by DOI.
    ///
    /// # Arguments
    ///
    /// * `dois` - DOI values.
    /// * `batch_size` - Maximum DOI count per request.
    ///
    /// # Returns
    ///
    /// OpenAlex works keyed by normalized DOI.
    pub fn fetch_openalex_by_dois(
        &mut self,
        dois: &[String],
        batch_size: usize,
    ) -> Result<BTreeMap<String, Value>, SourceError> {
        let normalized = unique_normalized_dois(dois);
        let mut results = BTreeMap::new();
        for batch in normalized.chunks(batch_size.max(1)) {
            let batch = batch.to_vec();
            let payload = self.transport.request(ScholarlyRequest {
                service: OPENALEX_SOURCE.to_string(),
                endpoint: "works".to_string(),
                method: "GET".to_string(),
                url: "https://api.openalex.org/works?filter=doi:https://doi.org/example&api_key=SECRET".to_string(),
                kind: ScholarlyRequestKind::OpenAlexWorksByDoi {
                    dois: batch.clone(),
                },
            })?;
            for item in json_array(&payload, "results") {
                if let Some(doi) = normalize_doi(item.get("doi")) {
                    results.insert(doi, item);
                }
            }
        }
        Ok(results)
    }

    /// Fetch Semantic Scholar enrichment by DOI.
    ///
    /// # Arguments
    ///
    /// * `dois` - DOI values.
    /// * `batch_size` - Requested DOI count per request.
    ///
    /// # Returns
    ///
    /// Semantic Scholar works keyed by normalized DOI.
    pub fn fetch_semantic_scholar_by_dois(
        &mut self,
        dois: &[String],
        batch_size: usize,
    ) -> Result<BTreeMap<String, Value>, SourceError> {
        let normalized = unique_normalized_dois(dois);
        if normalized.is_empty() {
            return Ok(BTreeMap::new());
        }
        if !self.has_semantic_scholar_key {
            return Err(SourceError::Configuration(
                "Semantic Scholar API key is required for DOI enrichment.".into(),
            ));
        }

        let mut results = BTreeMap::new();
        let effective_batch_size = batch_size.clamp(1, SEMANTIC_SCHOLAR_BATCH_SIZE);
        for batch in normalized.chunks(effective_batch_size) {
            let batch = batch.to_vec();
            let payload = match self.transport.request(ScholarlyRequest {
                service: SEMANTIC_SCHOLAR_SOURCE.to_string(),
                endpoint: "paper_batch".to_string(),
                method: "POST".to_string(),
                url: format!(
                    "https://api.semanticscholar.org/graph/v1/paper/batch?fields={SEMANTIC_SCHOLAR_FIELDS}&x-api-key=SECRET"
                ),
                kind: ScholarlyRequestKind::SemanticScholarBatch {
                    dois: batch.clone(),
                },
            }) {
                Ok(payload) => payload,
                Err(error) if is_semantic_scholar_no_valid_ids_error(&error) => continue,
                Err(error) => return Err(error),
            };
            if let Value::Array(items) = payload {
                for item in items {
                    if let Some(doi) = semantic_scholar_doi(&item) {
                        results.insert(doi, item);
                    }
                }
            }
        }
        Ok(results)
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

    /// Consume the client and return its transport.
    ///
    /// # Returns
    ///
    /// Owned transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

/// Normalize DOI-like values to lowercase bare DOI text.
///
/// # Arguments
///
/// * `value` - DOI-like JSON value.
///
/// # Returns
///
/// Normalized DOI.
pub fn normalize_doi(value: Option<&Value>) -> Option<String> {
    let text = json_text(value?)?;
    let mut lowered = text.to_lowercase();
    for prefix in ["https://doi.org/", "http://doi.org/", "doi:"] {
        if lowered.starts_with(prefix) {
            lowered = lowered[prefix.len()..].to_string();
            break;
        }
    }
    let stripped = lowered.trim().to_string();
    (!stripped.is_empty()).then_some(stripped)
}

fn json_array(payload: &Value, key: &str) -> Vec<Value> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn unique_normalized_dois(dois: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    dois.iter()
        .filter_map(|doi| normalize_doi(Some(&Value::String(doi.clone()))))
        .filter(|doi| seen.insert(doi.clone()))
        .collect()
}

fn semantic_scholar_doi(item: &Value) -> Option<String> {
    item.get("externalIds")
        .and_then(|external_ids| external_ids.get("DOI"))
        .and_then(|doi| normalize_doi(Some(doi)))
}

fn is_semantic_scholar_no_valid_ids_error(error: &SourceError) -> bool {
    let SourceError::HttpStatus {
        service,
        endpoint,
        status_code,
        body,
    } = error
    else {
        return false;
    };
    if service != SEMANTIC_SCHOLAR_SOURCE || endpoint != "paper_batch" || *status_code != 400 {
        return false;
    }
    body.get("error")
        .and_then(Value::as_str)
        .map(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("no valid paper ids given")
        })
        .unwrap_or(false)
}

fn openalex_source_matches_issn(item: &Value, issn: &str) -> bool {
    let Some(target) = normalize_issn(issn) else {
        return false;
    };
    let mut candidates = Vec::new();
    if let Some(value) = item.get("issn_l").and_then(Value::as_str) {
        candidates.push(normalize_issn(value));
    }
    if let Some(values) = item.get("issn").and_then(Value::as_array) {
        candidates.extend(values.iter().filter_map(Value::as_str).map(normalize_issn));
    }
    candidates
        .into_iter()
        .flatten()
        .any(|value| value == target)
}

fn openalex_source_matches_title(item: &Value, normalized_title: &str) -> bool {
    item.get("display_name")
        .and_then(Value::as_str)
        .map(normalize_source_title)
        .map(|candidate| candidate == normalized_title)
        .unwrap_or(false)
}

fn normalize_issn(value: &str) -> Option<String> {
    let text = value.trim().replace('-', "").to_uppercase();
    if text.len() == 8
        && text
            .chars()
            .take(7)
            .all(|character| character.is_ascii_digit())
        && text
            .chars()
            .nth(7)
            .map(|character| character.is_ascii_digit() || character == 'X')
            .unwrap_or(false)
    {
        Some(text)
    } else {
        None
    }
}

fn normalize_source_title(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn json_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => non_empty(text),
        other => non_empty(&other.to_string()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let stripped = value.trim();
    (!stripped.is_empty()).then(|| stripped.to_string())
}

fn value_pool_from_text(value: &str) -> Vec<String> {
    let mut pool = Vec::new();
    for part in value.split([',', ';', '\n']) {
        let item = part.trim();
        if !item.is_empty() && !pool.iter().any(|value| value == item) {
            pool.push(item.to_string());
        }
    }
    pool
}

fn openalex_short_source_id(source_id: &str) -> Option<String> {
    let value = source_id.trim().trim_end_matches('/');
    if value.is_empty() {
        return None;
    }
    value
        .rsplit('/')
        .next()
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn redact_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let redacted = query
        .split('&')
        .map(|part| {
            let key = part.split('=').next().unwrap_or_default();
            if key == "api_key" || key == "x-api-key" {
                format!("{key}=SECRET")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{redacted}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use serde_json::json;

    use super::{
        crossref_journal_filter, normalize_doi, normalize_issn, normalize_source_title,
        openalex_short_source_id, openalex_source_work_filter, redact_url,
        semantic_scholar_worker_interval, semantic_scholar_worker_offset, value_pool_from_text,
        FixtureScholarlyTransport, LiveScholarlyConfig, ScholarlyClient, ScholarlyFixtureData,
        ScholarlyTransport, SourceError, CROSSREF_ROWS,
    };

    #[test]
    fn value_pool_splits_runtime_config() {
        assert_eq!(
            value_pool_from_text(" one, two;one\nthree "),
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
    }

    #[test]
    fn live_config_debug_redacts_credentials() {
        let config = LiveScholarlyConfig {
            timeout_seconds: 30,
            openalex_api_keys: vec!["openalex-secret".to_string()],
            semantic_scholar_api_keys: vec!["semantic-secret".to_string()],
            crossref_mailtos: vec!["private@example.com".to_string()],
            semantic_scholar_worker_id: 0,
            semantic_scholar_process_count: 1,
            semantic_scholar_base_interval_ms: 1,
        };

        let debug = format!("{config:?}");

        assert!(debug.contains("openalex_api_key_count: 1"));
        assert!(!debug.contains("openalex-secret"));
        assert!(!debug.contains("semantic-secret"));
        assert!(!debug.contains("private@example.com"));
    }

    #[test]
    fn semantic_scholar_throttle_uses_worker_offset_and_process_interval() {
        let config = LiveScholarlyConfig {
            timeout_seconds: 1,
            openalex_api_keys: Vec::new(),
            semantic_scholar_api_keys: vec!["s2".to_string()],
            crossref_mailtos: Vec::new(),
            semantic_scholar_worker_id: 2,
            semantic_scholar_process_count: 4,
            semantic_scholar_base_interval_ms: 25,
        };

        assert_eq!(
            semantic_scholar_worker_offset(&config),
            Duration::from_millis(50)
        );
        assert_eq!(
            semantic_scholar_worker_interval(&config),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn semantic_scholar_batches_are_capped_at_five_hundred_ids() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData::default());
        let mut client = ScholarlyClient::new(transport, true);
        let dois = (0..501)
            .map(|index| format!("10.1/{index}"))
            .collect::<Vec<_>>();

        client
            .fetch_semantic_scholar_by_dois(&dois, 999)
            .expect("fixture S2 request should succeed");
        let transport = client.into_transport();

        assert_eq!(
            transport
                .semantic_scholar_batches()
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            vec![500, 1]
        );
    }

    #[test]
    fn semantic_scholar_no_valid_id_error_returns_empty() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            semantic_scholar_status: Some(400),
            semantic_scholar_error: Some("No valid paper ids given".into()),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let result = client
            .fetch_semantic_scholar_by_dois(&["10.1/new".into()], 500)
            .expect("no-valid-id sentinel should be swallowed");

        assert!(result.is_empty());
        assert_eq!(client.attempts()[0].status_code, Some(400));
        assert!(!client.attempts()[0].did_succeed);
    }

    #[test]
    fn semantic_scholar_other_http_errors_fail_loud() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            semantic_scholar_status: Some(400),
            semantic_scholar_error: Some("bad request".into()),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let error = client
            .fetch_semantic_scholar_by_dois(&["10.1/a".into()], 500)
            .expect_err("ordinary S2 400 should fail loud");

        assert!(matches!(
            error,
            SourceError::HttpStatus {
                status_code: 400,
                ..
            }
        ));
    }

    #[test]
    fn openalex_source_matching_uses_fixture_transport() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_by_issns: Some(json!({
                "id": "https://openalex.org/S1",
                "display_name": "Cognition",
                "issn_l": "0010-0277",
                "issn": ["0010-0277", "1873-7838"]
            })),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let source = client
            .fetch_openalex_source_by_issns(&["1873-7838".into()])
            .expect("fixture source lookup should succeed");

        assert_eq!(
            source.and_then(|value| value["id"].as_str().map(str::to_string)),
            Some("https://openalex.org/S1".into())
        );
    }

    #[test]
    fn openalex_matching_helpers_reject_mismatches_and_empty_titles() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_by_issns: Some(json!({
                "id": "https://openalex.org/S1",
                "display_name": "Wrong Journal",
                "issn_l": "0000-0000",
                "issn": ["1111-1111"]
            })),
            openalex_source_by_title: Some(json!({
                "id": "https://openalex.org/S2",
                "display_name": "Another Journal"
            })),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        assert!(client
            .fetch_openalex_source_by_issns(&["1234-567X".to_string()])
            .expect("mismatched ISSN source lookup should resolve")
            .is_none());
        assert!(client
            .fetch_openalex_source_by_title("Journal of Testing")
            .expect("mismatched title source lookup should resolve")
            .is_none());

        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData::default());
        let mut empty_title_client = ScholarlyClient::new(transport, true);
        assert!(empty_title_client
            .fetch_openalex_source_by_title("   ")
            .expect("empty title lookup should resolve")
            .is_none());
        assert!(empty_title_client.attempts().is_empty());
    }

    #[test]
    fn openalex_doi_lookup_deduplicates_and_batches() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_by_doi: BTreeMap::from([
                (
                    "10.1/a".to_string(),
                    json!({"id": "https://openalex.org/W1", "doi": "https://doi.org/10.1/a"}),
                ),
                (
                    "10.1/b".to_string(),
                    json!({"id": "https://openalex.org/W2", "doi": "doi:10.1/b"}),
                ),
            ]),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let results = client
            .fetch_openalex_by_dois(
                &[
                    "HTTPS://DOI.ORG/10.1/A".to_string(),
                    "doi:10.1/a".to_string(),
                    "10.1/b".to_string(),
                ],
                1,
            )
            .expect("OpenAlex DOI fixture should succeed");
        let transport = client.into_transport();

        assert_eq!(
            results.keys().cloned().collect::<Vec<_>>(),
            vec!["10.1/a", "10.1/b"]
        );
        assert_eq!(
            transport.openalex_doi_batches(),
            &[vec!["10.1/a".to_string()], vec!["10.1/b".to_string()]]
        );
    }

    #[test]
    fn openalex_title_and_source_work_requests_are_captured() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_by_title: Some(json!({
                "id": "https://openalex.org/S42",
                "display_name": "Journal of Testing"
            })),
            openalex_source_works: vec![json!({"id": "https://openalex.org/W42"})],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let source = client
            .fetch_openalex_source_by_title("Journal of Testing")
            .expect("title source lookup should succeed")
            .expect("title source should match");
        let works = client
            .fetch_openalex_works_by_source_page(
                source["id"].as_str().expect("source id should exist"),
                Some("2026-01-01"),
                None,
            )
            .expect("source works should load");
        let transport = client.into_transport();

        assert_eq!(works.items[0]["id"], "https://openalex.org/W42");
        assert_eq!(
            transport.source_lookup_titles(),
            &["Journal of Testing".to_string()]
        );
        assert_eq!(
            transport.source_work_requests(),
            &[(
                "https://openalex.org/S42".to_string(),
                Some("2026-01-01".to_string())
            )]
        );
    }

    #[test]
    fn openalex_plan_error_falls_back_to_full_source_pages() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_work_pages: vec![
                vec![json!({"id": "https://openalex.org/W42"})],
                vec![json!({"id": "https://openalex.org/W43"})],
            ],
            openalex_source_works_plan_restricted: true,
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let first_page = client
            .fetch_openalex_works_by_source_page("S42", Some("2026-01-01"), None)
            .expect("paid filter error should fall back to a full source page");
        let second_page = client
            .fetch_openalex_works_by_source_page(
                "S42",
                Some("2026-01-01"),
                first_page.next_cursor.as_deref(),
            )
            .expect("later pages should retain the full source fallback");
        let transport = client.into_transport();

        assert_eq!(first_page.items[0]["id"], "https://openalex.org/W42");
        assert_eq!(second_page.items[0]["id"], "https://openalex.org/W43");
        assert_eq!(
            transport.source_work_requests(),
            &[
                ("S42".to_string(), Some("2026-01-01".to_string())),
                ("S42".to_string(), None),
                ("S42".to_string(), None),
            ]
        );
        assert_eq!(
            transport
                .attempts()
                .iter()
                .map(|attempt| (attempt.status_code, attempt.did_succeed))
                .collect::<Vec<_>>(),
            vec![(Some(429), false), (Some(200), true), (Some(200), true)]
        );
        assert!(transport
            .attempts()
            .iter()
            .all(|attempt| !attempt.url.contains("2026-01-01")));
    }

    #[test]
    fn openalex_unrelated_rate_limit_remains_fatal() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_works_status: Some(429),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let error = client
            .fetch_openalex_works_by_source_page("S42", Some("2026-01-01"), None)
            .expect_err("ordinary rate limiting should remain fatal");
        let transport = client.into_transport();

        assert!(matches!(
            error,
            SourceError::HttpStatus {
                status_code: 429,
                ..
            }
        ));
        assert_eq!(
            transport.source_work_requests(),
            &[("S42".to_string(), Some("2026-01-01".to_string()))]
        );
        assert_eq!(transport.attempts().len(), 1);
    }

    #[test]
    fn openalex_undated_source_page_uses_one_request() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_works: vec![json!({"id": "https://openalex.org/W42"})],
            openalex_source_works_plan_restricted: true,
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        client
            .fetch_openalex_works_by_source_page("S42", None, None)
            .expect("full source page should not need a fallback");
        let transport = client.into_transport();

        assert_eq!(
            transport.source_work_requests(),
            &[("S42".to_string(), None)]
        );
        assert_eq!(transport.attempts().len(), 1);
        assert!(transport.attempts()[0].did_succeed);
    }

    #[test]
    fn crossref_status_errors_record_attempts() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_status: Some(503),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let error = client
            .fetch_journal_works_page("1234-5678", Some("2026-01-01"), None)
            .expect_err("Crossref fixture failure should fail loud");

        assert!(matches!(
            error,
            SourceError::HttpStatus {
                status_code: 503,
                ..
            }
        ));
        assert_eq!(client.attempts()[0].endpoint, "journal_works");
        assert!(!client.attempts()[0].did_succeed);
    }

    #[test]
    fn semantic_scholar_requires_key_before_transport_request() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData::default());
        let mut client = ScholarlyClient::new(transport, false);

        let error = client
            .fetch_semantic_scholar_by_dois(&["10.1/a".to_string()], 10)
            .expect_err("missing Semantic Scholar key should fail before transport");

        assert!(matches!(error, SourceError::Configuration(_)));
        assert!(client.attempts().is_empty());
    }

    #[test]
    fn crossref_success_and_url_helpers_cover_edge_inputs() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_works: vec![json!({"DOI": "10.1/success"})],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let works = client
            .fetch_journal_works_page("1234-5678", None, None)
            .expect("Crossref fixture success should resolve");

        assert_eq!(works.items[0]["DOI"], "10.1/success");
        assert_eq!(client.attempts()[0].status_code, Some(200));
        assert!(client.attempts()[0].did_succeed);
        assert_eq!(normalize_issn("1234-567X"), Some("1234567X".to_string()));
        assert_eq!(normalize_issn("bad"), None);
        assert_eq!(
            normalize_source_title(" Journal   OF Testing "),
            "journal of testing"
        );
        assert_eq!(
            openalex_short_source_id("https://openalex.org/S123/"),
            Some("S123".to_string())
        );
        assert_eq!(openalex_short_source_id("   "), None);
        assert_eq!(
            redact_url("https://api.test/path?api_key=abc&x-api-key=def&mail=me"),
            "https://api.test/path?api_key=SECRET&x-api-key=SECRET&mail=me"
        );
        assert_eq!(redact_url("https://api.test/path"), "https://api.test/path");
    }

    #[test]
    fn crossref_pages_follow_cursors_and_attempts_can_be_drained() {
        let first_page = (0..CROSSREF_ROWS)
            .map(|index| json!({"DOI": format!("10.1/{index}")}))
            .collect::<Vec<_>>();
        let second_page = (CROSSREF_ROWS..(CROSSREF_ROWS * 2))
            .map(|index| json!({"DOI": format!("10.1/{index}")}))
            .collect::<Vec<_>>();
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_work_pages: vec![first_page, second_page, vec![json!({"DOI": "10.1/final"})]],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);

        let first = client
            .fetch_journal_works_page("1234-5678", None, None)
            .expect("first page should load");
        assert_eq!(first.items.len(), CROSSREF_ROWS);
        assert_eq!(
            first.next_cursor.as_deref(),
            Some("stateful-crossref-cursor")
        );
        assert_eq!(client.drain_attempts().len(), 1);
        assert!(client.attempts().is_empty());

        let second = client
            .fetch_journal_works_page("1234-5678", None, first.next_cursor.as_deref())
            .expect("second page should load");
        assert_eq!(second.items.len(), CROSSREF_ROWS);
        assert_eq!(second.next_cursor, first.next_cursor);

        let third = client
            .fetch_journal_works_page("1234-5678", None, second.next_cursor.as_deref())
            .expect("third page should load");
        assert_eq!(third.items, vec![json!({"DOI": "10.1/final"})]);
        assert_eq!(third.next_cursor, None);
    }

    #[test]
    fn crossref_page_rows_stay_within_live_memory_budget() {
        assert_eq!(CROSSREF_ROWS, 225);
    }

    #[test]
    fn provider_filters_map_one_synchronization_date() {
        assert_eq!(
            crossref_journal_filter(Some("2026-01-02")),
            "type:journal-article,from-update-date:2026-01-02"
        );
        assert_eq!(
            openalex_source_work_filter("S42", Some("2026-01-02")),
            "primary_location.source.id:S42,type:article,from_created_date:2026-01-02"
        );
        assert_eq!(crossref_journal_filter(None), "type:journal-article");
        assert_eq!(
            openalex_source_work_filter("S42", None),
            "primary_location.source.id:S42,type:article"
        );
    }

    #[test]
    fn doi_normalization_handles_prefixes_and_empty_values() {
        assert_eq!(
            normalize_doi(Some(&json!("https://doi.org/10.1/ABC"))),
            Some("10.1/abc".to_string())
        );
        assert_eq!(
            normalize_doi(Some(&json!("doi:10.2/XYZ"))),
            Some("10.2/xyz".to_string())
        );
        assert_eq!(normalize_doi(Some(&json!(" "))), None);
        assert_eq!(normalize_doi(None), None);
    }
}
