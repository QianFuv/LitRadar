//! Scholarly source clients backed by deterministic fixture transports.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum DOI IDs accepted by one Semantic Scholar batch request.
pub const SEMANTIC_SCHOLAR_BATCH_SIZE: usize = 500;

const CROSSREF_SOURCE: &str = "crossref";
const OPENALEX_SOURCE: &str = "openalex";
const SEMANTIC_SCHOLAR_SOURCE: &str = "semantic_scholar";
const SEMANTIC_SCHOLAR_FIELDS: &str = "externalIds,url,isOpenAccess,openAccessPdf,abstract";

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
        /// Optional lower publication-date filter.
        from_pub_date: Option<String>,
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
        /// Optional lower publication-date filter.
        from_pub_date: Option<String>,
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
    /// OpenAlex source returned by ISSN lookup.
    #[serde(default)]
    pub openalex_source_by_issns: Option<Value>,
    /// OpenAlex source returned by title lookup.
    #[serde(default)]
    pub openalex_source_by_title: Option<Value>,
    /// OpenAlex works returned by source lookup.
    #[serde(default)]
    pub openalex_source_works: Vec<Value>,
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
}

/// Deterministic fixture transport for scholarly source tests.
#[derive(Debug, Clone)]
pub struct FixtureScholarlyTransport {
    data: ScholarlyFixtureData,
    attempts: Vec<SourceAttempt>,
    semantic_scholar_batches: Vec<Vec<String>>,
    openalex_doi_batches: Vec<Vec<String>>,
    source_lookup_issns: Vec<String>,
    source_lookup_titles: Vec<String>,
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
            semantic_scholar_batches: Vec::new(),
            openalex_doi_batches: Vec::new(),
            source_lookup_issns: Vec::new(),
            source_lookup_titles: Vec::new(),
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

impl ScholarlyTransport for FixtureScholarlyTransport {
    /// Execute one scholarly fixture request.
    fn request(&mut self, request: ScholarlyRequest) -> Result<Value, SourceError> {
        match &request.kind {
            ScholarlyRequestKind::CrossrefJournalWorks { .. } => {
                let status_code = self.data.crossref_status.unwrap_or(200);
                if status_code != 200 {
                    return Err(self.http_error(
                        &request,
                        status_code,
                        json!({"message": "fixture crossref failure"}),
                    ));
                }
                self.record_attempt(&request, Some(200), true, None);
                Ok(json!({"message": {"items": self.data.crossref_works}}))
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
                from_pub_date,
            } => {
                self.source_work_requests
                    .push((source_id.clone(), from_pub_date.clone()));
                self.record_attempt(&request, Some(200), true, None);
                Ok(json!({"results": self.data.openalex_source_works}))
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
}

/// Scholarly metadata client using a transport implementation.
#[derive(Debug, Clone)]
pub struct ScholarlyClient<T> {
    transport: T,
    has_semantic_scholar_key: bool,
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
        }
    }

    /// Fetch Crossref journal article works by ISSN.
    ///
    /// # Arguments
    ///
    /// * `issn` - ISSN lookup candidate.
    /// * `from_pub_date` - Optional lower publication-date filter.
    ///
    /// # Returns
    ///
    /// Crossref works.
    pub fn fetch_journal_works(
        &mut self,
        issn: &str,
        from_pub_date: Option<&str>,
    ) -> Result<Vec<Value>, SourceError> {
        let url = format!("https://api.crossref.org/journals/{issn}/works");
        let payload = self.transport.request(ScholarlyRequest {
            service: CROSSREF_SOURCE.to_string(),
            endpoint: "journal_works".to_string(),
            method: "GET".to_string(),
            url,
            kind: ScholarlyRequestKind::CrossrefJournalWorks {
                issn: issn.to_string(),
                from_pub_date: from_pub_date.map(str::to_string),
            },
        })?;
        Ok(payload
            .get("message")
            .and_then(|message| message.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
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

    /// Fetch OpenAlex works for a source identifier.
    ///
    /// # Arguments
    ///
    /// * `source_id` - OpenAlex source id or URL.
    /// * `from_pub_date` - Optional lower publication-date filter.
    ///
    /// # Returns
    ///
    /// OpenAlex work payloads.
    pub fn fetch_openalex_works_by_source(
        &mut self,
        source_id: &str,
        from_pub_date: Option<&str>,
    ) -> Result<Vec<Value>, SourceError> {
        let payload = self.transport.request(ScholarlyRequest {
            service: OPENALEX_SOURCE.to_string(),
            endpoint: "source_works".to_string(),
            method: "GET".to_string(),
            url: format!("https://api.openalex.org/works?filter=primary_location.source.id:{source_id}&api_key=SECRET"),
            kind: ScholarlyRequestKind::OpenAlexWorksBySource {
                source_id: source_id.to_string(),
                from_pub_date: from_pub_date.map(str::to_string),
            },
        })?;
        Ok(json_array(&payload, "results"))
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FixtureScholarlyTransport, ScholarlyClient, ScholarlyFixtureData, SourceError};

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
}
