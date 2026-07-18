//! Scholarly source clients backed by deterministic fixture transports.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::{blocking::Client, header::HeaderMap, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum DOI IDs accepted by one Semantic Scholar batch request.
pub const SEMANTIC_SCHOLAR_BATCH_SIZE: usize = 500;

/// Maximum OpenAlex DOI requests allowed in flight per process.
pub const OPENALEX_MAX_WORKERS_PER_PROCESS: usize = 6;

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
const OPENALEX_DOI_FILTER_MAX_VALUES: usize = 100;
const OPENALEX_DOI_REQUEST_URL_BUDGET: usize = 1_900;
const OPENALEX_DEFAULT_REMAINING_CREDITS: u64 = 100_000;
const OPENALEX_KEY_START_INTERVAL: Duration = Duration::from_millis(34);
const OPENALEX_SOURCE_WORK_ROWS: usize = 200;
const DEFAULT_MAX_RETRIES: usize = 2;
const RETRY_STATUS_CODES: [u16; 5] = [429, 500, 502, 503, 504];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct OpenAlexRateHeaders {
    remaining: Option<u64>,
    reset_after: Option<Duration>,
    retry_after: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAlexHealthOutcome {
    Success,
    AuthenticationFailure,
    RateLimited,
    TransientFailure,
    TerminalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenAlexSlotReservation {
    slot_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAlexScheduleDecision {
    Reserved(OpenAlexSlotReservation),
    WaitUntil(Duration),
    Unavailable,
}

#[derive(Debug, Clone)]
struct OpenAlexKeyState {
    remaining: Option<u64>,
    reset_at: Option<Duration>,
    cooldown_until: Option<Duration>,
    in_flight: usize,
    next_start_at: Duration,
    is_disabled: bool,
    selection_credit: i128,
}

impl Default for OpenAlexKeyState {
    fn default() -> Self {
        Self {
            remaining: None,
            reset_at: None,
            cooldown_until: None,
            in_flight: 0,
            next_start_at: Duration::ZERO,
            is_disabled: false,
            selection_credit: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct OpenAlexSchedulerState {
    slots: Vec<OpenAlexKeyState>,
    next_tie_slot: usize,
}

impl OpenAlexSchedulerState {
    fn new(key_count: usize, process_id: usize) -> Self {
        Self {
            slots: vec![OpenAlexKeyState::default(); key_count],
            next_tie_slot: if key_count == 0 {
                0
            } else {
                process_id % key_count
            },
        }
    }

    fn reserve_slot(
        &mut self,
        now: Duration,
        excluded_slots: &[usize],
    ) -> OpenAlexScheduleDecision {
        self.refresh(now);
        let ready_slots = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| (self.ready_at(slot)? <= now).then_some(index))
            .collect::<Vec<_>>();
        let preferred_slots = ready_slots
            .iter()
            .copied()
            .filter(|index| !excluded_slots.contains(index))
            .collect::<Vec<_>>();
        let candidates = if preferred_slots.is_empty() {
            &ready_slots
        } else {
            &preferred_slots
        };
        if let Some(slot_index) = self.best_slot(candidates) {
            let slot = &mut self.slots[slot_index];
            let reservation = OpenAlexSlotReservation { slot_index };
            slot.in_flight = slot.in_flight.saturating_add(1);
            slot.next_start_at = now.saturating_add(OPENALEX_KEY_START_INTERVAL);
            self.next_tie_slot = (slot_index + 1) % self.slots.len();
            return OpenAlexScheduleDecision::Reserved(reservation);
        }
        self.slots
            .iter()
            .filter_map(|slot| self.ready_at(slot))
            .min()
            .map_or(OpenAlexScheduleDecision::Unavailable, |ready_at| {
                OpenAlexScheduleDecision::WaitUntil(ready_at)
            })
    }

    fn finish_slot(
        &mut self,
        reservation: &OpenAlexSlotReservation,
        now: Duration,
        headers: OpenAlexRateHeaders,
        outcome: OpenAlexHealthOutcome,
        retry_delay: Duration,
    ) {
        self.refresh(now);
        let Some(slot) = self.slots.get_mut(reservation.slot_index) else {
            return;
        };
        slot.in_flight = slot.in_flight.saturating_sub(1);
        let has_quota_headers = headers.remaining.is_some() || headers.reset_after.is_some();
        if has_quota_headers {
            if let Some(remaining) = headers.remaining {
                slot.remaining = Some(
                    slot.remaining
                        .map_or(remaining, |current| current.min(remaining)),
                );
            }
            if let Some(reset_after) = headers.reset_after {
                let proposed_reset = now.saturating_add(reset_after);
                slot.reset_at = Some(
                    slot.reset_at
                        .map_or(proposed_reset, |current| current.max(proposed_reset)),
                );
            }
        }
        match outcome {
            OpenAlexHealthOutcome::Success | OpenAlexHealthOutcome::TerminalFailure => {}
            OpenAlexHealthOutcome::AuthenticationFailure => {
                slot.is_disabled = true;
                slot.cooldown_until = None;
            }
            OpenAlexHealthOutcome::RateLimited => {
                let cooldown = headers
                    .reset_after
                    .into_iter()
                    .chain(headers.retry_after)
                    .chain((!retry_delay.is_zero()).then_some(retry_delay))
                    .max()
                    .unwrap_or(Duration::from_secs(1));
                slot.cooldown_until = Some(now.saturating_add(cooldown));
            }
            OpenAlexHealthOutcome::TransientFailure => {
                if !retry_delay.is_zero() {
                    slot.cooldown_until = Some(now.saturating_add(retry_delay));
                }
            }
        }
    }

    fn refresh(&mut self, now: Duration) {
        for slot in &mut self.slots {
            if slot.reset_at.is_some_and(|reset_at| reset_at <= now) {
                slot.remaining = None;
                slot.reset_at = None;
            }
            if slot
                .cooldown_until
                .is_some_and(|cooldown_until| cooldown_until <= now)
            {
                slot.cooldown_until = None;
            }
        }
    }

    fn ready_at(&self, slot: &OpenAlexKeyState) -> Option<Duration> {
        if slot.is_disabled {
            return None;
        }
        let mut ready_at = slot.next_start_at;
        if let Some(cooldown_until) = slot.cooldown_until {
            ready_at = ready_at.max(cooldown_until);
        }
        if slot.remaining == Some(0) {
            ready_at = ready_at.max(slot.reset_at?);
        }
        Some(ready_at)
    }

    fn best_slot(&mut self, candidates: &[usize]) -> Option<usize> {
        let mut total_weight = 0_i128;
        for index in candidates {
            let slot = &mut self.slots[*index];
            let weight = i128::from(slot.remaining.unwrap_or(OPENALEX_DEFAULT_REMAINING_CREDITS))
                / i128::try_from(slot.in_flight.saturating_add(1)).unwrap_or(i128::MAX);
            let weight = weight.max(1);
            slot.selection_credit = slot.selection_credit.saturating_add(weight);
            total_weight = total_weight.saturating_add(weight);
        }
        let selected = candidates.iter().copied().max_by(|left, right| {
            self.slots[*left]
                .selection_credit
                .cmp(&self.slots[*right].selection_credit)
                .then_with(|| {
                    let slot_count = self.slots.len();
                    let left_distance = (*left + slot_count - self.next_tie_slot) % slot_count;
                    let right_distance = (*right + slot_count - self.next_tie_slot) % slot_count;
                    right_distance.cmp(&left_distance)
                })
        });
        if let Some(index) = selected {
            self.slots[index].selection_credit = self.slots[index]
                .selection_credit
                .saturating_sub(total_weight);
        }
        selected
    }
}

struct OpenAlexReservation {
    slot: OpenAlexSlotReservation,
    api_key: String,
}

impl fmt::Debug for OpenAlexReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAlexReservation")
            .field("slot_index", &self.slot.slot_index)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

struct OpenAlexScheduler {
    api_keys: Vec<String>,
    state: Mutex<OpenAlexSchedulerState>,
    changed: Condvar,
    started_at: Instant,
}

impl fmt::Debug for OpenAlexScheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAlexScheduler")
            .field("key_count", &self.api_keys.len())
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

impl OpenAlexScheduler {
    fn new(api_keys: Vec<String>, process_id: usize) -> Self {
        Self {
            state: Mutex::new(OpenAlexSchedulerState::new(api_keys.len(), process_id)),
            api_keys,
            changed: Condvar::new(),
            started_at: Instant::now(),
        }
    }

    fn key_count(&self) -> usize {
        self.api_keys.len()
    }

    fn reserve(&self, excluded_slots: &[usize]) -> Result<OpenAlexReservation, SourceError> {
        let mut state = self.state.lock().map_err(|_| {
            SourceError::Configuration("OpenAlex key scheduler is unavailable.".to_string())
        })?;
        loop {
            let now = self.started_at.elapsed();
            match state.reserve_slot(now, excluded_slots) {
                OpenAlexScheduleDecision::Reserved(slot) => {
                    return Ok(OpenAlexReservation {
                        api_key: self.api_keys[slot.slot_index].clone(),
                        slot,
                    });
                }
                OpenAlexScheduleDecision::WaitUntil(ready_at) => {
                    let wait = ready_at.saturating_sub(now);
                    if wait.is_zero() {
                        continue;
                    }
                    if wait >= Duration::from_secs(1) {
                        tracing::info!(
                            event = "source.openalex.quota_wait",
                            component = "source",
                            provider = OPENALEX_SOURCE,
                            reason = "quota_or_cooldown",
                            wait_ms = wait.as_millis().min(u128::from(u64::MAX)) as u64,
                            key_slot_count = self.api_keys.len(),
                        );
                    }
                    let (next_state, _) = self.changed.wait_timeout(state, wait).map_err(|_| {
                        SourceError::Configuration(
                            "OpenAlex key scheduler is unavailable.".to_string(),
                        )
                    })?;
                    state = next_state;
                }
                OpenAlexScheduleDecision::Unavailable => {
                    return Err(SourceError::Configuration(
                        "No eligible OpenAlex API key is available.".to_string(),
                    ));
                }
            }
        }
    }

    fn finish(
        &self,
        reservation: &OpenAlexReservation,
        headers: OpenAlexRateHeaders,
        outcome: OpenAlexHealthOutcome,
        retry_delay: Duration,
    ) {
        if let Ok(mut state) = self.state.lock() {
            state.finish_slot(
                &reservation.slot,
                self.started_at.elapsed(),
                headers,
                outcome,
                retry_delay,
            );
            self.changed.notify_all();
        }
    }
}

fn openalex_rate_headers(headers: &HeaderMap) -> OpenAlexRateHeaders {
    OpenAlexRateHeaders {
        remaining: header_u64(headers, "x-ratelimit-remaining"),
        reset_after: header_u64(headers, "x-ratelimit-reset").map(Duration::from_secs),
        retry_after: header_u64(headers, "retry-after").map(Duration::from_secs),
    }
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn run_bounded_indexed<T, R, F>(
    items: &[T],
    requested_worker_count: usize,
    operation: F,
) -> Result<Vec<R>, SourceError>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let worker_count = requested_worker_count
        .clamp(1, OPENALEX_MAX_WORKERS_PER_PROCESS)
        .min(items.len());
    let next_index = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(items.len()));
    let did_panic = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let operation = &operation;
            let results = &results;
            let next_index = &next_index;
            handles.push(scope.spawn(move || loop {
                let index = next_index.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(index) else {
                    break;
                };
                let result = operation(index, item);
                results
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((index, result));
            }));
        }
        let mut did_panic = false;
        for handle in handles {
            did_panic |= handle.join().is_err();
        }
        did_panic
    });
    if did_panic {
        return Err(SourceError::Request {
            service: OPENALEX_SOURCE.to_string(),
            endpoint: "works".to_string(),
            message: "bounded OpenAlex worker failed".to_string(),
        });
    }
    let mut indexed = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    indexed.sort_by_key(|(index, _)| *index);
    Ok(indexed.into_iter().map(|(_, result)| result).collect())
}

fn openalex_doi_query(
    dois: &[String],
    api_key: Option<&str>,
    mailto: Option<&str>,
) -> Vec<(String, String)> {
    let mut query = vec![
        ("filter".to_string(), format!("doi:{}", dois.join("|"))),
        ("per-page".to_string(), dois.len().max(1).to_string()),
        ("select".to_string(), OPENALEX_WORK_FIELDS.to_string()),
    ];
    if let Some(api_key) = api_key {
        query.push(("api_key".to_string(), api_key.to_string()));
    }
    if let Some(mailto) = mailto {
        query.push(("mailto".to_string(), mailto.to_string()));
    }
    query
}

fn openalex_doi_request_url(
    dois: &[String],
    api_key: Option<&str>,
    mailto: Option<&str>,
) -> Result<Url, SourceError> {
    let mut url = Url::parse(&format!("{OPENALEX_BASE_URL}/works")).map_err(|_| {
        SourceError::Configuration(
            "OpenAlex DOI enrichment URL configuration is invalid.".to_string(),
        )
    })?;
    {
        let mut query_pairs = url.query_pairs_mut();
        for (name, value) in openalex_doi_query(dois, api_key, mailto) {
            query_pairs.append_pair(&name, &value);
        }
    }
    Ok(url)
}

fn partition_openalex_doi_batches(
    dois: &[String],
    requested_batch_size: usize,
    url_budget: usize,
    api_key: Option<&str>,
    mailto: Option<&str>,
) -> Result<Vec<Vec<String>>, SourceError> {
    let maximum_batch_size = requested_batch_size.clamp(1, OPENALEX_DOI_FILTER_MAX_VALUES);
    let mut batches = Vec::new();
    let mut current = Vec::new();
    for doi in dois {
        let mut candidate = current.clone();
        candidate.push(doi.clone());
        let does_candidate_fit = candidate.len() <= maximum_batch_size
            && openalex_doi_request_url(&candidate, api_key, mailto)?
                .as_str()
                .len()
                <= url_budget;
        if does_candidate_fit {
            current = candidate;
            continue;
        }
        if current.is_empty() {
            return Err(SourceError::Configuration(
                "One OpenAlex DOI enrichment value exceeds the request URL budget.".to_string(),
            ));
        }
        batches.push(std::mem::take(&mut current));
        current.push(doi.clone());
        if openalex_doi_request_url(&current, api_key, mailto)?
            .as_str()
            .len()
            > url_budget
        {
            return Err(SourceError::Configuration(
                "One OpenAlex DOI enrichment value exceeds the request URL budget.".to_string(),
            ));
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    Ok(batches)
}

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
    /// Partition normalized OpenAlex DOI values into transport-safe batches.
    ///
    /// # Arguments
    ///
    /// * `dois` - Unique normalized DOI values.
    /// * `requested_batch_size` - Requested maximum values per batch.
    ///
    /// # Returns
    ///
    /// Ordered DOI batches that fit the OpenAlex request contract.
    fn prepare_openalex_doi_batches(
        &self,
        dois: &[String],
        requested_batch_size: usize,
    ) -> Result<Vec<Vec<String>>, SourceError> {
        partition_openalex_doi_batches(
            dois,
            requested_batch_size,
            OPENALEX_DOI_REQUEST_URL_BUDGET,
            None,
            None,
        )
    }

    /// Execute ordered OpenAlex DOI batches.
    ///
    /// # Arguments
    ///
    /// * `batches` - Transport-safe DOI batches in logical order.
    ///
    /// # Returns
    ///
    /// JSON response payloads in the same logical order.
    fn request_openalex_doi_batches(
        &mut self,
        batches: &[Vec<String>],
    ) -> Result<Vec<Value>, SourceError> {
        let mut payloads = Vec::with_capacity(batches.len());
        for batch in batches {
            payloads.push(self.request(ScholarlyRequest {
                service: OPENALEX_SOURCE.to_string(),
                endpoint: "works".to_string(),
                method: "GET".to_string(),
                url: "https://api.openalex.org/works?filter=REDACTED&api_key=SECRET".to_string(),
                kind: ScholarlyRequestKind::OpenAlexWorksByDoi {
                    dois: batch.clone(),
                },
            })?);
        }
        Ok(payloads)
    }

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
    openalex_scheduler: Arc<OpenAlexScheduler>,
    openalex_worker_count: usize,
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
    attempt: usize,
    status_code: Option<u16>,
    did_succeed: bool,
    did_retry: bool,
    will_retry: bool,
    error_kind: &'static str,
    duration_ms: u64,
    error: Option<String>,
}

struct OpenAlexAttemptRecord {
    source_attempt: SourceAttempt,
    attempt_number: usize,
    key_slot: usize,
    will_retry: bool,
    error_kind: &'static str,
    duration_ms: u64,
}

struct OpenAlexExecution {
    result: Result<Value, SourceError>,
    attempts: Vec<OpenAlexAttemptRecord>,
}

fn execute_openalex_batches(
    client: &Client,
    scheduler: &Arc<OpenAlexScheduler>,
    worker_count: usize,
    url: &str,
    mailto: Option<&str>,
    batches: &[Vec<String>],
) -> Result<Vec<OpenAlexExecution>, SourceError> {
    let client = client.clone();
    let scheduler = Arc::clone(scheduler);
    let url = url.to_string();
    let mailto = mailto.map(str::to_string);
    run_bounded_indexed(batches, worker_count, move |_, batch| {
        let query = openalex_doi_query(batch, None, mailto.as_deref());
        execute_openalex_request(&client, scheduler.as_ref(), "works", &url, &query)
    })
}

fn execute_openalex_request(
    client: &Client,
    scheduler: &OpenAlexScheduler,
    endpoint: &str,
    url: &str,
    base_query: &[(String, String)],
) -> OpenAlexExecution {
    let maximum_attempts = scheduler
        .key_count()
        .max(1)
        .saturating_add(DEFAULT_MAX_RETRIES);
    let mut attempts = Vec::new();
    let mut excluded_slots = Vec::new();
    let mut last_error = None;
    for attempt_index in 0..maximum_attempts {
        let reservation = match scheduler.reserve(&excluded_slots) {
            Ok(reservation) => reservation,
            Err(error) => {
                return OpenAlexExecution {
                    result: Err(last_error.unwrap_or(error)),
                    attempts,
                };
            }
        };
        let attempt_number = attempt_index + 1;
        let retry_delay = Duration::from_secs(1_u64 << attempt_index.min(5));
        let mut query = base_query.to_vec();
        query.push(("api_key".to_string(), reservation.api_key.clone()));
        let request = match client.get(url).query(&query).build() {
            Ok(request) => request,
            Err(_) => {
                scheduler.finish(
                    &reservation,
                    OpenAlexRateHeaders::default(),
                    OpenAlexHealthOutcome::TerminalFailure,
                    Duration::ZERO,
                );
                return OpenAlexExecution {
                    result: Err(SourceError::Request {
                        service: OPENALEX_SOURCE.to_string(),
                        endpoint: endpoint.to_string(),
                        message: "request build failed".to_string(),
                    }),
                    attempts,
                };
            }
        };
        if base_query.iter().any(|(name, value)| {
            name == "filter" && value.trim_start().to_ascii_lowercase().starts_with("doi:")
        }) && request.url().as_str().len() > OPENALEX_DOI_REQUEST_URL_BUDGET
        {
            scheduler.finish(
                &reservation,
                OpenAlexRateHeaders::default(),
                OpenAlexHealthOutcome::TerminalFailure,
                Duration::ZERO,
            );
            return OpenAlexExecution {
                result: Err(SourceError::Configuration(
                    "OpenAlex DOI enrichment request exceeds the URL budget.".to_string(),
                )),
                attempts,
            };
        }
        let request_url = redact_url(request.url().as_ref());
        let started_at = Instant::now();
        match client.execute(request) {
            Ok(response) => {
                let status_code = response.status().as_u16();
                let headers = openalex_rate_headers(response.headers());
                let text = match response.text() {
                    Ok(text) => text,
                    Err(_) => {
                        let will_retry = attempt_number < maximum_attempts;
                        scheduler.finish(
                            &reservation,
                            headers,
                            OpenAlexHealthOutcome::TransientFailure,
                            retry_delay,
                        );
                        attempts.push(openalex_attempt_record(
                            endpoint,
                            &request_url,
                            attempt_number,
                            reservation.slot.slot_index,
                            Some(status_code),
                            false,
                            will_retry,
                            "response_body",
                            elapsed_millis(started_at),
                        ));
                        let error = SourceError::Request {
                            service: OPENALEX_SOURCE.to_string(),
                            endpoint: endpoint.to_string(),
                            message: "response body could not be read".to_string(),
                        };
                        if will_retry {
                            add_excluded_slot(&mut excluded_slots, reservation.slot.slot_index);
                            last_error = Some(error);
                            continue;
                        }
                        return OpenAlexExecution {
                            result: Err(error),
                            attempts,
                        };
                    }
                };
                let payload = serde_json::from_str::<Value>(&text)
                    .unwrap_or_else(|_| json!({ "error": "OpenAlex returned invalid JSON" }));
                if (200..300).contains(&status_code) {
                    scheduler.finish(
                        &reservation,
                        headers,
                        OpenAlexHealthOutcome::Success,
                        Duration::ZERO,
                    );
                    attempts.push(openalex_attempt_record(
                        endpoint,
                        &request_url,
                        attempt_number,
                        reservation.slot.slot_index,
                        Some(status_code),
                        true,
                        false,
                        "none",
                        elapsed_millis(started_at),
                    ));
                    return OpenAlexExecution {
                        result: Ok(payload),
                        attempts,
                    };
                }
                let health = match status_code {
                    401 | 403 => OpenAlexHealthOutcome::AuthenticationFailure,
                    429 => OpenAlexHealthOutcome::RateLimited,
                    status if RETRY_STATUS_CODES.contains(&status) => {
                        OpenAlexHealthOutcome::TransientFailure
                    }
                    _ => OpenAlexHealthOutcome::TerminalFailure,
                };
                let is_retryable = !matches!(health, OpenAlexHealthOutcome::TerminalFailure);
                let will_retry = is_retryable && attempt_number < maximum_attempts;
                scheduler.finish(&reservation, headers, health, retry_delay);
                attempts.push(openalex_attempt_record(
                    endpoint,
                    &request_url,
                    attempt_number,
                    reservation.slot.slot_index,
                    Some(status_code),
                    false,
                    will_retry,
                    "http_status",
                    elapsed_millis(started_at),
                ));
                let error = SourceError::HttpStatus {
                    service: OPENALEX_SOURCE.to_string(),
                    endpoint: endpoint.to_string(),
                    status_code,
                    body: safe_openalex_error_body(endpoint, &payload, &query),
                };
                if will_retry {
                    add_excluded_slot(&mut excluded_slots, reservation.slot.slot_index);
                    last_error = Some(error);
                    continue;
                }
                return OpenAlexExecution {
                    result: Err(error),
                    attempts,
                };
            }
            Err(_) => {
                let will_retry = attempt_number < maximum_attempts;
                scheduler.finish(
                    &reservation,
                    OpenAlexRateHeaders::default(),
                    OpenAlexHealthOutcome::TransientFailure,
                    retry_delay,
                );
                attempts.push(openalex_attempt_record(
                    endpoint,
                    &request_url,
                    attempt_number,
                    reservation.slot.slot_index,
                    None,
                    false,
                    will_retry,
                    "transport",
                    elapsed_millis(started_at),
                ));
                let error = SourceError::Request {
                    service: OPENALEX_SOURCE.to_string(),
                    endpoint: endpoint.to_string(),
                    message: "transport failure".to_string(),
                };
                if will_retry {
                    add_excluded_slot(&mut excluded_slots, reservation.slot.slot_index);
                    last_error = Some(error);
                    continue;
                }
                return OpenAlexExecution {
                    result: Err(error),
                    attempts,
                };
            }
        }
    }
    OpenAlexExecution {
        result: Err(last_error.unwrap_or_else(|| SourceError::Request {
            service: OPENALEX_SOURCE.to_string(),
            endpoint: endpoint.to_string(),
            message: "request retry loop exhausted".to_string(),
        })),
        attempts,
    }
}

#[allow(clippy::too_many_arguments)]
fn openalex_attempt_record(
    endpoint: &str,
    url: &str,
    attempt_number: usize,
    key_slot: usize,
    status_code: Option<u16>,
    did_succeed: bool,
    will_retry: bool,
    error_kind: &'static str,
    duration_ms: u64,
) -> OpenAlexAttemptRecord {
    OpenAlexAttemptRecord {
        source_attempt: SourceAttempt {
            service: OPENALEX_SOURCE.to_string(),
            endpoint: endpoint.to_string(),
            method: "GET".to_string(),
            url: url.to_string(),
            status_code,
            did_succeed,
            did_retry: attempt_number > 1,
            error: (!did_succeed).then(|| error_kind.to_string()),
        },
        attempt_number,
        key_slot,
        will_retry,
        error_kind,
        duration_ms,
    }
}

fn add_excluded_slot(excluded_slots: &mut Vec<usize>, slot_index: usize) {
    if !excluded_slots.contains(&slot_index) {
        excluded_slots.push(slot_index);
    }
}

fn safe_openalex_error_body(endpoint: &str, payload: &Value, query: &[(String, String)]) -> Value {
    if endpoint != "source_works" {
        return json!({ "error": "OpenAlex request failed" });
    }
    let mut safe = serde_json::Map::new();
    for name in ["error", "message"] {
        let Some(mut text) = payload
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        for (_, value) in query {
            if !value.is_empty() {
                text = text.replace(value, "[REDACTED]");
            }
        }
        safe.insert(name.to_string(), Value::String(text));
    }
    if safe.is_empty() {
        json!({ "error": "OpenAlex request failed" })
    } else {
        Value::Object(safe)
    }
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
        Self::new_with_openalex_workers(config, 1)
    }

    /// Build a live Scholarly transport with bounded OpenAlex enrichment workers.
    ///
    /// # Arguments
    ///
    /// * `config` - Live source configuration.
    /// * `openalex_worker_count` - Requested OpenAlex DOI requests in flight.
    ///
    /// # Returns
    ///
    /// Live transport or a request configuration error.
    pub fn new_with_openalex_workers(
        config: LiveScholarlyConfig,
        openalex_worker_count: usize,
    ) -> Result<Self, SourceError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.max(1)))
            .user_agent(DEFAULT_USER_AGENT)
            .build()
            .map_err(|error| SourceError::Request {
                service: "http".to_string(),
                endpoint: "client".to_string(),
                message: error.to_string(),
            })?;
        let openalex_scheduler = Arc::new(OpenAlexScheduler::new(
            config.openalex_api_keys.clone(),
            config.semantic_scholar_worker_id,
        ));
        Ok(Self {
            client,
            next_semantic_scholar_at: Some(
                Instant::now() + semantic_scholar_worker_offset(&config),
            ),
            config,
            attempts: Vec::new(),
            openalex_scheduler,
            openalex_worker_count: openalex_worker_count.clamp(1, OPENALEX_MAX_WORKERS_PER_PROCESS),
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
        self.append_openalex_mailto(&mut query);
        self.openalex_get_json(
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
        self.append_openalex_mailto(&mut query);
        self.openalex_get_json(
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
        self.append_openalex_mailto(&mut query);
        let mut payload = self.openalex_get_json(
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
        let mailto = self.config.crossref_mailtos.first().map(String::as_str);
        let query = openalex_doi_query(dois, None, mailto);
        self.openalex_get_json(
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

    fn append_openalex_mailto(&self, query: &mut Vec<(String, String)>) {
        if let Some(mailto) = self.config.crossref_mailtos.first() {
            query.push(("mailto".to_string(), mailto.clone()));
        }
    }

    fn openalex_get_json(
        &mut self,
        service: &str,
        endpoint: &str,
        url: &str,
        query: &[(String, String)],
    ) -> Result<Value, SourceError> {
        debug_assert_eq!(service, OPENALEX_SOURCE);
        let execution = execute_openalex_request(
            &self.client,
            self.openalex_scheduler.as_ref(),
            endpoint,
            url,
            query,
        );
        self.finish_openalex_execution(execution)
    }

    fn openalex_doi_batches(&mut self, batches: &[Vec<String>]) -> Result<Vec<Value>, SourceError> {
        let executions = execute_openalex_batches(
            &self.client,
            &self.openalex_scheduler,
            self.openalex_worker_count,
            &format!("{OPENALEX_BASE_URL}/works"),
            self.config.crossref_mailtos.first().map(String::as_str),
            batches,
        )?;
        let mut payloads = Vec::with_capacity(executions.len());
        let mut first_error = None;
        for execution in executions {
            let OpenAlexExecution { result, attempts } = execution;
            for attempt in attempts {
                self.record_openalex_attempt(attempt);
            }
            match result {
                Ok(payload) => payloads.push(payload),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(payloads), Err)
    }

    fn finish_openalex_execution(
        &mut self,
        execution: OpenAlexExecution,
    ) -> Result<Value, SourceError> {
        let OpenAlexExecution { result, attempts } = execution;
        for attempt in attempts {
            self.record_openalex_attempt(attempt);
        }
        result
    }

    fn record_openalex_attempt(&mut self, attempt: OpenAlexAttemptRecord) {
        let outcome = if attempt.source_attempt.did_succeed {
            "success"
        } else {
            "failure"
        };
        tracing::info!(
            event = "source.openalex.attempt",
            component = "source",
            provider = OPENALEX_SOURCE,
            endpoint = attempt.source_attempt.endpoint,
            method = attempt.source_attempt.method,
            attempt = attempt.attempt_number,
            key_slot = attempt.key_slot,
            outcome,
            error_kind = attempt.error_kind,
            http_status = attempt.source_attempt.status_code.unwrap_or(0),
            has_http_status = attempt.source_attempt.status_code.is_some(),
            is_retry = attempt.source_attempt.did_retry,
            will_retry = attempt.will_retry,
            duration_ms = attempt.duration_ms,
        );
        self.attempts.push(attempt.source_attempt);
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
            let started_at = Instant::now();
            let attempt_number = attempt + 1;
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
                    let text = match response.text() {
                        Ok(text) => text,
                        Err(error) => {
                            self.record_attempt(LiveAttempt {
                                service: live_request.service,
                                endpoint: live_request.endpoint,
                                method: live_request.method,
                                url: &request_url,
                                attempt: attempt_number,
                                status_code: Some(status_code),
                                did_succeed: false,
                                did_retry: attempt > 0,
                                will_retry: false,
                                error_kind: "response_body",
                                duration_ms: elapsed_millis(started_at),
                                error: Some(error.to_string()),
                            });
                            return Err(SourceError::Request {
                                service: live_request.service.to_string(),
                                endpoint: live_request.endpoint.to_string(),
                                message: error.to_string(),
                            });
                        }
                    };
                    let payload = serde_json::from_str::<Value>(&text)
                        .unwrap_or_else(|_| json!({ "error": text }));
                    if !(200..300).contains(&status_code) {
                        let will_retry = RETRY_STATUS_CODES.contains(&status_code)
                            && attempt < DEFAULT_MAX_RETRIES;
                        self.record_attempt(LiveAttempt {
                            service: live_request.service,
                            endpoint: live_request.endpoint,
                            method: live_request.method,
                            url: &request_url,
                            attempt: attempt_number,
                            status_code: Some(status_code),
                            did_succeed: false,
                            did_retry: attempt > 0,
                            will_retry,
                            error_kind: "http_status",
                            duration_ms: elapsed_millis(started_at),
                            error: payload
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                        if will_retry {
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
                        attempt: attempt_number,
                        status_code: Some(status_code),
                        did_succeed: true,
                        did_retry: attempt > 0,
                        will_retry: false,
                        error_kind: "none",
                        duration_ms: elapsed_millis(started_at),
                        error: None,
                    });
                    return Ok(payload);
                }
                Err(error) => {
                    let will_retry = attempt < DEFAULT_MAX_RETRIES;
                    self.record_attempt(LiveAttempt {
                        service: live_request.service,
                        endpoint: live_request.endpoint,
                        method: live_request.method,
                        url: &redact_url(live_request.url),
                        attempt: attempt_number,
                        status_code: None,
                        did_succeed: false,
                        did_retry: attempt > 0,
                        will_retry,
                        error_kind: "transport",
                        duration_ms: elapsed_millis(started_at),
                        error: Some(error.to_string()),
                    });
                    if will_retry {
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
        if !attempt.did_succeed {
            tracing::warn!(
                event = "source.request.failed",
                component = "source",
                provider = attempt.service,
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
    /// Partition OpenAlex DOI values using the longest configured credentials.
    fn prepare_openalex_doi_batches(
        &self,
        dois: &[String],
        requested_batch_size: usize,
    ) -> Result<Vec<Vec<String>>, SourceError> {
        let api_key = self
            .config
            .openalex_api_keys
            .iter()
            .max_by_key(|value| value.len())
            .map(String::as_str);
        let mailto = self
            .config
            .crossref_mailtos
            .iter()
            .max_by_key(|value| value.len())
            .map(String::as_str);
        partition_openalex_doi_batches(
            dois,
            requested_batch_size,
            OPENALEX_DOI_REQUEST_URL_BUDGET,
            api_key,
            mailto,
        )
    }

    /// Execute OpenAlex DOI batches with bounded concurrent workers.
    fn request_openalex_doi_batches(
        &mut self,
        batches: &[Vec<String>],
    ) -> Result<Vec<Value>, SourceError> {
        self.openalex_doi_batches(batches)
    }

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
                tracing::warn!(
                    event = "source.fallback.activated",
                    component = "source",
                    provider = OPENALEX_SOURCE,
                    endpoint = "source_works",
                    reason = "plan_restriction",
                    fallback = "full_source_pages",
                );
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
        let batches = self
            .transport
            .prepare_openalex_doi_batches(&normalized, batch_size)?;
        let payloads = self.transport.request_openalex_doi_batches(&batches)?;
        if payloads.len() != batches.len() {
            return Err(SourceError::InvalidFixture(
                "OpenAlex DOI batch response count does not match the request count.".to_string(),
            ));
        }
        for payload in payloads {
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
            } else if matches!(key, "filter" | "search" | "mailto" | "cursor") {
                format!("{key}=REDACTED")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{redacted}")
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
/// Shared structured-log capture helpers for source module tests.
pub(crate) mod test_support {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use serde_json::Value;
    use tracing_subscriber::fmt::MakeWriter;

    /// Thread-safe byte buffer used as a tracing test writer.
    #[derive(Clone, Default)]
    pub(crate) struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        /// Build a JSON tracing subscriber that records every level.
        ///
        /// # Returns
        ///
        /// Subscriber backed by this capture buffer.
        pub(crate) fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .with_writer(self.clone())
                .json()
                .flatten_event(true)
                .finish()
        }

        /// Return all captured bytes as UTF-8 text.
        ///
        /// # Returns
        ///
        /// Captured JSON Lines text.
        pub(crate) fn text(&self) -> String {
            String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured logs should be UTF-8")
        }

        /// Parse captured JSON Lines into event values.
        ///
        /// # Returns
        ///
        /// Parsed event objects in emission order.
        pub(crate) fn events(&self) -> Vec<Value> {
            self.text()
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_str(line).expect("captured log should be JSON"))
                .collect()
        }
    }

    /// Writer handle sharing a captured byte buffer.
    pub(crate) struct CapturedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedLogs {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use serde_json::json;

    use super::test_support::CapturedLogs;

    use super::{
        crossref_journal_filter, execute_openalex_batches, normalize_doi, normalize_issn,
        normalize_source_title, openalex_doi_query, openalex_doi_request_url,
        openalex_rate_headers, openalex_short_source_id, openalex_source_work_filter,
        partition_openalex_doi_batches, redact_url, run_bounded_indexed,
        semantic_scholar_worker_interval, semantic_scholar_worker_offset, value_pool_from_text,
        FixtureScholarlyTransport, LiveScholarlyConfig, LiveScholarlyTransport,
        OpenAlexHealthOutcome, OpenAlexRateHeaders, OpenAlexScheduleDecision, OpenAlexScheduler,
        OpenAlexSchedulerState, ScholarlyClient, ScholarlyFixtureData, ScholarlyTransport,
        SourceError, CROSSREF_ROWS, OPENALEX_DOI_FILTER_MAX_VALUES,
        OPENALEX_DOI_REQUEST_URL_BUDGET, OPENALEX_KEY_START_INTERVAL,
    };

    #[test]
    fn live_attempt_events_keep_worker_context_and_omit_request_material() {
        let sentinel = "source-url-query-header-body-sentinel";
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test request should connect");
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).expect("test request should read");
            let body = format!(r#"{{"error":"{sentinel}"}}"#);
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("test response should write");
        });
        let config = LiveScholarlyConfig {
            timeout_seconds: 2,
            openalex_api_keys: vec![sentinel.to_string()],
            semantic_scholar_api_keys: vec![sentinel.to_string()],
            crossref_mailtos: vec![sentinel.to_string()],
            semantic_scholar_worker_id: 7,
            semantic_scholar_process_count: 8,
            semantic_scholar_base_interval_ms: 0,
        };
        let mut transport =
            LiveScholarlyTransport::new(config).expect("live transport should build");
        let logs = CapturedLogs::default();
        let url = format!("http://{address}/works?credential={sentinel}");
        let body = json!({ "article_text": sentinel });

        let error = tracing::subscriber::with_default(logs.subscriber(), || {
            let span = tracing::info_span!(
                "index.worker",
                run_id = "run-source-correlation",
                worker_id = 7,
            );
            span.in_scope(|| {
                transport.post_json(
                    "semantic_scholar",
                    "privacy_test",
                    &url,
                    &[("query".to_string(), sentinel.to_string())],
                    &body,
                    Some(("x-api-key", sentinel.to_string())),
                )
            })
        })
        .expect_err("test request should return a non-retryable failure");
        server.join().expect("test server should finish");

        assert!(matches!(
            error,
            SourceError::HttpStatus {
                status_code: 400,
                ..
            }
        ));
        let events = logs.events();
        let failed = events
            .iter()
            .find(|event| event["event"] == "source.request.failed")
            .expect("source failure should be logged");
        assert_eq!(failed["provider"], "semantic_scholar");
        assert_eq!(failed["endpoint"], "privacy_test");
        assert_eq!(failed["attempt"], 1);
        assert_eq!(failed["http_status"], 400);
        assert_eq!(failed["will_retry"], false);
        assert_eq!(failed["span"]["run_id"], "run-source-correlation");
        assert_eq!(failed["span"]["worker_id"], 7);
        let text = logs.text();
        assert!(!text.contains(sentinel));
        assert!(!text.contains(&url));
    }

    #[test]
    fn openalex_fallback_event_is_symbolic_and_redacted() {
        let sentinel = "source-fallback-sentinel";
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            openalex_source_work_pages: vec![vec![json!({"id": "W1"})]],
            openalex_source_works_plan_restricted: true,
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);
        let logs = CapturedLogs::default();

        tracing::subscriber::with_default(logs.subscriber(), || {
            client
                .fetch_openalex_works_by_source_page(sentinel, Some("2026-01-01"), None)
                .expect("plan restriction should activate fallback")
        });

        let fallback = logs
            .events()
            .into_iter()
            .find(|event| event["event"] == "source.fallback.activated")
            .expect("fallback should be logged");
        assert_eq!(fallback["provider"], "openalex");
        assert_eq!(fallback["fallback"], "full_source_pages");
        assert!(!logs.text().contains(sentinel));
    }

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
    fn openalex_doi_batches_fit_the_encoded_url_budget_without_losing_values() {
        let dois = (0..225)
            .map(|index| {
                format!(
                    "10.1234/{index:03}-{}",
                    "x".repeat(12 + usize::try_from(index % 7).expect("remainder should fit"))
                )
            })
            .collect::<Vec<_>>();
        let api_key = "k".repeat(32);
        let mailto = "load-test@example.invalid";

        let batches = partition_openalex_doi_batches(
            &dois,
            OPENALEX_DOI_FILTER_MAX_VALUES,
            OPENALEX_DOI_REQUEST_URL_BUDGET,
            Some(&api_key),
            Some(mailto),
        )
        .expect("representative DOI values should partition");

        assert_eq!(batches.len(), 5);
        assert_eq!(batches.iter().flatten().cloned().collect::<Vec<_>>(), dois);
        for batch in &batches {
            assert!(batch.len() <= OPENALEX_DOI_FILTER_MAX_VALUES);
            let url = openalex_doi_request_url(batch, Some(&api_key), Some(mailto))
                .expect("batch URL should build");
            assert!(url.as_str().len() <= OPENALEX_DOI_REQUEST_URL_BUDGET);
        }
    }

    #[test]
    fn openalex_doi_batches_enforce_the_value_count_ceiling() {
        let dois = (0..205)
            .map(|index| format!("d{index}"))
            .collect::<Vec<_>>();

        let batches = partition_openalex_doi_batches(
            &dois,
            usize::MAX,
            OPENALEX_DOI_REQUEST_URL_BUDGET,
            None,
            None,
        )
        .expect("short DOI values should partition");

        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![100, 100, 5]
        );
    }

    #[test]
    fn openalex_doi_batches_fail_redacted_when_one_value_cannot_fit() {
        let oversized_doi = format!("10.1234/{}", "sensitive".repeat(300));

        let error = partition_openalex_doi_batches(
            std::slice::from_ref(&oversized_doi),
            OPENALEX_DOI_FILTER_MAX_VALUES,
            OPENALEX_DOI_REQUEST_URL_BUDGET,
            None,
            None,
        )
        .expect_err("one oversized DOI should fail before transport");

        assert!(matches!(error, SourceError::Configuration(_)));
        assert!(!error.to_string().contains(&oversized_doi));
    }

    #[test]
    fn live_openalex_batch_planning_uses_the_longest_configured_credentials() {
        let long_key = "long-key".repeat(24);
        let long_mailto = format!("{}@example.invalid", "m".repeat(96));
        let config = LiveScholarlyConfig {
            timeout_seconds: 2,
            openalex_api_keys: vec!["short".to_string(), long_key.clone()],
            semantic_scholar_api_keys: Vec::new(),
            crossref_mailtos: vec!["a@b.test".to_string(), long_mailto.clone()],
            semantic_scholar_worker_id: 0,
            semantic_scholar_process_count: 1,
            semantic_scholar_base_interval_ms: 1_000,
        };
        let transport =
            LiveScholarlyTransport::new(config).expect("live transport should initialize");
        let dois = (0..225)
            .map(|index| format!("10.1234/{index:03}-identifier"))
            .collect::<Vec<_>>();

        let batches = transport
            .prepare_openalex_doi_batches(&dois, usize::MAX)
            .expect("live DOI batches should plan");

        for batch in batches {
            let url = openalex_doi_request_url(
                &batch,
                Some(long_key.as_str()),
                Some(long_mailto.as_str()),
            )
            .expect("worst-case batch URL should build");
            assert!(url.as_str().len() <= OPENALEX_DOI_REQUEST_URL_BUDGET);
        }
    }

    #[test]
    fn openalex_doi_request_uses_bare_filter_values() {
        let dois = ["10.1/a".to_string(), "10.1/b".to_string()];
        let api_key = "key-value";
        let mailto = "contact@example.invalid";
        let url = openalex_doi_request_url(&dois, Some(api_key), Some(mailto))
            .expect("DOI request URL should build");
        let filter = url
            .query_pairs()
            .find_map(|(name, value)| (name == "filter").then(|| value.into_owned()))
            .expect("filter query should exist");
        let request = reqwest::blocking::Client::new()
            .get(format!("{}/works", super::OPENALEX_BASE_URL))
            .query(&openalex_doi_query(&dois, Some(api_key), Some(mailto)))
            .build()
            .expect("live-equivalent request should build");

        assert_eq!(filter, "doi:10.1/a|10.1/b");
        assert!(!filter.contains("https://doi.org/"));
        assert_eq!(request.url(), &url);
    }

    #[test]
    fn openalex_scheduler_offsets_equal_capacity_by_process() {
        for (process_id, expected_slot) in [(0, 0), (1, 1), (2, 2)] {
            let mut state = OpenAlexSchedulerState::new(3, process_id);

            let OpenAlexScheduleDecision::Reserved(reservation) =
                state.reserve_slot(Duration::ZERO, &[])
            else {
                panic!("an equal-capacity key should be reserved");
            };

            assert_eq!(reservation.slot_index, expected_slot);
        }
    }

    #[test]
    fn openalex_scheduler_enforces_pacing_and_ignores_stale_quota_increases() {
        let mut state = OpenAlexSchedulerState::new(1, 0);
        let OpenAlexScheduleDecision::Reserved(first) = state.reserve_slot(Duration::ZERO, &[])
        else {
            panic!("first key reservation should succeed");
        };
        assert_eq!(
            state.reserve_slot(Duration::ZERO, &[]),
            OpenAlexScheduleDecision::WaitUntil(OPENALEX_KEY_START_INTERVAL)
        );
        let OpenAlexScheduleDecision::Reserved(second) =
            state.reserve_slot(OPENALEX_KEY_START_INTERVAL, &[])
        else {
            panic!("second paced reservation should succeed");
        };
        state.finish_slot(
            &second,
            Duration::from_millis(40),
            OpenAlexRateHeaders {
                remaining: Some(10),
                reset_after: Some(Duration::from_secs(100)),
                retry_after: None,
            },
            OpenAlexHealthOutcome::Success,
            Duration::ZERO,
        );
        state.finish_slot(
            &first,
            Duration::from_millis(50),
            OpenAlexRateHeaders {
                remaining: Some(90),
                reset_after: Some(Duration::from_secs(100)),
                retry_after: None,
            },
            OpenAlexHealthOutcome::Success,
            Duration::ZERO,
        );

        assert_eq!(state.slots[0].remaining, Some(10));
        assert_eq!(state.slots[0].in_flight, 0);
    }

    #[test]
    fn openalex_scheduler_disables_auth_failures_and_cools_rate_limits() {
        let mut state = OpenAlexSchedulerState::new(3, 0);
        let OpenAlexScheduleDecision::Reserved(first) = state.reserve_slot(Duration::ZERO, &[])
        else {
            panic!("first key reservation should succeed");
        };
        state.finish_slot(
            &first,
            Duration::ZERO,
            OpenAlexRateHeaders::default(),
            OpenAlexHealthOutcome::AuthenticationFailure,
            Duration::ZERO,
        );
        assert!(state.slots[first.slot_index].is_disabled);

        let OpenAlexScheduleDecision::Reserved(second) = state.reserve_slot(Duration::ZERO, &[])
        else {
            panic!("second key reservation should succeed");
        };
        state.finish_slot(
            &second,
            Duration::ZERO,
            OpenAlexRateHeaders {
                remaining: Some(0),
                reset_after: Some(Duration::from_secs(2)),
                retry_after: None,
            },
            OpenAlexHealthOutcome::RateLimited,
            Duration::from_secs(1),
        );
        assert_eq!(
            state.slots[second.slot_index].cooldown_until,
            Some(Duration::from_secs(2))
        );

        let OpenAlexScheduleDecision::Reserved(third) = state.reserve_slot(Duration::ZERO, &[])
        else {
            panic!("healthy key should continue while another cools");
        };
        assert_ne!(third.slot_index, first.slot_index);
        assert_ne!(third.slot_index, second.slot_index);
    }

    #[test]
    fn openalex_scheduler_waits_for_all_key_reset_and_rejects_all_disabled() {
        let mut cooling = OpenAlexSchedulerState::new(1, 0);
        let OpenAlexScheduleDecision::Reserved(rate_limited) =
            cooling.reserve_slot(Duration::ZERO, &[])
        else {
            panic!("rate-limited key should first reserve");
        };
        cooling.finish_slot(
            &rate_limited,
            Duration::ZERO,
            OpenAlexRateHeaders {
                remaining: Some(0),
                reset_after: Some(Duration::from_secs(2)),
                retry_after: None,
            },
            OpenAlexHealthOutcome::RateLimited,
            Duration::from_secs(1),
        );
        assert_eq!(
            cooling.reserve_slot(Duration::ZERO, &[]),
            OpenAlexScheduleDecision::WaitUntil(Duration::from_secs(2))
        );
        assert!(matches!(
            cooling.reserve_slot(Duration::from_secs(2), &[]),
            OpenAlexScheduleDecision::Reserved(_)
        ));

        let mut disabled = OpenAlexSchedulerState::new(1, 0);
        let OpenAlexScheduleDecision::Reserved(invalid) =
            disabled.reserve_slot(Duration::ZERO, &[])
        else {
            panic!("invalid key should first reserve");
        };
        disabled.finish_slot(
            &invalid,
            Duration::ZERO,
            OpenAlexRateHeaders::default(),
            OpenAlexHealthOutcome::AuthenticationFailure,
            Duration::ZERO,
        );
        assert_eq!(
            disabled.reserve_slot(Duration::ZERO, &[]),
            OpenAlexScheduleDecision::Unavailable
        );
    }

    #[test]
    fn openalex_pacing_keeps_three_processes_below_one_hundred_rps_per_key() {
        let interval_ms = OPENALEX_KEY_START_INTERVAL.as_millis();

        assert!(1_000_u128.div_ceil(interval_ms) <= 30);
        assert!(3_u128.saturating_mul(1_000) < 100_u128.saturating_mul(interval_ms));
    }

    #[test]
    fn openalex_scheduler_weights_capacity_without_starving_lower_slots() {
        let mut state = OpenAlexSchedulerState::new(3, 0);
        state.slots[0].remaining = Some(90_000);
        state.slots[1].remaining = Some(30_000);
        state.slots[2].remaining = Some(10_000);
        let mut starts = [0_usize; 3];

        for index in 0..120_u64 {
            let now = OPENALEX_KEY_START_INTERVAL.saturating_mul(u32::try_from(index).unwrap());
            let OpenAlexScheduleDecision::Reserved(reservation) = state.reserve_slot(now, &[])
            else {
                panic!("one healthy key should always be ready");
            };
            starts[reservation.slot_index] += 1;
            state.finish_slot(
                &reservation,
                now,
                OpenAlexRateHeaders::default(),
                OpenAlexHealthOutcome::Success,
                Duration::ZERO,
            );
        }

        assert!(starts[0] > starts[1]);
        assert!(starts[1] > starts[2]);
        assert!(starts.iter().all(|count| *count > 0));
    }

    #[test]
    fn openalex_rate_headers_parse_remaining_reset_and_retry_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-ratelimit-remaining",
            reqwest::header::HeaderValue::from_static("8766"),
        );
        headers.insert(
            "x-ratelimit-reset",
            reqwest::header::HeaderValue::from_static("43200"),
        );
        headers.insert(
            "retry-after",
            reqwest::header::HeaderValue::from_static("3"),
        );

        assert_eq!(
            openalex_rate_headers(&headers),
            OpenAlexRateHeaders {
                remaining: Some(8_766),
                reset_after: Some(Duration::from_secs(43_200)),
                retry_after: Some(Duration::from_secs(3)),
            }
        );
    }

    #[test]
    fn live_openalex_request_fails_over_across_auth_and_quota_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let server = thread::spawn(move || {
            for (status, reason, headers, body) in [
                (401, "Unauthorized", "", r#"{"error":"invalid key"}"#),
                (
                    429,
                    "Too Many Requests",
                    "X-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 0\r\n",
                    r#"{"error":"quota"}"#,
                ),
                (503, "Service Unavailable", "", r#"{"error":"temporary"}"#),
                (
                    200,
                    "OK",
                    "X-RateLimit-Remaining: 99990\r\nX-RateLimit-Reset: 43200\r\n",
                    r#"{"results":[]}"#,
                ),
            ] {
                let (mut stream, _) = listener.accept().expect("test request should connect");
                let mut request = [0_u8; 8192];
                let _ = stream.read(&mut request).expect("test request should read");
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("test response should write");
            }
        });
        let scheduler = OpenAlexScheduler::new(
            vec![
                "key-zero".into(),
                "key-one".into(),
                "key-two".into(),
                "key-three".into(),
            ],
            0,
        );
        let execution = super::execute_openalex_request(
            &reqwest::blocking::Client::new(),
            &scheduler,
            "works",
            &format!("http://{address}/works"),
            &openalex_doi_query(&["10.1/example".to_string()], None, None),
        );
        server.join().expect("test server should finish");

        assert!(execution.result.is_ok());
        assert_eq!(
            execution
                .attempts
                .iter()
                .map(|attempt| attempt.key_slot)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            execution
                .attempts
                .iter()
                .map(|attempt| attempt.source_attempt.status_code)
                .collect::<Vec<_>>(),
            vec![Some(401), Some(429), Some(503), Some(200)]
        );
    }

    #[test]
    fn live_openalex_request_fails_over_after_a_transport_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let server = thread::spawn(move || {
            let (mut failed_stream, _) = listener.accept().expect("first request should connect");
            let mut first_request = [0_u8; 8192];
            let _ = failed_stream
                .read(&mut first_request)
                .expect("first request should read");
            drop(failed_stream);

            let (mut stream, _) = listener.accept().expect("retry should connect");
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).expect("retry should read");
            let body = r#"{"results":[]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-RateLimit-Remaining: 99990\r\nX-RateLimit-Reset: 43200\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("retry response should write");
        });
        let scheduler = OpenAlexScheduler::new(vec!["key-zero".into(), "key-one".into()], 0);
        let execution = super::execute_openalex_request(
            &reqwest::blocking::Client::new(),
            &scheduler,
            "works",
            &format!("http://{address}/works"),
            &openalex_doi_query(&["10.1/example".to_string()], None, None),
        );
        server.join().expect("test server should finish");

        assert!(execution.result.is_ok());
        assert_eq!(
            execution
                .attempts
                .iter()
                .map(|attempt| (attempt.key_slot, attempt.source_attempt.status_code))
                .collect::<Vec<_>>(),
            vec![(0, None), (1, Some(200))]
        );
    }

    #[test]
    fn live_openalex_failures_redact_key_mailto_doi_and_response_body() {
        let secret_key = "openalex-key-sentinel";
        let secret_mailto = "private-mailto@example.invalid";
        let secret_doi = "10.1/private-doi-sentinel";
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let response_body = format!(r#"{{"error":"{secret_key} {secret_mailto} {secret_doi}"}}"#);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test request should connect");
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).expect("test request should read");
            write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            )
            .expect("test response should write");
        });
        let scheduler = OpenAlexScheduler::new(vec![secret_key.to_string()], 0);
        let execution = super::execute_openalex_request(
            &reqwest::blocking::Client::new(),
            &scheduler,
            "works",
            &format!("http://{address}/works"),
            &openalex_doi_query(&[secret_doi.to_string()], None, Some(secret_mailto)),
        );
        server.join().expect("test server should finish");

        let error = execution
            .result
            .expect_err("one invalid key should fail without another request");
        let combined = format!(
            "{error:?} {scheduler:?} {:?}",
            execution.attempts[0].source_attempt
        );
        for secret in [secret_key, secret_mailto, secret_doi] {
            assert!(!combined.contains(secret));
        }
        assert_eq!(execution.attempts[0].key_slot, 0);
        assert!(execution.attempts[0]
            .source_attempt
            .url
            .contains("filter=REDACTED"));
        assert!(execution.attempts[0]
            .source_attempt
            .url
            .contains("api_key=SECRET"));
        let logs = CapturedLogs::default();
        let config = LiveScholarlyConfig {
            timeout_seconds: 2,
            openalex_api_keys: vec![secret_key.to_string()],
            semantic_scholar_api_keys: Vec::new(),
            crossref_mailtos: vec![secret_mailto.to_string()],
            semantic_scholar_worker_id: 0,
            semantic_scholar_process_count: 1,
            semantic_scholar_base_interval_ms: 1_000,
        };
        let mut transport =
            LiveScholarlyTransport::new(config).expect("live transport should initialize");
        tracing::subscriber::with_default(logs.subscriber(), || {
            for attempt in execution.attempts {
                transport.record_openalex_attempt(attempt);
            }
        });
        assert_eq!(
            logs.events()
                .iter()
                .find(|event| event["event"] == "source.openalex.attempt")
                .expect("OpenAlex attempt metric should be emitted")["key_slot"],
            0
        );
        for secret in [secret_key, secret_mailto, secret_doi] {
            assert!(!logs.text().contains(secret));
        }
    }

    #[test]
    fn bounded_openalex_work_preserves_order_and_caps_in_flight() {
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let items = (0..24).collect::<Vec<_>>();

        let output = run_bounded_indexed(&items, usize::MAX, |index, value| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(current, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(
                u64::try_from(7 - index % 7).expect("delay should fit"),
            ));
            active.fetch_sub(1, Ordering::SeqCst);
            (index, *value)
        })
        .expect("bounded work should complete");

        assert_eq!(
            output,
            (0..24).map(|value| (value, value)).collect::<Vec<_>>()
        );
        assert!((2..=6).contains(&maximum.load(Ordering::SeqCst)));
    }

    #[test]
    fn live_openalex_batches_use_all_keys_and_never_exceed_six_in_flight() {
        let request_count = 12;
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let server_active = Arc::clone(&active);
        let server_maximum = Arc::clone(&maximum);
        let server = thread::spawn(move || {
            let mut handlers = Vec::new();
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().expect("test request should connect");
                let active = Arc::clone(&server_active);
                let maximum = Arc::clone(&server_maximum);
                handlers.push(thread::spawn(move || {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    let mut request = [0_u8; 8192];
                    let _ = stream.read(&mut request).expect("test request should read");
                    thread::sleep(Duration::from_millis(80));
                    let body = r#"{"results":[]}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-RateLimit-Remaining: 99990\r\nX-RateLimit-Reset: 43200\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .expect("test response should write");
                    active.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            for handler in handlers {
                handler.join().expect("test response handler should finish");
            }
        });
        let scheduler = Arc::new(OpenAlexScheduler::new(
            vec!["key-zero".into(), "key-one".into(), "key-two".into()],
            0,
        ));
        let batches = (0..request_count)
            .map(|index| vec![format!("10.1/value-{index}")])
            .collect::<Vec<_>>();

        let executions = execute_openalex_batches(
            &reqwest::blocking::Client::new(),
            &scheduler,
            usize::MAX,
            &format!("http://{address}/works"),
            None,
            &batches,
        )
        .expect("bounded OpenAlex batches should execute");
        server.join().expect("test server should finish");

        assert_eq!(executions.len(), request_count);
        assert!(executions.iter().all(|execution| execution.result.is_ok()));
        assert!((4..=6).contains(&maximum.load(Ordering::SeqCst)));
        let mut key_starts = [0_usize; 3];
        for attempt in executions
            .iter()
            .flat_map(|execution| execution.attempts.iter())
        {
            key_starts[attempt.key_slot] += 1;
        }
        assert!(key_starts.iter().all(|count| *count > 0));
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
