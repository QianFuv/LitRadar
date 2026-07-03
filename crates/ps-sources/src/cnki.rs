//! CNKI metadata source parsing and fixture transport.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::scholarly::{SourceAttempt, SourceError};

const BASE_URL: &str = "https://oversea.cnki.net";
const DEFAULT_PCODE: &str = "CJFD,CCJD";
const CNKI_CHINESE_LANGUAGE: &str = "CHS";

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

    /// Return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured source attempts.
    fn attempts(&self) -> &[SourceAttempt];
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
        let text = self.transport.text("journal_detail", None)?;
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
    pub fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, CnkiSourceError> {
        let _ = journal;
        let text = self.transport.text("year_issues", None)?;
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
    pub fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
    ) -> Result<Vec<Value>, CnkiSourceError> {
        let _ = journal;
        let year_issue = json_text(issue.get("year_issue"))
            .ok_or_else(|| CnkiSourceError::Parse("CNKI issue missing year_issue".to_string()))?;
        let text = self.transport.text("issue_articles", Some(&year_issue))?;
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
    pub fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, CnkiSourceError> {
        let key = platform_id.unwrap_or(article_url);
        let text = self.transport.text("article_detail", Some(key))?;
        parse_article_detail(&text, article_url)
    }

    /// Return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured source attempts.
    pub fn attempts(&self) -> &[SourceAttempt] {
        self.transport.attempts()
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
    use serde_json::json;

    use super::{
        checked_text, parse_article_detail, parse_issue_articles, parse_journal_detail,
        parse_year_issues,
    };

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
    fn verification_pages_fail_loud() {
        let error = checked_text("<html>captcha 安全验证</html>", "blocked")
            .expect_err("verification page should fail");

        assert!(error.to_string().contains("verification required"));
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
}
