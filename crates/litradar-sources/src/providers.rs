//! Built-in canonical indexing and request-time article access provider adapters.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use litradar_domain::{
    normalize_bibliographic_label, normalize_bibliographic_text, normalize_contract_date,
    normalize_contract_doi, normalize_contract_pmid, normalize_contract_text, ArticleAccessContext,
    ArticleAuthorDraft, ArticleDraft, ArticleLocator, ArticleRedirect, IndexFetchContext,
    IndexSyncMode, IssueDraft, JournalCatalogEntry, JournalDraft, ProviderBatch,
    ProviderCapabilityInfo, ProviderProgress,
};
use litradar_provider::{
    ArticleAbstractProvider, IndexContentProvider, ProviderCapabilities, ProviderDescriptor,
    ProviderError, ProviderErrorKind, ProviderImplementations, ProviderRegistration,
    ProviderRegistryError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cnki_domestic::{DomesticCnkiAnchor, DOMESTIC_CNKI_ANCHOR_VERSION};
use crate::{
    CnkiClient, CnkiSourceError, CnkiTransport, DomesticCnkiCheckpoint, DomesticCnkiClient,
    DomesticCnkiSourceError, DomesticCnkiTransport, DomesticIssueArticlePage,
    DomesticJournalLocator, ScholarlyClient, ScholarlyTransport, ScholarlyWorksPage, SourceAttempt,
    SourceError, SEMANTIC_SCHOLAR_BATCH_SIZE,
};

/// Stable runtime name for the built-in Scholarly indexing provider.
pub const SCHOLARLY_PROVIDER_NAME: &str = "scholarly";

/// Stable runtime name for the built-in overseas CNKI indexing provider.
pub const CNKI_OVERSEA_PROVIDER_NAME: &str = "cnki_oversea";

/// Stable runtime name for the built-in domestic NZKPT CNKI provider.
pub const CNKI_PROVIDER_NAME: &str = "cnki";

const DOMESTIC_CNKI_BATCH_ATTEMPT_LIMIT: usize = 3;

/// Stable runtime name for the Zhejiang Library full-text Provider.
pub const ZJLIB_PROVIDER_NAME: &str = "zjlib";

/// Exact HTTPS hosts emitted by the Scholarly online access provider.
pub const SCHOLARLY_REDIRECT_HOSTS: &[&str] = &["doi.org", "pubmed.ncbi.nlm.nih.gov"];

/// Exact HTTPS hosts emitted by the overseas CNKI online access provider.
pub const CNKI_REDIRECT_HOSTS: &[&str] = &["oversea.cnki.net", "kns.cnki.net", "www.cnki.net"];

/// Exact HTTPS hosts emitted by the domestic CNKI online access provider.
pub const DOMESTIC_CNKI_REDIRECT_HOSTS: &[&str] =
    &["navi.cnki.net", "kns.cnki.net", "www.cnki.net"];

/// Return aggregate capabilities for every built-in logical Provider.
///
/// # Returns
///
/// Deterministically ordered Provider capability metadata.
pub fn built_in_provider_capabilities() -> Vec<ProviderCapabilityInfo> {
    vec![
        ProviderCapabilityInfo {
            name: CNKI_PROVIDER_NAME.to_string(),
            index_content: true,
            article_abstract: true,
            article_full_text: false,
        },
        ProviderCapabilityInfo {
            name: CNKI_OVERSEA_PROVIDER_NAME.to_string(),
            index_content: true,
            article_abstract: true,
            article_full_text: false,
        },
        ProviderCapabilityInfo {
            name: SCHOLARLY_PROVIDER_NAME.to_string(),
            index_content: true,
            article_abstract: true,
            article_full_text: false,
        },
        ProviderCapabilityInfo {
            name: ZJLIB_PROVIDER_NAME.to_string(),
            index_content: false,
            article_abstract: false,
            article_full_text: true,
        },
    ]
}

const SCHOLARLY_ENRICHMENT_BATCH_SIZE: usize = 100;
const CROSSREF_CURSOR_REUSE_SECONDS: u64 = 240;

/// Stateless Scholarly access provider that derives live DOI or PubMed destinations.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScholarlyArticleAccessProvider;

impl ArticleAbstractProvider for ScholarlyArticleAccessProvider {
    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        _context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        scholarly_article_redirect(article)
    }
}

/// CNKI access provider that locates an article from canonical metadata on every request.
pub struct CnkiArticleAccessProvider<T> {
    client: Mutex<CnkiClient<T>>,
}

impl<T> CnkiArticleAccessProvider<T>
where
    T: CnkiTransport,
{
    /// Build a request-time CNKI access provider.
    ///
    /// # Arguments
    ///
    /// * `transport` - CNKI source transport.
    ///
    /// # Returns
    ///
    /// Provider that retains upstream handles only inside one invocation.
    pub fn new(transport: T) -> Self {
        Self {
            client: Mutex::new(CnkiClient::new(transport)),
        }
    }

    fn resolve(&self, article: &ArticleLocator) -> Result<ArticleRedirect, ProviderError> {
        let mut client = self.client.lock().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "CNKI access provider state is unavailable",
            )
        })?;
        let result = resolve_cnki_article_redirect(&mut client, article);
        emit_source_attempt_summary(CNKI_OVERSEA_PROVIDER_NAME, &client.drain_attempts());
        result
    }
}

impl<T> ArticleAbstractProvider for CnkiArticleAccessProvider<T>
where
    T: CnkiTransport + Send,
{
    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        _context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        self.resolve(article)
    }
}

/// Canonical Scholarly indexing provider backed by one source transport.
pub struct ScholarlyIndexProvider<T> {
    client: Mutex<ScholarlyClient<T>>,
    has_semantic_scholar_key: bool,
}

impl<T> ScholarlyIndexProvider<T>
where
    T: ScholarlyTransport,
{
    /// Build a canonical Scholarly provider.
    ///
    /// # Arguments
    ///
    /// * `transport` - Scholarly source transport.
    /// * `has_semantic_scholar_key` - Whether DOI enrichment is configured.
    ///
    /// # Returns
    ///
    /// Provider adapter that emits only canonical content batches.
    pub fn new(transport: T, has_semantic_scholar_key: bool) -> Self {
        Self {
            client: Mutex::new(ScholarlyClient::new(transport, has_semantic_scholar_key)),
            has_semantic_scholar_key,
        }
    }
}

impl<T> IndexContentProvider for ScholarlyIndexProvider<T>
where
    T: ScholarlyTransport + Send,
{
    fn fetch(
        &self,
        catalog: &JournalCatalogEntry,
        context: IndexFetchContext<'_>,
    ) -> Result<ProviderBatch, ProviderError> {
        let mut client = self.client.lock().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "scholarly provider state is unavailable",
            )
        })?;
        let result =
            fetch_scholarly_batch(&mut client, catalog, context, self.has_semantic_scholar_key);
        emit_source_attempt_summary(SCHOLARLY_PROVIDER_NAME, &client.drain_attempts());
        result
    }
}

/// Canonical CNKI indexing provider backed by one source transport.
pub struct CnkiIndexProvider<T> {
    client: Mutex<CnkiClient<T>>,
}

impl<T> CnkiIndexProvider<T>
where
    T: CnkiTransport,
{
    /// Build a canonical CNKI provider.
    ///
    /// # Arguments
    ///
    /// * `transport` - CNKI source transport.
    ///
    /// # Returns
    ///
    /// Provider adapter that discards all transport identifiers and links.
    pub fn new(transport: T) -> Self {
        Self {
            client: Mutex::new(CnkiClient::new(transport)),
        }
    }
}

impl<T> IndexContentProvider for CnkiIndexProvider<T>
where
    T: CnkiTransport + Send,
{
    fn fetch(
        &self,
        catalog: &JournalCatalogEntry,
        context: IndexFetchContext<'_>,
    ) -> Result<ProviderBatch, ProviderError> {
        if context.traversal_checkpoint.is_some() {
            return Err(ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "CNKI provider received an unsupported checkpoint",
            ));
        }
        let mut client = self.client.lock().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "CNKI provider state is unavailable",
            )
        })?;
        let result = fetch_cnki_batch(&mut client, catalog);
        emit_source_attempt_summary(CNKI_OVERSEA_PROVIDER_NAME, &client.drain_attempts());
        result
    }
}

/// Register one built-in Scholarly indexing capability.
///
/// # Arguments
///
/// * `transport` - Scholarly source transport.
/// * `has_semantic_scholar_key` - Whether Semantic Scholar enrichment is configured.
///
/// # Returns
///
/// Registration declaring exactly the canonical indexing capability.
pub fn scholarly_index_registration<T>(
    transport: T,
    has_semantic_scholar_key: bool,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: ScholarlyTransport + Send + 'static,
{
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: SCHOLARLY_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                index_content: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: Vec::new(),
        },
        ProviderImplementations {
            index_content: Some(Arc::new(ScholarlyIndexProvider::new(
                transport,
                has_semantic_scholar_key,
            ))),
            ..ProviderImplementations::default()
        },
    )
}

/// Register one built-in CNKI indexing capability.
///
/// # Arguments
///
/// * `transport` - CNKI source transport.
///
/// # Returns
///
/// Registration declaring exactly the canonical indexing capability.
pub fn cnki_oversea_index_registration<T>(
    transport: T,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: CnkiTransport + Send + 'static,
{
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_OVERSEA_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                index_content: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: Vec::new(),
        },
        ProviderImplementations {
            index_content: Some(Arc::new(CnkiIndexProvider::new(transport))),
            ..ProviderImplementations::default()
        },
    )
}

/// Register the Scholarly abstract-page access capability.
///
/// # Returns
///
/// Access-only Scholarly registration.
pub fn scholarly_access_registration() -> Result<ProviderRegistration, ProviderRegistryError> {
    let provider = Arc::new(ScholarlyArticleAccessProvider);
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: SCHOLARLY_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: SCHOLARLY_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

/// Register the CNKI abstract-page access capability.
///
/// # Arguments
///
/// * `transport` - CNKI source transport used only for request-time resolution.
///
/// # Returns
///
/// Access-only CNKI registration.
pub fn cnki_oversea_access_registration<T>(
    transport: T,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: CnkiTransport + Send + 'static,
{
    let provider = Arc::new(CnkiArticleAccessProvider::new(transport));
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_OVERSEA_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: CNKI_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

/// Domestic NZKPT access provider that locates an article from canonical metadata.
pub struct DomesticCnkiArticleAccessProvider<T> {
    client: Mutex<DomesticCnkiClient<T>>,
}

impl<T> DomesticCnkiArticleAccessProvider<T>
where
    T: DomesticCnkiTransport,
{
    /// Build a request-time domestic CNKI access provider.
    ///
    /// # Arguments
    ///
    /// * `transport` - Domestic CNKI source transport.
    ///
    /// # Returns
    ///
    /// Provider that retains upstream handles only inside one invocation.
    pub fn new(transport: T) -> Self {
        Self {
            client: Mutex::new(DomesticCnkiClient::new(transport)),
        }
    }

    fn resolve(&self, article: &ArticleLocator) -> Result<ArticleRedirect, ProviderError> {
        let mut client = self.client.lock().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "domestic CNKI access provider state is unavailable",
            )
        })?;
        let result = resolve_domestic_cnki_article_redirect(&mut client, article);
        emit_source_attempt_summary(CNKI_PROVIDER_NAME, &client.drain_attempts());
        result
    }
}

impl<T> ArticleAbstractProvider for DomesticCnkiArticleAccessProvider<T>
where
    T: DomesticCnkiTransport + Send,
{
    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        _context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        self.resolve(article)
    }
}

/// Canonical domestic CNKI indexing provider backed by one source transport.
pub struct DomesticCnkiIndexProvider<T> {
    state: Mutex<DomesticCnkiIndexState<T>>,
}

struct DomesticCnkiIndexState<T> {
    client: DomesticCnkiClient<T>,
    detail_pool: DomesticCnkiDetailPool,
    journal_snapshots: BTreeMap<String, DomesticCnkiJournalSnapshot>,
}

struct DomesticCnkiJournalSnapshot {
    journal: Value,
    issue_payloads: Vec<Value>,
}

impl<T> DomesticCnkiIndexProvider<T>
where
    T: DomesticCnkiTransport + Clone + Send + 'static,
{
    /// Build a canonical domestic CNKI provider.
    ///
    /// # Arguments
    ///
    /// * `transport` - Domestic CNKI source transport.
    ///
    /// # Returns
    ///
    /// Provider adapter that emits only canonical content batches.
    pub fn new(transport: T) -> Result<Self, ProviderRegistryError> {
        Self::with_worker_count(transport, 1)
    }

    /// Build a canonical domestic CNKI provider with bounded detail workers.
    ///
    /// # Arguments
    ///
    /// * `transport` - Domestic CNKI source transport.
    /// * `worker_count` - Maximum concurrent article detail requests.
    ///
    /// # Returns
    ///
    /// Provider adapter with an exact validated detail-worker count.
    pub fn with_worker_count(
        transport: T,
        worker_count: usize,
    ) -> Result<Self, ProviderRegistryError> {
        let worker_count = litradar_domain::validate_domestic_cnki_worker_count(worker_count)
            .map_err(|error| ProviderRegistryError::InvalidConfiguration {
                provider: CNKI_PROVIDER_NAME.to_string(),
                detail: error.to_string(),
            })?;
        let client = DomesticCnkiClient::new(transport);
        let detail_pool = DomesticCnkiDetailPool::new(&client, worker_count)?;
        Ok(Self {
            state: Mutex::new(DomesticCnkiIndexState {
                client,
                detail_pool,
                journal_snapshots: BTreeMap::new(),
            }),
        })
    }
}

impl<T> IndexContentProvider for DomesticCnkiIndexProvider<T>
where
    T: DomesticCnkiTransport + Clone + Send + 'static,
{
    fn fetch(
        &self,
        catalog: &JournalCatalogEntry,
        context: IndexFetchContext<'_>,
    ) -> Result<ProviderBatch, ProviderError> {
        let mut state = self.state.lock().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "domestic CNKI provider state is unavailable",
            )
        })?;
        let mut attempts = Vec::new();
        let mut batch_attempt = 1;
        let result = loop {
            let mut detail_attempts = Vec::new();
            let result = {
                let DomesticCnkiIndexState {
                    client,
                    detail_pool,
                    journal_snapshots,
                } = &mut *state;
                fetch_domestic_cnki_batch(
                    client,
                    journal_snapshots,
                    catalog,
                    context,
                    detail_pool,
                    &mut detail_attempts,
                )
            };
            attempts.append(&mut state.client.drain_attempts());
            attempts.append(&mut detail_attempts);
            let should_retry = result.as_ref().is_err_and(|error| {
                is_retryable_domestic_batch_error(error)
                    && batch_attempt < DOMESTIC_CNKI_BATCH_ATTEMPT_LIMIT
            });
            if !should_retry {
                break result;
            }
            let error = result.as_ref().expect_err("retry requires an error");
            tracing::info!(
                event = "index.provider.batch.retry",
                component = "index",
                provider = CNKI_PROVIDER_NAME,
                failed_attempt = batch_attempt,
                next_attempt = batch_attempt + 1,
                failure_kind = ?error.kind(),
            );
            if let Err(error) = reset_domestic_cnki_batch_state(&mut state) {
                break Err(error);
            }
            thread::sleep(domestic_batch_retry_delay(batch_attempt));
            batch_attempt += 1;
        };
        emit_source_attempt_summary(CNKI_PROVIDER_NAME, &attempts);
        if result
            .as_ref()
            .is_ok_and(|batch| matches!(&batch.progress, ProviderProgress::Complete { .. }))
        {
            state.journal_snapshots.remove(&catalog.catalog_id);
        }
        result
    }
}

fn reset_domestic_cnki_batch_state<T>(
    state: &mut DomesticCnkiIndexState<T>,
) -> Result<(), ProviderError>
where
    T: DomesticCnkiTransport + Clone + Send + 'static,
{
    state
        .client
        .reset_transient_state()
        .map_err(map_domestic_cnki_error)?;
    let detail_pool = DomesticCnkiDetailPool::new(&state.client, state.detail_pool.worker_count)
        .map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "domestic CNKI detail worker pool could not restart",
            )
        })?;
    state.detail_pool = detail_pool;
    Ok(())
}

fn is_retryable_domestic_batch_error(error: &ProviderError) -> bool {
    matches!(
        error.kind(),
        ProviderErrorKind::TemporarilyUnavailable | ProviderErrorKind::InvalidResponse
    )
}

fn domestic_batch_retry_delay(failed_attempt: usize) -> Duration {
    let exponent = failed_attempt.saturating_sub(1).min(3) as u32;
    Duration::from_secs(1_u64 << exponent)
}

/// Register one built-in domestic CNKI indexing capability.
///
/// # Arguments
///
/// * `transport` - Domestic CNKI source transport.
///
/// # Returns
///
/// Registration declaring exactly the canonical indexing capability.
pub fn cnki_index_registration<T>(
    transport: T,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: DomesticCnkiTransport + Clone + Send + 'static,
{
    cnki_index_registration_with_workers(transport, 1)
}

/// Register domestic CNKI indexing with bounded per-process detail workers.
///
/// # Arguments
///
/// * `transport` - Domestic CNKI source transport.
/// * `worker_count` - Maximum concurrent article detail requests.
///
/// # Returns
///
/// Registration declaring exactly the canonical indexing capability.
pub fn cnki_index_registration_with_workers<T>(
    transport: T,
    worker_count: usize,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: DomesticCnkiTransport + Clone + Send + 'static,
{
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                index_content: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: Vec::new(),
        },
        ProviderImplementations {
            index_content: Some(Arc::new(DomesticCnkiIndexProvider::with_worker_count(
                transport,
                worker_count,
            )?)),
            ..ProviderImplementations::default()
        },
    )
}

/// Register the domestic CNKI abstract-page access capability.
///
/// # Arguments
///
/// * `transport` - Domestic CNKI source transport used only for request-time resolution.
///
/// # Returns
///
/// Access-only domestic CNKI registration.
pub fn cnki_access_registration<T>(
    transport: T,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: DomesticCnkiTransport + Send + 'static,
{
    let provider = Arc::new(DomesticCnkiArticleAccessProvider::new(transport));
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: DOMESTIC_CNKI_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

const SCHOLARLY_ANCHOR_VERSION: u32 = 1;
const SCHOLARLY_CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ScholarlyIssueFingerprint {
    VolumeIssue {
        publication_year: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        volume: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        issue: Option<String>,
    },
    Date {
        date: String,
    },
    Title {
        #[serde(skip_serializing_if = "Option::is_none")]
        publication_year: Option<i64>,
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScholarlyAnchor {
    version: u32,
    issue: ScholarlyIssueFingerprint,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_sync_date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScholarlyScanPhase {
    Bounded,
    Unbounded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScholarlyWindowCheckpoint {
    sync_mode: IndexSyncMode,
    phase: ScholarlyScanPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_anchor: Option<ScholarlyAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_anchor: Option<ScholarlyAnchor>,
    has_reached_candidate: bool,
    has_seen_base: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ScholarlySourceCheckpoint {
    Crossref {
        issn: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
        page_index: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor_refreshed_at_epoch_seconds: Option<u64>,
    },
    OpenAlex {
        source_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScholarlyCheckpoint {
    version: u32,
    window: ScholarlyWindowCheckpoint,
    source: ScholarlySourceCheckpoint,
}

struct ScholarlyCanonicalPage {
    articles: Vec<ArticleDraft>,
    has_unfingerprintable_item: bool,
    precomputed_window_plan: Option<ScholarlyPageWindowPlan>,
    source_head: ScholarlySourceCheckpoint,
    next_source: Option<ScholarlySourceCheckpoint>,
    did_restart_from_head: bool,
    did_fallback_to_unfiltered: bool,
}

struct ScholarlyCrossrefPageContext<'a> {
    catalog: &'a JournalCatalogEntry,
    window: &'a ScholarlyWindowCheckpoint,
    source: ScholarlySourceCheckpoint,
    did_restart_from_head: bool,
}

struct ScholarlyPageWindowPlan {
    selected_indices: Vec<usize>,
    window: ScholarlyWindowCheckpoint,
    progress: ScholarlyPageProgress,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScholarlyPageProgress {
    Continue,
    Complete,
    ReplayUnbounded,
}

fn current_epoch_seconds() -> Result<u64, ProviderError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "system clock is before the Unix epoch",
            )
        })
}

fn crossref_cursor_is_fresh(
    cursor_refreshed_at_epoch_seconds: Option<u64>,
    current_epoch_seconds: u64,
) -> bool {
    cursor_refreshed_at_epoch_seconds
        .and_then(|refreshed_at| current_epoch_seconds.checked_sub(refreshed_at))
        .is_some_and(|age| age < CROSSREF_CURSOR_REUSE_SECONDS)
}

fn crossref_checkpoint_epoch(
    next_cursor: Option<&String>,
    is_empty: bool,
    clock: &mut impl FnMut() -> Result<u64, ProviderError>,
) -> Result<Option<u64>, ProviderError> {
    if next_cursor.is_some() && !is_empty {
        clock().map(Some)
    } else {
        Ok(None)
    }
}

fn is_crossref_cursor_http_500(error: &SourceError) -> bool {
    matches!(
        error,
        SourceError::HttpStatus {
            status_code: 500,
            ..
        }
    )
}

fn emit_crossref_cursor_restart(reason: &'static str, prior_page_index: u64) {
    tracing::warn!(
        event = "source.crossref.cursor_restarted",
        component = "source",
        provider = "crossref",
        reason,
        prior_page_index,
    );
}

fn fetch_scholarly_batch<T>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    context: IndexFetchContext<'_>,
    has_semantic_scholar_key: bool,
) -> Result<ProviderBatch, ProviderError>
where
    T: ScholarlyTransport,
{
    let mut clock = current_epoch_seconds;
    fetch_scholarly_batch_for_context_with_clock_and_restart(
        client,
        catalog,
        context,
        has_semantic_scholar_key,
        &mut clock,
        &mut emit_crossref_cursor_restart,
    )
}

#[cfg(test)]
fn fetch_scholarly_batch_with_clock<T, F>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    checkpoint: Option<&str>,
    has_semantic_scholar_key: bool,
    clock: &mut F,
) -> Result<ProviderBatch, ProviderError>
where
    T: ScholarlyTransport,
    F: FnMut() -> Result<u64, ProviderError>,
{
    let mut restart = emit_crossref_cursor_restart;
    fetch_scholarly_batch_for_context_with_clock_and_restart(
        client,
        catalog,
        IndexFetchContext {
            mode: IndexSyncMode::Bootstrap,
            committed_anchor: None,
            traversal_checkpoint: checkpoint,
        },
        has_semantic_scholar_key,
        clock,
        &mut restart,
    )
}

#[cfg(test)]
fn fetch_scholarly_batch_with_clock_and_restart<T, F, R>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    checkpoint: Option<&str>,
    has_semantic_scholar_key: bool,
    clock: &mut F,
    restart: &mut R,
) -> Result<ProviderBatch, ProviderError>
where
    T: ScholarlyTransport,
    F: FnMut() -> Result<u64, ProviderError>,
    R: FnMut(&'static str, u64),
{
    fetch_scholarly_batch_for_context_with_clock_and_restart(
        client,
        catalog,
        IndexFetchContext {
            mode: IndexSyncMode::Bootstrap,
            committed_anchor: None,
            traversal_checkpoint: checkpoint,
        },
        has_semantic_scholar_key,
        clock,
        restart,
    )
}

fn fetch_scholarly_batch_for_context_with_clock_and_restart<T, F, R>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    context: IndexFetchContext<'_>,
    has_semantic_scholar_key: bool,
    clock: &mut F,
    restart: &mut R,
) -> Result<ProviderBatch, ProviderError>
where
    T: ScholarlyTransport,
    F: FnMut() -> Result<u64, ProviderError>,
    R: FnMut(&'static str, u64),
{
    let (mut window, source) = scholarly_window_from_context(context)?;
    let page = match source {
        Some(source) => fetch_scholarly_source_page(
            client,
            catalog,
            &window,
            source,
            has_semantic_scholar_key,
            clock,
            restart,
        )?,
        None => {
            fetch_first_scholarly_page(client, catalog, &window, has_semantic_scholar_key, clock)?
        }
    };
    if page.did_restart_from_head {
        prepare_scholarly_replay(&mut window);
    }
    if page.did_fallback_to_unfiltered {
        switch_scholarly_window_to_unbounded(&mut window);
    }
    apply_scholarly_page(catalog, window, page)
}

fn fetch_first_scholarly_page<T>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    window: &ScholarlyWindowCheckpoint,
    has_semantic_scholar_key: bool,
    clock: &mut impl FnMut() -> Result<u64, ProviderError>,
) -> Result<ScholarlyCanonicalPage, ProviderError>
where
    T: ScholarlyTransport,
{
    let from_sync_date = scholarly_window_filter(window);
    let issns = catalog_issns(catalog);
    for issn in &issns {
        match client.fetch_journal_works_page(issn, from_sync_date, None) {
            Ok(page) if page.items.is_empty() && window.phase == ScholarlyScanPhase::Unbounded => {}
            Ok(page) => {
                return crossref_canonical_page(
                    client,
                    ScholarlyCrossrefPageContext {
                        catalog,
                        window,
                        source: ScholarlySourceCheckpoint::Crossref {
                            issn: issn.clone(),
                            cursor: None,
                            page_index: 0,
                            cursor_refreshed_at_epoch_seconds: None,
                        },
                        did_restart_from_head: false,
                    },
                    page,
                    has_semantic_scholar_key,
                    clock,
                )
            }
            Err(SourceError::HttpStatus {
                status_code: 404, ..
            }) => {}
            Err(error) => return Err(map_scholarly_error(error)),
        }
    }

    let source = client
        .fetch_openalex_source_by_issns(&issns)
        .map_err(map_scholarly_error)?;
    let source = match source {
        Some(source) => Some(source),
        None => client
            .fetch_openalex_source_by_title(&catalog.title)
            .map_err(map_scholarly_error)?,
    }
    .ok_or_else(|| {
        ProviderError::new(
            ProviderErrorKind::NotFound,
            "scholarly provider could not resolve the journal",
        )
    })?;
    let source_id = json_text(source.get("id")).ok_or_else(|| {
        ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "OpenAlex source has no identifier",
        )
    })?;
    let page = client
        .fetch_openalex_works_by_source_page(&source_id, from_sync_date, None)
        .map_err(map_scholarly_error)?;
    openalex_canonical_page(
        catalog,
        ScholarlySourceCheckpoint::OpenAlex {
            source_id,
            cursor: None,
        },
        page,
        false,
    )
}

fn fetch_scholarly_source_page<T>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    window: &ScholarlyWindowCheckpoint,
    source: ScholarlySourceCheckpoint,
    has_semantic_scholar_key: bool,
    clock: &mut impl FnMut() -> Result<u64, ProviderError>,
    restart: &mut impl FnMut(&'static str, u64),
) -> Result<ScholarlyCanonicalPage, ProviderError>
where
    T: ScholarlyTransport,
{
    match source {
        ScholarlySourceCheckpoint::Crossref {
            issn,
            cursor,
            page_index,
            cursor_refreshed_at_epoch_seconds,
        } => {
            let mut source = ScholarlySourceCheckpoint::Crossref {
                issn: issn.clone(),
                cursor: cursor.clone(),
                page_index,
                cursor_refreshed_at_epoch_seconds,
            };
            let mut did_restart_from_head = false;
            if cursor.is_some()
                && !crossref_cursor_is_fresh(cursor_refreshed_at_epoch_seconds, clock()?)
            {
                restart("expired_or_legacy", page_index);
                source = reset_scholarly_source(&source);
                did_restart_from_head = true;
            }
            let (request_cursor, request_page_index) = match &source {
                ScholarlySourceCheckpoint::Crossref {
                    cursor, page_index, ..
                } => (cursor.as_deref(), *page_index),
                ScholarlySourceCheckpoint::OpenAlex { .. } => unreachable!(),
            };
            let from_sync_date = scholarly_window_filter(window);
            let page = match client.fetch_journal_works_page(&issn, from_sync_date, request_cursor)
            {
                Ok(page) => page,
                Err(error) if request_cursor.is_some() && is_crossref_cursor_http_500(&error) => {
                    restart("cursor_http_500", request_page_index);
                    source = reset_scholarly_source(&source);
                    did_restart_from_head = true;
                    client
                        .fetch_journal_works_page(&issn, from_sync_date, None)
                        .map_err(map_scholarly_error)?
                }
                Err(error) => return Err(map_scholarly_error(error)),
            };
            crossref_canonical_page(
                client,
                ScholarlyCrossrefPageContext {
                    catalog,
                    window,
                    source,
                    did_restart_from_head,
                },
                page,
                has_semantic_scholar_key,
                clock,
            )
        }
        ScholarlySourceCheckpoint::OpenAlex { source_id, cursor } => {
            let page = client
                .fetch_openalex_works_by_source_page(
                    &source_id,
                    scholarly_window_filter(window),
                    cursor.as_deref(),
                )
                .map_err(map_scholarly_error)?;
            openalex_canonical_page(
                catalog,
                ScholarlySourceCheckpoint::OpenAlex { source_id, cursor },
                page,
                false,
            )
        }
    }
}

fn crossref_canonical_page<T>(
    client: &mut ScholarlyClient<T>,
    context: ScholarlyCrossrefPageContext<'_>,
    page: ScholarlyWorksPage,
    has_semantic_scholar_key: bool,
    clock: &mut impl FnMut() -> Result<u64, ProviderError>,
) -> Result<ScholarlyCanonicalPage, ProviderError>
where
    T: ScholarlyTransport,
{
    let ScholarlyCrossrefPageContext {
        catalog,
        window,
        source,
        did_restart_from_head,
    } = context;
    let is_empty = page.items.is_empty();
    let cursor_refreshed_at_epoch_seconds =
        crossref_checkpoint_epoch(page.next_cursor.as_ref(), is_empty, clock)?;
    let next_source = next_scholarly_source(
        &source,
        page.next_cursor,
        is_empty,
        cursor_refreshed_at_epoch_seconds,
    )?;
    let mut page_window = window.clone();
    if did_restart_from_head {
        prepare_scholarly_replay(&mut page_window);
    }
    let mut precomputed_window_plan = if page_window.phase == ScholarlyScanPhase::Bounded {
        let anchors = page
            .items
            .iter()
            .map(crossref_work_issue_anchor)
            .collect::<Vec<_>>();
        let has_unfingerprintable_item = anchors.iter().any(Option::is_none);
        Some(plan_scholarly_page_window(
            page_window.clone(),
            &anchors,
            has_unfingerprintable_item,
            next_source.is_some(),
        )?)
    } else {
        None
    };
    let selected_indices = precomputed_window_plan
        .as_ref()
        .map(|plan| plan.selected_indices.clone())
        .unwrap_or_else(|| (0..page.items.len()).collect());
    let selected_count = selected_indices.len();
    let mut articles = enrich_crossref_articles(
        client,
        catalog,
        &page.items,
        &selected_indices,
        has_semantic_scholar_key,
    )?;
    let has_unfingerprintable_item = articles.len() != selected_count
        || articles
            .iter()
            .any(|article| scholarly_issue_anchor(article).is_none());
    if page_window.phase == ScholarlyScanPhase::Bounded && has_unfingerprintable_item {
        articles.clear();
        precomputed_window_plan = Some(plan_scholarly_page_window(
            page_window,
            &[],
            true,
            next_source.is_some(),
        )?);
    }
    Ok(ScholarlyCanonicalPage {
        articles,
        has_unfingerprintable_item: precomputed_window_plan.is_none() && has_unfingerprintable_item,
        precomputed_window_plan,
        source_head: reset_scholarly_source(&source),
        next_source,
        did_restart_from_head,
        did_fallback_to_unfiltered: false,
    })
}

fn openalex_canonical_page(
    catalog: &JournalCatalogEntry,
    source: ScholarlySourceCheckpoint,
    page: ScholarlyWorksPage,
    did_restart_from_head: bool,
) -> Result<ScholarlyCanonicalPage, ProviderError> {
    let is_empty = page.items.is_empty();
    let effective_source = if page.did_fallback_to_unfiltered {
        reset_scholarly_source(&source)
    } else {
        source
    };
    let next_source = next_scholarly_source(&effective_source, page.next_cursor, is_empty, None)?;
    let articles = page
        .items
        .iter()
        .filter_map(|work| openalex_article_draft(catalog, work))
        .collect::<Vec<_>>();
    let has_unfingerprintable_item = articles.len() != page.items.len()
        || articles
            .iter()
            .any(|article| scholarly_issue_anchor(article).is_none());
    Ok(ScholarlyCanonicalPage {
        articles,
        has_unfingerprintable_item,
        precomputed_window_plan: None,
        source_head: reset_scholarly_source(&effective_source),
        next_source,
        did_restart_from_head,
        did_fallback_to_unfiltered: page.did_fallback_to_unfiltered,
    })
}

fn enrich_crossref_articles<T>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    works: &[Value],
    selected_indices: &[usize],
    has_semantic_scholar_key: bool,
) -> Result<Vec<ArticleDraft>, ProviderError>
where
    T: ScholarlyTransport,
{
    let dois = selected_indices
        .iter()
        .filter_map(|index| works.get(*index))
        .filter_map(|work| normalize_contract_doi(json_text(work.get("DOI"))?.as_str()))
        .collect::<Vec<_>>();
    let openalex = if dois.is_empty() {
        BTreeMap::new()
    } else {
        client
            .fetch_openalex_by_dois(&dois, SCHOLARLY_ENRICHMENT_BATCH_SIZE)
            .map_err(map_scholarly_error)?
    };
    let semantic_scholar = if dois.is_empty() || !has_semantic_scholar_key {
        BTreeMap::new()
    } else {
        client
            .fetch_semantic_scholar_by_dois(&dois, SEMANTIC_SCHOLAR_BATCH_SIZE)
            .map_err(map_scholarly_error)?
    };
    Ok(selected_indices
        .iter()
        .filter_map(|index| works.get(*index))
        .filter_map(|work| {
            let doi = normalize_contract_doi(json_text(work.get("DOI"))?.as_str());
            scholarly_article_draft(
                catalog,
                work,
                doi.as_ref().and_then(|value| openalex.get(value)),
                doi.as_ref().and_then(|value| semantic_scholar.get(value)),
            )
        })
        .collect())
}

fn next_scholarly_source(
    current: &ScholarlySourceCheckpoint,
    next_cursor: Option<String>,
    is_empty: bool,
    cursor_refreshed_at_epoch_seconds: Option<u64>,
) -> Result<Option<ScholarlySourceCheckpoint>, ProviderError> {
    let Some(next_cursor) = next_cursor.filter(|_| !is_empty) else {
        return Ok(None);
    };
    let next = match current {
        ScholarlySourceCheckpoint::Crossref {
            issn, page_index, ..
        } => ScholarlySourceCheckpoint::Crossref {
            issn: issn.clone(),
            cursor: Some(next_cursor),
            page_index: page_index.checked_add(1).ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "scholarly Crossref checkpoint page index overflowed",
                )
            })?,
            cursor_refreshed_at_epoch_seconds: Some(cursor_refreshed_at_epoch_seconds.ok_or_else(
                || {
                    ProviderError::new(
                        ProviderErrorKind::Internal,
                        "scholarly Crossref checkpoint timestamp is unavailable",
                    )
                },
            )?),
        },
        ScholarlySourceCheckpoint::OpenAlex { source_id, cursor } => {
            if cursor.as_deref() == Some(next_cursor.as_str()) {
                return Err(ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "scholarly provider returned a repeated cursor",
                ));
            }
            ScholarlySourceCheckpoint::OpenAlex {
                source_id: source_id.clone(),
                cursor: Some(next_cursor),
            }
        }
    };
    Ok(Some(next))
}

fn reset_scholarly_source(source: &ScholarlySourceCheckpoint) -> ScholarlySourceCheckpoint {
    match source {
        ScholarlySourceCheckpoint::Crossref { issn, .. } => ScholarlySourceCheckpoint::Crossref {
            issn: issn.clone(),
            cursor: None,
            page_index: 0,
            cursor_refreshed_at_epoch_seconds: None,
        },
        ScholarlySourceCheckpoint::OpenAlex { source_id, .. } => {
            ScholarlySourceCheckpoint::OpenAlex {
                source_id: source_id.clone(),
                cursor: None,
            }
        }
    }
}

fn scholarly_window_from_context(
    context: IndexFetchContext<'_>,
) -> Result<(ScholarlyWindowCheckpoint, Option<ScholarlySourceCheckpoint>), ProviderError> {
    let committed_anchor = match (context.mode, context.committed_anchor) {
        (IndexSyncMode::Incremental, Some(anchor)) => Some(decode_scholarly_anchor(anchor)?),
        _ => None,
    };
    if let Some(checkpoint) = context.traversal_checkpoint {
        let checkpoint = decode_scholarly_checkpoint(checkpoint)?;
        if checkpoint.window.sync_mode != context.mode
            || checkpoint.window.base_anchor != committed_anchor
        {
            return Err(ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "scholarly checkpoint does not match the frozen synchronization window",
            ));
        }
        return Ok((checkpoint.window, Some(checkpoint.source)));
    }
    let phase = if committed_anchor
        .as_ref()
        .and_then(|anchor| anchor.from_sync_date.as_ref())
        .is_some()
    {
        ScholarlyScanPhase::Bounded
    } else {
        ScholarlyScanPhase::Unbounded
    };
    Ok((
        ScholarlyWindowCheckpoint {
            sync_mode: context.mode,
            phase,
            base_anchor: committed_anchor,
            candidate_anchor: None,
            has_reached_candidate: false,
            has_seen_base: false,
        },
        None,
    ))
}

fn scholarly_window_filter(window: &ScholarlyWindowCheckpoint) -> Option<&str> {
    let current_date = current_utc_date();
    scholarly_window_filter_at(window, current_date.as_deref())
}

fn scholarly_window_filter_at<'a>(
    window: &'a ScholarlyWindowCheckpoint,
    current_date: Option<&str>,
) -> Option<&'a str> {
    (window.phase == ScholarlyScanPhase::Bounded)
        .then(|| {
            window
                .base_anchor
                .as_ref()
                .and_then(|anchor| anchor.from_sync_date.as_deref())
        })
        .flatten()
        .filter(|from_sync_date| {
            current_date.is_none_or(|current_date| *from_sync_date <= current_date)
        })
}

fn current_utc_date() -> Option<String> {
    let epoch_seconds = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let epoch_seconds = i64::try_from(epoch_seconds).ok()?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(epoch_seconds, 0)
        .map(|date_time| date_time.date_naive().to_string())
}

fn prepare_scholarly_replay(window: &mut ScholarlyWindowCheckpoint) {
    window.has_reached_candidate = false;
    window.has_seen_base = false;
}

fn switch_scholarly_window_to_unbounded(window: &mut ScholarlyWindowCheckpoint) {
    window.phase = ScholarlyScanPhase::Unbounded;
    prepare_scholarly_replay(window);
}

fn plan_scholarly_page_window(
    mut window: ScholarlyWindowCheckpoint,
    issue_anchors: &[Option<ScholarlyAnchor>],
    has_unfingerprintable_item: bool,
    has_next_page: bool,
) -> Result<ScholarlyPageWindowPlan, ProviderError> {
    if window.phase == ScholarlyScanPhase::Bounded && has_unfingerprintable_item {
        switch_scholarly_window_to_unbounded(&mut window);
        return Ok(ScholarlyPageWindowPlan {
            selected_indices: Vec::new(),
            window,
            progress: ScholarlyPageProgress::ReplayUnbounded,
        });
    }
    let mut selected_indices = Vec::new();
    for (index, issue_anchor) in issue_anchors.iter().enumerate() {
        if window.phase == ScholarlyScanPhase::Bounded && issue_anchor.is_none() {
            switch_scholarly_window_to_unbounded(&mut window);
            return Ok(ScholarlyPageWindowPlan {
                selected_indices: Vec::new(),
                window,
                progress: ScholarlyPageProgress::ReplayUnbounded,
            });
        }
        if window.candidate_anchor.is_none() {
            if let Some(issue_anchor) = issue_anchor.as_ref() {
                if window.phase == ScholarlyScanPhase::Bounded
                    && window.base_anchor.as_ref().is_some_and(|base| {
                        scholarly_issue_is_older(&issue_anchor.issue, &base.issue)
                    })
                {
                    switch_scholarly_window_to_unbounded(&mut window);
                    return Ok(ScholarlyPageWindowPlan {
                        selected_indices: Vec::new(),
                        window,
                        progress: ScholarlyPageProgress::ReplayUnbounded,
                    });
                }
                window.candidate_anchor = Some(issue_anchor.clone());
                window.has_reached_candidate = true;
            }
        } else if !window.has_reached_candidate {
            if issue_anchor.as_ref().is_some_and(|anchor| {
                window
                    .candidate_anchor
                    .as_ref()
                    .is_some_and(|candidate| anchor.issue == candidate.issue)
            }) {
                window.has_reached_candidate = true;
            } else {
                continue;
            }
        }
        if window.has_reached_candidate
            && issue_anchor.as_ref().is_some_and(|anchor| {
                window.candidate_anchor.as_ref().is_some_and(|candidate| {
                    anchor.issue != candidate.issue
                        && scholarly_issue_is_older(&candidate.issue, &anchor.issue)
                })
            })
        {
            continue;
        }
        if window.phase == ScholarlyScanPhase::Bounded {
            let issue_anchor = issue_anchor
                .as_ref()
                .expect("bounded articles require an issue anchor");
            if window.has_seen_base
                && window
                    .base_anchor
                    .as_ref()
                    .is_some_and(|base| issue_anchor.issue != base.issue)
            {
                return Ok(ScholarlyPageWindowPlan {
                    selected_indices,
                    window,
                    progress: ScholarlyPageProgress::Complete,
                });
            }
            if window
                .base_anchor
                .as_ref()
                .is_some_and(|base| issue_anchor.issue == base.issue)
            {
                window.has_seen_base = true;
            }
        }
        if window.candidate_anchor.is_none() || window.has_reached_candidate {
            selected_indices.push(index);
        }
    }
    if has_next_page {
        return Ok(ScholarlyPageWindowPlan {
            selected_indices,
            window,
            progress: ScholarlyPageProgress::Continue,
        });
    }
    let progress = match window.phase {
        ScholarlyScanPhase::Bounded if window.has_seen_base => ScholarlyPageProgress::Complete,
        ScholarlyScanPhase::Bounded => {
            switch_scholarly_window_to_unbounded(&mut window);
            ScholarlyPageProgress::ReplayUnbounded
        }
        ScholarlyScanPhase::Unbounded
            if window.candidate_anchor.is_some() && !window.has_reached_candidate =>
        {
            return Err(ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "scholarly frozen candidate issue disappeared during replay",
            ));
        }
        ScholarlyScanPhase::Unbounded => ScholarlyPageProgress::Complete,
    };
    Ok(ScholarlyPageWindowPlan {
        selected_indices,
        window,
        progress,
    })
}

fn apply_scholarly_page(
    catalog: &JournalCatalogEntry,
    window: ScholarlyWindowCheckpoint,
    page: ScholarlyCanonicalPage,
) -> Result<ProviderBatch, ProviderError> {
    let ScholarlyCanonicalPage {
        articles,
        has_unfingerprintable_item,
        precomputed_window_plan,
        source_head,
        next_source,
        ..
    } = page;
    let are_articles_preselected = precomputed_window_plan.is_some();
    let plan = match precomputed_window_plan {
        Some(plan) => plan,
        None => {
            let issue_anchors = articles
                .iter()
                .map(scholarly_issue_anchor)
                .collect::<Vec<_>>();
            plan_scholarly_page_window(
                window,
                &issue_anchors,
                has_unfingerprintable_item,
                next_source.is_some(),
            )?
        }
    };
    let ScholarlyPageWindowPlan {
        selected_indices,
        window,
        progress,
    } = plan;
    let articles = if are_articles_preselected {
        articles
    } else {
        let mut selected_indices = selected_indices.into_iter();
        let mut next_selected = selected_indices.next();
        articles
            .into_iter()
            .enumerate()
            .filter_map(|(index, article)| {
                if next_selected == Some(index) {
                    next_selected = selected_indices.next();
                    Some(article)
                } else {
                    None
                }
            })
            .collect()
    };
    match progress {
        ScholarlyPageProgress::Continue => {
            let Some(next_source) = next_source else {
                return Err(ProviderError::new(
                    ProviderErrorKind::Internal,
                    "scholarly page plan requires a missing continuation source",
                ));
            };
            let checkpoint = encode_scholarly_checkpoint(&ScholarlyCheckpoint {
                version: SCHOLARLY_CHECKPOINT_VERSION,
                window,
                source: next_source,
            })?;
            Ok(batch_from_articles(
                catalog,
                articles,
                ProviderProgress::Continue { checkpoint },
            ))
        }
        ScholarlyPageProgress::Complete => scholarly_complete_batch(catalog, articles, &window),
        ScholarlyPageProgress::ReplayUnbounded => {
            scholarly_unbounded_replay_batch(catalog, window, source_head)
        }
    }
}

fn scholarly_unbounded_replay_batch(
    catalog: &JournalCatalogEntry,
    mut window: ScholarlyWindowCheckpoint,
    source_head: ScholarlySourceCheckpoint,
) -> Result<ProviderBatch, ProviderError> {
    switch_scholarly_window_to_unbounded(&mut window);
    let checkpoint = encode_scholarly_checkpoint(&ScholarlyCheckpoint {
        version: SCHOLARLY_CHECKPOINT_VERSION,
        window,
        source: source_head,
    })?;
    Ok(batch_from_articles(
        catalog,
        Vec::new(),
        ProviderProgress::Continue { checkpoint },
    ))
}

fn scholarly_complete_batch(
    catalog: &JournalCatalogEntry,
    articles: Vec<ArticleDraft>,
    window: &ScholarlyWindowCheckpoint,
) -> Result<ProviderBatch, ProviderError> {
    let next_anchor = window
        .candidate_anchor
        .as_ref()
        .or(window.base_anchor.as_ref())
        .map(encode_scholarly_anchor)
        .transpose()?;
    Ok(batch_from_articles(
        catalog,
        articles,
        ProviderProgress::Complete { next_anchor },
    ))
}

fn crossref_work_issue_anchor(work: &Value) -> Option<ScholarlyAnchor> {
    let date = crossref_date(work);
    let publication_year = date
        .as_deref()
        .and_then(|value| value.get(..4))
        .and_then(|value| value.parse().ok());
    let volume = json_text(work.get("volume"));
    let issue_number = json_text(work.get("issue"));
    scholarly_issue_anchor_from_fields(
        publication_year,
        date.as_deref(),
        None,
        volume.as_deref(),
        issue_number.as_deref(),
    )
}

fn scholarly_issue_anchor(article: &ArticleDraft) -> Option<ScholarlyAnchor> {
    scholarly_issue_anchor_from_fields(
        article.publication_year,
        article.date.as_deref(),
        article.issue_title.as_deref(),
        article.volume.as_deref(),
        article.issue_number.as_deref(),
    )
}

fn scholarly_issue_anchor_from_fields(
    publication_year: Option<i64>,
    date: Option<&str>,
    issue_title: Option<&str>,
    volume: Option<&str>,
    issue_number: Option<&str>,
) -> Option<ScholarlyAnchor> {
    let volume = volume
        .map(normalize_bibliographic_label)
        .filter(|value| !value.is_empty());
    let issue = issue_number
        .map(normalize_bibliographic_label)
        .filter(|value| !value.is_empty());
    let fingerprint = if let Some(publication_year) = publication_year.filter(valid_year) {
        if volume.is_some() || issue.is_some() {
            Some(ScholarlyIssueFingerprint::VolumeIssue {
                publication_year,
                volume,
                issue,
            })
        } else {
            date.map(|date| ScholarlyIssueFingerprint::Date {
                date: date.to_string(),
            })
            .or_else(|| scholarly_title_fingerprint(issue_title, Some(publication_year)))
        }
    } else {
        date.map(|date| ScholarlyIssueFingerprint::Date {
            date: date.to_string(),
        })
        .or_else(|| scholarly_title_fingerprint(issue_title, None))
    }?;
    let from_sync_date = publication_year
        .filter(valid_year)
        .or_else(|| scholarly_fingerprint_year(&fingerprint))
        .map(|year| format!("{year:04}-01-01"));
    Some(ScholarlyAnchor {
        version: SCHOLARLY_ANCHOR_VERSION,
        issue: fingerprint,
        from_sync_date,
    })
}

fn scholarly_title_fingerprint(
    issue_title: Option<&str>,
    publication_year: Option<i64>,
) -> Option<ScholarlyIssueFingerprint> {
    let title = normalize_bibliographic_text(issue_title?);
    (!title.is_empty()).then_some(ScholarlyIssueFingerprint::Title {
        publication_year,
        title,
    })
}

fn scholarly_fingerprint_year(fingerprint: &ScholarlyIssueFingerprint) -> Option<i64> {
    match fingerprint {
        ScholarlyIssueFingerprint::VolumeIssue {
            publication_year, ..
        } => Some(*publication_year),
        ScholarlyIssueFingerprint::Date { date } => date.get(..4)?.parse().ok(),
        ScholarlyIssueFingerprint::Title {
            publication_year, ..
        } => *publication_year,
    }
}

fn scholarly_issue_is_older(
    candidate: &ScholarlyIssueFingerprint,
    base: &ScholarlyIssueFingerprint,
) -> bool {
    match (
        scholarly_fingerprint_year(candidate),
        scholarly_fingerprint_year(base),
    ) {
        (Some(candidate_year), Some(base_year)) if candidate_year != base_year => {
            return candidate_year < base_year;
        }
        _ => {}
    }
    match (candidate, base) {
        (
            ScholarlyIssueFingerprint::Date {
                date: candidate_date,
            },
            ScholarlyIssueFingerprint::Date { date: base_date },
        ) => candidate_date < base_date,
        (
            ScholarlyIssueFingerprint::VolumeIssue {
                volume: candidate_volume,
                issue: candidate_issue,
                ..
            },
            ScholarlyIssueFingerprint::VolumeIssue {
                volume: base_volume,
                issue: base_issue,
                ..
            },
        ) => numeric_label_is_older(candidate_volume.as_deref(), base_volume.as_deref())
            .or_else(|| numeric_label_is_older(candidate_issue.as_deref(), base_issue.as_deref()))
            .unwrap_or(false),
        _ => false,
    }
}

fn numeric_label_is_older(candidate: Option<&str>, base: Option<&str>) -> Option<bool> {
    let candidate = candidate?.parse::<u64>().ok()?;
    let base = base?.parse::<u64>().ok()?;
    (candidate != base).then_some(candidate < base)
}

fn valid_year(year: &i64) -> bool {
    (1..=9_999).contains(year)
}

fn encode_scholarly_anchor(anchor: &ScholarlyAnchor) -> Result<String, ProviderError> {
    serde_json::to_string(anchor).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::Internal,
            "scholarly anchor could not be encoded",
        )
    })
}

fn decode_scholarly_anchor(raw: &str) -> Result<ScholarlyAnchor, ProviderError> {
    let anchor = serde_json::from_str::<ScholarlyAnchor>(raw).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "scholarly committed anchor is invalid",
        )
    })?;
    if !is_valid_scholarly_anchor(&anchor) {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "scholarly committed anchor is invalid",
        ));
    }
    Ok(anchor)
}

fn is_valid_scholarly_anchor(anchor: &ScholarlyAnchor) -> bool {
    if anchor.version != SCHOLARLY_ANCHOR_VERSION
        || anchor.from_sync_date.as_ref().is_some_and(|date| {
            normalize_contract_date(date)
                .is_none_or(|normalized| normalized.value != *date || date.len() != 10)
        })
    {
        return false;
    }
    match &anchor.issue {
        ScholarlyIssueFingerprint::VolumeIssue {
            publication_year,
            volume,
            issue,
        } => {
            valid_year(publication_year)
                && (volume.is_some() || issue.is_some())
                && [volume.as_deref(), issue.as_deref()]
                    .into_iter()
                    .flatten()
                    .all(|value| !value.is_empty() && normalize_bibliographic_label(value) == value)
        }
        ScholarlyIssueFingerprint::Date { date } => {
            normalize_contract_date(date).is_some_and(|normalized| normalized.value == *date)
        }
        ScholarlyIssueFingerprint::Title {
            publication_year,
            title,
        } => {
            publication_year.as_ref().is_none_or(valid_year)
                && !title.is_empty()
                && normalize_bibliographic_text(title) == *title
        }
    }
}

fn encode_scholarly_checkpoint(checkpoint: &ScholarlyCheckpoint) -> Result<String, ProviderError> {
    serde_json::to_string(checkpoint).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::Internal,
            "scholarly checkpoint could not be encoded",
        )
    })
}

fn decode_scholarly_checkpoint(raw: &str) -> Result<ScholarlyCheckpoint, ProviderError> {
    let mut checkpoint = serde_json::from_str::<ScholarlyCheckpoint>(raw).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "scholarly checkpoint is invalid",
        )
    })?;
    normalize_empty_unbounded_replay_checkpoint(&mut checkpoint);
    let is_source_valid = match &checkpoint.source {
        ScholarlySourceCheckpoint::Crossref {
            issn,
            cursor,
            page_index,
            cursor_refreshed_at_epoch_seconds,
        } => {
            !issn.trim().is_empty()
                && cursor.as_ref().is_none_or(|cursor| !cursor.is_empty())
                && match cursor {
                    Some(_) => cursor_refreshed_at_epoch_seconds.is_some(),
                    None => *page_index == 0 && cursor_refreshed_at_epoch_seconds.is_none(),
                }
        }
        ScholarlySourceCheckpoint::OpenAlex { source_id, cursor } => {
            !source_id.trim().is_empty() && cursor.as_ref().is_none_or(|cursor| !cursor.is_empty())
        }
    };
    let is_window_valid = checkpoint
        .window
        .base_anchor
        .as_ref()
        .is_none_or(is_valid_scholarly_anchor)
        && checkpoint
            .window
            .candidate_anchor
            .as_ref()
            .is_none_or(is_valid_scholarly_anchor)
        && (checkpoint.window.phase != ScholarlyScanPhase::Bounded
            || checkpoint
                .window
                .base_anchor
                .as_ref()
                .and_then(|anchor| anchor.from_sync_date.as_ref())
                .is_some())
        && (checkpoint.window.candidate_anchor.is_some()
            || (!checkpoint.window.has_reached_candidate && !checkpoint.window.has_seen_base))
        && (!checkpoint.window.has_reached_candidate
            || checkpoint.window.candidate_anchor.is_some())
        && (!checkpoint.window.has_seen_base
            || (checkpoint.window.phase == ScholarlyScanPhase::Bounded
                && checkpoint.window.has_reached_candidate));
    if checkpoint.version != SCHOLARLY_CHECKPOINT_VERSION || !is_source_valid || !is_window_valid {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "scholarly checkpoint is invalid",
        ));
    }
    Ok(checkpoint)
}

fn normalize_empty_unbounded_replay_checkpoint(checkpoint: &mut ScholarlyCheckpoint) {
    let current_date = current_utc_date();
    normalize_empty_unbounded_replay_checkpoint_at(checkpoint, current_date.as_deref());
}

fn normalize_empty_unbounded_replay_checkpoint_at(
    checkpoint: &mut ScholarlyCheckpoint,
    current_date: Option<&str>,
) {
    let is_source_head = match &checkpoint.source {
        ScholarlySourceCheckpoint::Crossref {
            cursor,
            page_index,
            cursor_refreshed_at_epoch_seconds,
            ..
        } => cursor.is_none() && *page_index == 0 && cursor_refreshed_at_epoch_seconds.is_none(),
        ScholarlySourceCheckpoint::OpenAlex { cursor, .. } => cursor.is_none(),
    };
    if checkpoint.version == SCHOLARLY_CHECKPOINT_VERSION
        && checkpoint.window.phase == ScholarlyScanPhase::Unbounded
        && checkpoint.window.base_anchor.is_some()
        && checkpoint.window.candidate_anchor.is_none()
        && checkpoint.window.has_reached_candidate
        && !checkpoint.window.has_seen_base
        && is_source_head
    {
        checkpoint.window.has_reached_candidate = false;
        if checkpoint
            .window
            .base_anchor
            .as_ref()
            .and_then(|anchor| anchor.from_sync_date.as_deref())
            .is_some_and(|from_sync_date| {
                current_date.is_some_and(|current_date| from_sync_date > current_date)
            })
        {
            checkpoint.window.phase = ScholarlyScanPhase::Bounded;
        }
    }
}

fn fetch_cnki_batch<T>(
    client: &mut CnkiClient<T>,
    catalog: &JournalCatalogEntry,
) -> Result<ProviderBatch, ProviderError>
where
    T: CnkiTransport,
{
    let row = BTreeMap::from([
        ("catalog_id".to_string(), catalog.catalog_id.clone()),
        ("title".to_string(), catalog.title.clone()),
        ("issn".to_string(), catalog.issn.clone().unwrap_or_default()),
    ]);
    let journal = client
        .resolve_journal(&row)
        .map_err(map_cnki_error)?
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::NotFound,
                "CNKI provider could not resolve the journal",
            )
        })?;
    let issue_payloads = client.year_issues(&journal).map_err(map_cnki_error)?;
    let mut issues = Vec::new();
    let mut articles = Vec::new();
    for issue_payload in issue_payloads {
        let Some(issue) = cnki_issue_draft(catalog, &issue_payload) else {
            continue;
        };
        for summary in client
            .issue_articles(&journal, &issue_payload)
            .map_err(map_cnki_error)?
        {
            let Some(article_url) = json_text(summary.get("article_url")) else {
                continue;
            };
            let platform_id = json_text(summary.get("platform_id"));
            let detail = client
                .article_detail(&article_url, platform_id.as_deref())
                .map_err(map_cnki_error)?;
            if let Some(article) = cnki_article_draft(catalog, &issue, &summary, &detail) {
                articles.push(article);
            }
        }
        issues.push(issue);
    }
    Ok(ProviderBatch {
        catalog_id: catalog.catalog_id.clone(),
        journal: journal_observation(catalog),
        issues,
        articles,
        progress: ProviderProgress::Complete { next_anchor: None },
    })
}

fn batch_from_articles(
    catalog: &JournalCatalogEntry,
    articles: Vec<ArticleDraft>,
    progress: ProviderProgress,
) -> ProviderBatch {
    let mut issue_keys = BTreeSet::new();
    let issues = articles
        .iter()
        .filter_map(issue_from_article)
        .filter(|issue| {
            issue_keys.insert((
                issue.publication_year,
                issue.volume.clone(),
                issue.number.clone(),
                issue.date.clone(),
                issue.title.clone(),
            ))
        })
        .collect();
    ProviderBatch {
        catalog_id: catalog.catalog_id.clone(),
        journal: journal_observation(catalog),
        issues,
        articles,
        progress,
    }
}

fn journal_observation(catalog: &JournalCatalogEntry) -> JournalDraft {
    JournalDraft {
        catalog_id: catalog.catalog_id.clone(),
        observed_title: Some(catalog.title.clone()),
        observed_issns: catalog_issns(catalog),
        observed_title_aliases: Vec::new(),
    }
}

fn issue_from_article(article: &ArticleDraft) -> Option<IssueDraft> {
    let issue = IssueDraft {
        catalog_id: article.catalog_id.clone(),
        publication_year: article.publication_year,
        title: article.issue_title.clone(),
        volume: article.volume.clone(),
        number: article.issue_number.clone(),
        date: article.date.clone(),
    };
    (issue.publication_year.is_some() && (issue.volume.is_some() || issue.number.is_some())
        || issue.date.is_some()
        || issue.title.is_some())
    .then_some(issue)
}

fn scholarly_article_draft(
    catalog: &JournalCatalogEntry,
    work: &Value,
    openalex: Option<&Value>,
    semantic_scholar: Option<&Value>,
) -> Option<ArticleDraft> {
    let title = first_text(work.get("title"))?;
    let date = crossref_date(work);
    let publication_year = date
        .as_deref()
        .and_then(|value| value.get(..4))
        .and_then(|value| value.parse().ok());
    let doi = json_text(work.get("DOI")).and_then(|value| normalize_contract_doi(&value));
    let pmid = json_text(work.get("PMID")).and_then(|value| normalize_contract_pmid(&value));
    let volume = json_text(work.get("volume"));
    let issue_number = json_text(work.get("issue"));
    let (start_page, end_page) = split_pages(json_text(work.get("page")).as_deref());
    let authors = crossref_authors(work.get("author"));
    let abstract_text = json_text(work.get("abstract"))
        .and_then(|value| strip_markup(&value))
        .or_else(|| openalex.and_then(openalex_abstract))
        .or_else(|| semantic_scholar.and_then(|value| json_text(value.get("abstract"))));
    let open_access = semantic_scholar
        .and_then(|value| value.get("isOpenAccess"))
        .and_then(Value::as_bool)
        .or_else(|| {
            openalex.map(|value| {
                value
                    .get("best_oa_location")
                    .is_some_and(|location| !location.is_null())
            })
        });
    canonical_article(ArticleDraft {
        catalog_id: catalog.catalog_id.clone(),
        title,
        publication_year,
        date,
        issue_title: None,
        volume,
        issue_number,
        authors,
        start_page,
        end_page,
        abstract_text,
        doi,
        pmid,
        open_access,
        in_press: Some(work.get("issue").is_none()),
        retraction_dois: updated_by_retraction_dois(work.get("updated-by")),
    })
}

fn openalex_article_draft(catalog: &JournalCatalogEntry, work: &Value) -> Option<ArticleDraft> {
    let title = json_text(work.get("display_name")).or_else(|| json_text(work.get("title")))?;
    let date = json_text(work.get("publication_date"));
    let publication_year = work
        .get("publication_year")
        .and_then(Value::as_i64)
        .or_else(|| {
            date.as_deref()
                .and_then(|value| value.get(..4))
                .and_then(|value| value.parse().ok())
        });
    let biblio = work.get("biblio");
    let volume = biblio.and_then(|value| json_text(value.get("volume")));
    let issue_number = biblio.and_then(|value| json_text(value.get("issue")));
    let start_page = biblio.and_then(|value| json_text(value.get("first_page")));
    let end_page = biblio.and_then(|value| json_text(value.get("last_page")));
    let authors = work
        .get("authorships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.get("author"))
        .filter_map(|value| json_text(value.get("display_name")))
        .map(|display_name| ArticleAuthorDraft { display_name })
        .collect();
    canonical_article(ArticleDraft {
        catalog_id: catalog.catalog_id.clone(),
        title,
        publication_year,
        date,
        issue_title: None,
        volume,
        issue_number,
        authors,
        start_page,
        end_page,
        abstract_text: openalex_abstract(work),
        doi: json_text(work.get("doi")).and_then(|value| normalize_contract_doi(&value)),
        pmid: None,
        open_access: work
            .get("open_access")
            .and_then(|value| value.get("is_oa"))
            .and_then(Value::as_bool),
        in_press: Some(false),
        retraction_dois: Vec::new(),
    })
}

fn cnki_issue_draft(catalog: &JournalCatalogEntry, issue: &Value) -> Option<IssueDraft> {
    let publication_year = issue
        .get("year")
        .and_then(Value::as_i64)
        .or_else(|| json_text(issue.get("year"))?.parse().ok());
    let number = json_text(issue.get("number"));
    let date = publication_year.map(|year| {
        number
            .as_deref()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| (1..=12).contains(value))
            .map(|month| format!("{year:04}-{month:02}"))
            .unwrap_or_else(|| format!("{year:04}"))
    });
    Some(IssueDraft {
        catalog_id: catalog.catalog_id.clone(),
        publication_year,
        title: json_text(issue.get("title")),
        volume: json_text(issue.get("volume")),
        number,
        date,
    })
}

fn cnki_article_draft(
    catalog: &JournalCatalogEntry,
    issue: &IssueDraft,
    summary: &Value,
    detail: &Value,
) -> Option<ArticleDraft> {
    let title = json_text(detail.get("title")).or_else(|| json_text(summary.get("title")))?;
    let date = ["online_release_date", "date", "publication_date"]
        .into_iter()
        .find_map(|field| json_text(detail.get(field)))
        .or_else(|| json_text(summary.get("date")))
        .or_else(|| issue.date.clone());
    let publication_year = date
        .as_deref()
        .and_then(|value| value.get(..4))
        .and_then(|value| value.parse().ok())
        .or(issue.publication_year);
    let (start_page, end_page) = split_pages(
        json_text(detail.get("pages"))
            .or_else(|| json_text(summary.get("pages")))
            .as_deref(),
    );
    let authors = json_text(detail.get("authors"))
        .or_else(|| json_text(summary.get("authors")))
        .map(|value| split_authors(&value))
        .unwrap_or_default();
    canonical_article(ArticleDraft {
        catalog_id: catalog.catalog_id.clone(),
        title,
        publication_year,
        date,
        issue_title: issue.title.clone(),
        volume: issue.volume.clone(),
        issue_number: issue.number.clone(),
        authors,
        start_page,
        end_page,
        abstract_text: json_text(detail.get("abstract")),
        doi: json_text(detail.get("doi")).and_then(|value| normalize_contract_doi(&value)),
        pmid: json_text(detail.get("pmid")).and_then(|value| normalize_contract_pmid(&value)),
        open_access: bool_value(detail.get("open_access")),
        in_press: Some(false),
        retraction_dois: json_text(detail.get("retraction_doi"))
            .and_then(|value| normalize_contract_doi(&value))
            .into_iter()
            .collect(),
    })
}

fn canonical_article(mut article: ArticleDraft) -> Option<ArticleDraft> {
    article.title = normalize_contract_text(&article.title)?;
    article.date = article
        .date
        .as_deref()
        .and_then(normalize_contract_date)
        .map(|date| date.value);
    article.issue_title = canonical_optional_text(article.issue_title);
    article.volume = canonical_optional_text(article.volume);
    article.issue_number = canonical_optional_text(article.issue_number);
    article.start_page = canonical_optional_text(article.start_page);
    article.end_page = canonical_optional_text(article.end_page);
    article.abstract_text = canonical_optional_text(article.abstract_text);
    article.authors = article
        .authors
        .into_iter()
        .filter_map(|author| {
            normalize_contract_text(&author.display_name)
                .map(|display_name| ArticleAuthorDraft { display_name })
        })
        .collect();
    article.retraction_dois = article
        .retraction_dois
        .into_iter()
        .filter_map(|value| normalize_contract_doi(&value))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let has_external_identifier = article.doi.is_some() || article.pmid.is_some();
    let has_bibliographic_identity = article.publication_year.is_some()
        && (article.volume.is_some()
            || article.issue_number.is_some()
            || article.start_page.is_some());
    (has_external_identifier || has_bibliographic_identity).then_some(article)
}

fn canonical_optional_text(value: Option<String>) -> Option<String> {
    value.as_deref().and_then(normalize_contract_text)
}

fn catalog_issns(catalog: &JournalCatalogEntry) -> Vec<String> {
    let mut values = catalog.all_issns.clone();
    for value in [catalog.issn.as_ref(), catalog.eissn.as_ref()]
        .into_iter()
        .flatten()
    {
        if !values.contains(value) {
            values.push(value.clone());
        }
    }
    values
}

fn crossref_date(work: &Value) -> Option<String> {
    for key in ["published-online", "published-print", "published", "issued"] {
        let Some(parts) = work
            .get(key)
            .and_then(|value| value.get("date-parts"))
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_array)
        else {
            continue;
        };
        if let Some(year) = parts.first().and_then(Value::as_i64) {
            let month = parts.get(1).and_then(Value::as_i64);
            let day = parts.get(2).and_then(Value::as_i64);
            let candidate = match (month, day) {
                (Some(month), Some(day)) => format!("{year:04}-{month:02}-{day:02}"),
                (Some(month), None) => format!("{year:04}-{month:02}"),
                (None, _) => format!("{year:04}"),
            };
            return normalize_contract_date(&candidate).map(|date| date.value);
        }
    }
    None
}

fn crossref_authors(value: Option<&Value>) -> Vec<ArticleAuthorDraft> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|author| {
            let name = [
                json_text(author.get("given")),
                json_text(author.get("family")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            normalize_contract_text(&name).map(|display_name| ArticleAuthorDraft { display_name })
        })
        .collect()
}

fn split_authors(value: &str) -> Vec<ArticleAuthorDraft> {
    value
        .split([';', '；', ','])
        .filter_map(normalize_contract_text)
        .map(|display_name| ArticleAuthorDraft { display_name })
        .collect()
}

fn split_pages(value: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(value) = value.and_then(normalize_contract_text) else {
        return (None, None);
    };
    for separator in ['-', '–', '—'] {
        if let Some((start, end)) = value.split_once(separator) {
            return (normalize_contract_text(start), normalize_contract_text(end));
        }
    }
    (Some(value), None)
}

fn first_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(values) = value.as_array() {
        return values.iter().find_map(|value| json_text(Some(value)));
    }
    json_text(Some(value))
}

fn json_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => normalize_contract_text(value),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn strip_markup(value: &str) -> Option<String> {
    let mut output = String::with_capacity(value.len());
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    normalize_contract_text(&output)
}

fn openalex_abstract(value: &Value) -> Option<String> {
    let object = value.get("abstract_inverted_index")?.as_object()?;
    let mut positions = Vec::new();
    for (word, indexes) in object {
        for index in indexes.as_array()? {
            positions.push((index.as_i64()?, word.clone()));
        }
    }
    positions.sort_by_key(|(index, _)| *index);
    normalize_contract_text(
        &positions
            .into_iter()
            .map(|(_, word)| word)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn updated_by_retraction_dois(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            json_text(item.get("type"))
                .is_some_and(|update_type| update_type.eq_ignore_ascii_case("retraction"))
        })
        .filter_map(|item| {
            json_text(item.get("DOI")).and_then(|value| normalize_contract_doi(&value))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn bool_value(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => Some(value.as_i64()? != 0),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn scholarly_article_redirect(article: &ArticleLocator) -> Result<ArticleRedirect, ProviderError> {
    if let Some(doi) = article.doi.as_deref() {
        return Ok(ArticleRedirect {
            location: format!("https://doi.org/{}", encode_doi_path(doi)),
        });
    }
    if let Some(pmid) = article.pmid.as_deref() {
        return Ok(ArticleRedirect {
            location: format!("https://pubmed.ncbi.nlm.nih.gov/{pmid}/"),
        });
    }
    Err(ProviderError::new(
        ProviderErrorKind::NotFound,
        "scholarly provider requires a DOI or PubMed identifier",
    ))
}

fn encode_doi_path(doi: &str) -> String {
    let mut encoded = String::new();
    for byte in doi.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn resolve_cnki_article_redirect<T>(
    client: &mut CnkiClient<T>,
    article: &ArticleLocator,
) -> Result<ArticleRedirect, ProviderError>
where
    T: CnkiTransport,
{
    let row = BTreeMap::from([
        ("title".to_string(), article.journal_title.clone()),
        (
            "issn".to_string(),
            article.journal_issns.first().cloned().unwrap_or_default(),
        ),
    ]);
    let journal = client
        .resolve_journal(&row)
        .map_err(map_cnki_error)?
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::NotFound,
                "CNKI provider could not resolve the journal",
            )
        })?;
    let issue_payloads = client.year_issues(&journal).map_err(map_cnki_error)?;
    for issue_payload in issue_payloads {
        if !cnki_issue_matches_locator(&issue_payload, article) {
            continue;
        }
        for summary in client
            .issue_articles(&journal, &issue_payload)
            .map_err(map_cnki_error)?
        {
            let Some(summary_title) = json_text(summary.get("title")) else {
                continue;
            };
            if normalize_bibliographic_text(&summary_title)
                != normalize_bibliographic_text(&article.title)
            {
                continue;
            }
            let Some(article_url) = json_text(summary.get("article_url")) else {
                continue;
            };
            let platform_id = json_text(summary.get("platform_id"));
            let detail = client
                .article_detail(&article_url, platform_id.as_deref())
                .map_err(map_cnki_error)?;
            if !cnki_detail_matches_locator(&detail, article) {
                continue;
            }
            let location = json_text(detail.get("permalink")).ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "CNKI detail response omitted its request-time destination",
                )
            })?;
            return Ok(ArticleRedirect { location });
        }
    }
    Err(ProviderError::new(
        ProviderErrorKind::NotFound,
        "CNKI provider could not find an exact article match",
    ))
}

fn cnki_issue_matches_locator(issue: &Value, article: &ArticleLocator) -> bool {
    let issue_year = json_text(issue.get("year")).and_then(|value| value.parse().ok());
    if article.publication_year.is_some() && issue_year != article.publication_year {
        return false;
    }
    let issue_number = json_text(issue.get("number"));
    if let (Some(expected), Some(observed)) =
        (article.issue_number.as_deref(), issue_number.as_deref())
    {
        if normalize_bibliographic_label(expected) != normalize_bibliographic_label(observed) {
            return false;
        }
    }
    true
}

fn cnki_detail_matches_locator(detail: &Value, article: &ArticleLocator) -> bool {
    let Some(title) = json_text(detail.get("title")) else {
        return false;
    };
    if normalize_bibliographic_text(&title) != normalize_bibliographic_text(&article.title) {
        return false;
    }
    let detail_doi = json_text(detail.get("doi")).and_then(|value| normalize_contract_doi(&value));
    !matches!(
        (article.doi.as_deref(), detail_doi.as_deref()),
        (Some(expected), Some(observed)) if expected != observed
    )
}

fn map_scholarly_error(error: SourceError) -> ProviderError {
    let kind = match error {
        SourceError::HttpStatus {
            status_code: 404, ..
        } => ProviderErrorKind::NotFound,
        SourceError::HttpStatus { .. } | SourceError::Request { .. } => {
            ProviderErrorKind::TemporarilyUnavailable
        }
        SourceError::InvalidFixture(_) => ProviderErrorKind::InvalidResponse,
        SourceError::Configuration(_) => ProviderErrorKind::Internal,
    };
    ProviderError::new(kind, "scholarly provider request failed")
}

fn domestic_journal_locator_from_catalog(catalog: &JournalCatalogEntry) -> DomesticJournalLocator {
    let mut titles = vec![catalog.title.clone()];
    titles.extend(catalog.title_aliases.iter().cloned());
    let mut issns = catalog
        .issn
        .iter()
        .chain(catalog.eissn.iter())
        .cloned()
        .collect::<Vec<_>>();
    issns.extend(catalog.all_issns.iter().cloned());
    DomesticJournalLocator::new(titles, issns)
}

fn domestic_journal_locator_from_article(article: &ArticleLocator) -> DomesticJournalLocator {
    DomesticJournalLocator::new(
        vec![article.journal_title.clone()],
        article.journal_issns.clone(),
    )
}

fn domestic_cnki_lacks_authors_and_doi(summary: &Value, detail: &Value) -> bool {
    let has_authors = json_text(detail.get("authors"))
        .or_else(|| json_text(summary.get("authors")))
        .is_some_and(|value| !split_authors(&value).is_empty());
    let has_doi = json_text(detail.get("doi"))
        .and_then(|value| normalize_contract_doi(&value))
        .is_some();
    !has_authors && !has_doi
}

#[derive(Clone)]
struct DomesticCnkiDetailTask {
    article_index: usize,
    summary: Value,
    article_url: String,
    platform_id: Option<String>,
}

struct DomesticCnkiDetailOutcome {
    article_index: usize,
    summary: Value,
    detail: Result<Value, DomesticCnkiSourceError>,
    attempts: Vec<SourceAttempt>,
}

struct DomesticCnkiDetailJob {
    task: DomesticCnkiDetailTask,
    result_sender: Sender<Result<DomesticCnkiDetailOutcome, ProviderError>>,
}

struct DomesticCnkiDetailPool {
    sender: Option<SyncSender<DomesticCnkiDetailJob>>,
    workers: Vec<JoinHandle<()>>,
    worker_count: usize,
    active_requests: Arc<AtomicUsize>,
    peak_requests: Arc<AtomicUsize>,
}

impl DomesticCnkiDetailPool {
    fn new<T>(
        client: &DomesticCnkiClient<T>,
        worker_count: usize,
    ) -> Result<Self, ProviderRegistryError>
    where
        T: DomesticCnkiTransport + Clone + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(worker_count);
        let receiver = Arc::new(Mutex::new(receiver));
        let active_requests = Arc::new(AtomicUsize::new(0));
        let peak_requests = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::with_capacity(worker_count);
        for worker_id in 0..worker_count {
            let worker_client = client.clone();
            let worker_receiver = Arc::clone(&receiver);
            let worker_active_requests = Arc::clone(&active_requests);
            let worker_peak_requests = Arc::clone(&peak_requests);
            match thread::Builder::new()
                .name(format!("litradar-cnki-detail-{worker_id}"))
                .spawn(move || {
                    run_domestic_cnki_detail_worker(
                        worker_client,
                        worker_receiver,
                        worker_active_requests,
                        worker_peak_requests,
                    );
                }) {
                Ok(worker) => workers.push(worker),
                Err(_) => {
                    drop(sender);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(ProviderRegistryError::InvalidConfiguration {
                        provider: CNKI_PROVIDER_NAME.to_string(),
                        detail: "domestic CNKI detail worker could not start".to_string(),
                    });
                }
            }
        }
        Ok(Self {
            sender: Some(sender),
            workers,
            worker_count,
            active_requests,
            peak_requests,
        })
    }

    fn execute(
        &self,
        tasks: Vec<DomesticCnkiDetailTask>,
    ) -> Result<Vec<DomesticCnkiDetailOutcome>, ProviderError> {
        if tasks.is_empty() {
            return Ok(Vec::new());
        }
        let task_count = tasks.len();
        let (result_sender, result_receiver) = mpsc::channel();
        let sender = self.sender.as_ref().ok_or_else(detail_pool_unavailable)?;
        for task in tasks {
            sender
                .send(DomesticCnkiDetailJob {
                    task,
                    result_sender: result_sender.clone(),
                })
                .map_err(|_| detail_pool_unavailable())?;
        }
        drop(result_sender);
        let mut outcomes = Vec::with_capacity(task_count);
        let mut first_error = None;
        for _ in 0..task_count {
            match result_receiver.recv() {
                Ok(Ok(outcome)) => outcomes.push(outcome),
                Ok(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(_) => {
                    if first_error.is_none() {
                        first_error = Some(detail_pool_unavailable());
                    }
                }
            }
        }
        let peak_requests = self.peak_requests.load(Ordering::SeqCst);
        let active_requests = self.active_requests.load(Ordering::SeqCst);
        tracing::info!(
            event = "index.provider.concurrency",
            component = "index",
            provider = CNKI_PROVIDER_NAME,
            configured_workers = self.worker_count,
            effective_workers = self.worker_count.min(task_count),
            worker_threads_created = self.workers.len(),
            active_detail_requests = active_requests,
            peak_detail_requests = peak_requests,
            aggregate_limit = litradar_domain::INDEX_AGGREGATE_CONCURRENCY_MAX,
        );
        if let Some(error) = first_error {
            return Err(error);
        }
        outcomes.sort_by_key(|outcome| outcome.article_index);
        Ok(outcomes)
    }
}

impl Drop for DomesticCnkiDetailPool {
    fn drop(&mut self) {
        drop(self.sender.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct ActiveDomesticCnkiDetail {
    active_requests: Arc<AtomicUsize>,
}

impl ActiveDomesticCnkiDetail {
    fn start(active_requests: Arc<AtomicUsize>, peak_requests: &AtomicUsize) -> Self {
        let active = active_requests.fetch_add(1, Ordering::SeqCst) + 1;
        peak_requests.fetch_max(active, Ordering::SeqCst);
        Self { active_requests }
    }
}

impl Drop for ActiveDomesticCnkiDetail {
    fn drop(&mut self) {
        self.active_requests.fetch_sub(1, Ordering::SeqCst);
    }
}

fn run_domestic_cnki_detail_worker<T>(
    mut client: DomesticCnkiClient<T>,
    receiver: Arc<Mutex<Receiver<DomesticCnkiDetailJob>>>,
    active_requests: Arc<AtomicUsize>,
    peak_requests: Arc<AtomicUsize>,
) where
    T: DomesticCnkiTransport,
{
    loop {
        let job = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(job) = job else {
            return;
        };
        client.drain_attempts();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _active_detail = ActiveDomesticCnkiDetail::start(
                Arc::clone(&active_requests),
                peak_requests.as_ref(),
            );
            let detail =
                client.article_detail(&job.task.article_url, job.task.platform_id.as_deref());
            DomesticCnkiDetailOutcome {
                article_index: job.task.article_index,
                summary: job.task.summary,
                detail,
                attempts: client.drain_attempts(),
            }
        }))
        .map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "domestic CNKI detail worker panicked",
            )
        });
        let _ = job.result_sender.send(result);
    }
}

fn detail_pool_unavailable() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Internal,
        "domestic CNKI detail worker pool is unavailable",
    )
}

fn fetch_domestic_cnki_batch<T>(
    client: &mut DomesticCnkiClient<T>,
    journal_snapshots: &mut BTreeMap<String, DomesticCnkiJournalSnapshot>,
    catalog: &JournalCatalogEntry,
    context: IndexFetchContext<'_>,
    detail_pool: &DomesticCnkiDetailPool,
    detail_attempts: &mut Vec<SourceAttempt>,
) -> Result<ProviderBatch, ProviderError>
where
    T: DomesticCnkiTransport + Clone + Send,
{
    let committed_anchor = context
        .committed_anchor
        .map(decode_domestic_anchor)
        .transpose()?;
    let committed_issue_id = committed_anchor
        .as_ref()
        .map(|anchor| anchor.year_issue_id.clone());
    let resume = context
        .traversal_checkpoint
        .map(decode_domestic_checkpoint)
        .transpose()?;
    if let Some(resume) = &resume {
        if resume.version != crate::DOMESTIC_CNKI_CHECKPOINT_VERSION
            || resume
                .base_anchor_issue_id
                .as_deref()
                .is_some_and(|value| !is_stable_domestic_issue_id(value))
            || !is_stable_domestic_issue_id(&resume.candidate_head_issue_id)
            || !is_stable_domestic_issue_id(&resume.current_issue_id)
        {
            return Err(ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "domestic CNKI checkpoint version or issue id is invalid",
            ));
        }
        if resume.base_anchor_issue_id != committed_issue_id {
            return Err(ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "domestic CNKI checkpoint does not match the frozen committed anchor",
            ));
        }
    }

    if !journal_snapshots.contains_key(&catalog.catalog_id) {
        let locator = domestic_journal_locator_from_catalog(catalog);
        let journal = client
            .resolve_journal(&locator)
            .map_err(map_domestic_cnki_error)?
            .ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::NotFound,
                    "domestic CNKI provider could not resolve the journal",
                )
            })?;
        let issue_payloads = client
            .year_issues(&journal)
            .map_err(map_domestic_cnki_error)?;
        journal_snapshots.insert(
            catalog.catalog_id.clone(),
            DomesticCnkiJournalSnapshot {
                journal,
                issue_payloads,
            },
        );
    }
    let snapshot = journal_snapshots
        .get(&catalog.catalog_id)
        .expect("domestic CNKI snapshot inserted above");
    let journal = &snapshot.journal;
    let issue_payloads = &snapshot.issue_payloads;
    if issue_payloads.is_empty() {
        if resume.is_some() {
            return Err(missing_domestic_checkpoint_issue_error());
        }
        return Ok(ProviderBatch {
            catalog_id: catalog.catalog_id.clone(),
            journal: journal_observation(catalog),
            issues: Vec::new(),
            articles: Vec::new(),
            progress: ProviderProgress::Complete { next_anchor: None },
        });
    }
    let issue_ids = issue_payloads
        .iter()
        .map(|issue| {
            domestic_year_issue_id(issue).ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "domestic CNKI issue payload omitted its stable year_issue_id",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if issue_ids.iter().collect::<BTreeSet<_>>().len() != issue_ids.len() {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI issue tree contains duplicate stable issue ids",
        ));
    }
    let candidate_head_issue_id = resume.as_ref().map_or_else(
        || issue_ids[0].clone(),
        |resume| resume.candidate_head_issue_id.clone(),
    );
    let candidate_head_index = issue_ids
        .iter()
        .position(|issue_id| issue_id == &candidate_head_issue_id)
        .ok_or_else(missing_domestic_checkpoint_issue_error)?;
    let window_end_index = if context.mode == IndexSyncMode::Incremental {
        committed_issue_id
            .as_ref()
            .and_then(|base_issue_id| {
                issue_ids
                    .iter()
                    .enumerate()
                    .skip(candidate_head_index)
                    .find_map(|(index, issue_id)| (issue_id == base_issue_id).then_some(index))
            })
            .unwrap_or(issue_payloads.len() - 1)
    } else {
        issue_payloads.len() - 1
    };
    let (issue_index, page_index) = if let Some(resume) = &resume {
        let issue_index = issue_ids
            .iter()
            .position(|issue_id| issue_id == &resume.current_issue_id)
            .filter(|index| *index >= candidate_head_index && *index <= window_end_index)
            .ok_or_else(missing_domestic_checkpoint_issue_error)?;
        (issue_index, resume.page_index)
    } else {
        (candidate_head_index, 0)
    };
    let issue_payload = &issue_payloads[issue_index];
    let current_issue_id = issue_ids[issue_index].clone();
    if issue_payload.get("year").and_then(Value::as_i64).is_none() {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI issue payload omitted its publication year",
        ));
    }
    let issue = cnki_issue_draft(catalog, issue_payload).ok_or_else(|| {
        ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI issue payload could not be converted",
        )
    })?;

    let page = client
        .issue_articles(journal, issue_payload, page_index)
        .map_err(map_domestic_cnki_error)?;
    validate_domestic_issue_page(&page, page_index)?;
    let has_next_page = page.has_next_page;
    let detail_tasks = page
        .articles
        .into_iter()
        .enumerate()
        .map(|(article_index, summary)| {
            let article_url = json_text(summary.get("article_url")).ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "domestic CNKI article summary omitted its URL",
                )
            })?;
            Ok(DomesticCnkiDetailTask {
                article_index,
                platform_id: json_text(summary.get("platform_id")),
                summary,
                article_url,
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    let mut detail_outcomes = detail_pool.execute(detail_tasks)?;
    for outcome in &mut detail_outcomes {
        detail_attempts.append(&mut outcome.attempts);
    }
    let mut articles = Vec::new();
    for outcome in detail_outcomes {
        let article_index = outcome.article_index;
        let summary = outcome.summary;
        let detail = match outcome.detail {
            Ok(detail) => detail,
            Err(error) if is_permanent_domestic_article_error(&error) => {
                tracing::warn!(
                    event = "index.provider.article.skipped",
                    component = "index",
                    provider = CNKI_PROVIDER_NAME,
                    reason = "permanent_missing",
                    article_ordinal = article_index + 1,
                    http_status = error.http_status().unwrap_or(0),
                    has_http_status = error.http_status().is_some(),
                );
                continue;
            }
            Err(error) => return Err(map_domestic_cnki_error(error)),
        };
        if domestic_cnki_lacks_authors_and_doi(&summary, &detail) {
            tracing::info!(
                event = "index.provider.article.skipped",
                component = "index",
                provider = CNKI_PROVIDER_NAME,
                reason = "missing_authors_and_doi",
                article_ordinal = article_index + 1,
            );
            continue;
        }
        let article = cnki_article_draft(catalog, &issue, &summary, &detail).ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "domestic CNKI article payload could not be converted",
            )
        })?;
        articles.push(article);
    }
    let next = if has_next_page {
        Some(DomesticCnkiCheckpoint {
            version: crate::DOMESTIC_CNKI_CHECKPOINT_VERSION,
            base_anchor_issue_id: committed_issue_id.clone(),
            candidate_head_issue_id: candidate_head_issue_id.clone(),
            current_issue_id,
            page_index: page_index.checked_add(1).ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "domestic CNKI page index overflowed",
                )
            })?,
        })
    } else if issue_index < window_end_index {
        Some(DomesticCnkiCheckpoint {
            version: crate::DOMESTIC_CNKI_CHECKPOINT_VERSION,
            base_anchor_issue_id: committed_issue_id,
            candidate_head_issue_id: candidate_head_issue_id.clone(),
            current_issue_id: issue_ids[issue_index + 1].clone(),
            page_index: 0,
        })
    } else {
        None
    };
    let progress = match next {
        Some(checkpoint) => ProviderProgress::Continue {
            checkpoint: encode_domestic_checkpoint(&checkpoint)?,
        },
        None => ProviderProgress::Complete {
            next_anchor: Some(encode_domestic_anchor(&DomesticCnkiAnchor {
                version: DOMESTIC_CNKI_ANCHOR_VERSION,
                year_issue_id: candidate_head_issue_id,
            })?),
        },
    };
    Ok(ProviderBatch {
        catalog_id: catalog.catalog_id.clone(),
        journal: journal_observation(catalog),
        issues: vec![issue],
        articles,
        progress,
    })
}

fn encode_domestic_checkpoint(
    checkpoint: &DomesticCnkiCheckpoint,
) -> Result<String, ProviderError> {
    serde_json::to_string(checkpoint).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::Internal,
            "domestic CNKI checkpoint could not be encoded",
        )
    })
}

fn decode_domestic_checkpoint(raw: &str) -> Result<DomesticCnkiCheckpoint, ProviderError> {
    reject_domestic_opaque_secrets(raw, "checkpoint")?;
    serde_json::from_str(raw).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI checkpoint is invalid",
        )
    })
}

fn encode_domestic_anchor(anchor: &DomesticCnkiAnchor) -> Result<String, ProviderError> {
    serde_json::to_string(anchor).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::Internal,
            "domestic CNKI anchor could not be encoded",
        )
    })
}

fn decode_domestic_anchor(raw: &str) -> Result<DomesticCnkiAnchor, ProviderError> {
    reject_domestic_opaque_secrets(raw, "anchor")?;
    let anchor = serde_json::from_str::<DomesticCnkiAnchor>(raw).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI anchor is invalid",
        )
    })?;
    if anchor.version != DOMESTIC_CNKI_ANCHOR_VERSION
        || !is_stable_domestic_issue_id(&anchor.year_issue_id)
    {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI anchor version or issue id is invalid",
        ));
    }
    Ok(anchor)
}

fn reject_domestic_opaque_secrets(raw: &str, state_kind: &str) -> Result<(), ProviderError> {
    let lowered = raw.to_ascii_lowercase();
    if [
        "captcha",
        "secretkey",
        "pointjson",
        "jfbym",
        "session",
        "cookie",
        "token",
        "http://",
        "https://",
        "url",
    ]
    .iter()
    .any(|forbidden| lowered.contains(forbidden))
    {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            format!("domestic CNKI {state_kind} must not contain session or transport fields"),
        ));
    }
    Ok(())
}

fn domestic_year_issue_id(issue: &Value) -> Option<String> {
    json_text(issue.get("year_issue_id")).filter(|value| is_stable_domestic_issue_id(value))
}

fn is_stable_domestic_issue_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn missing_domestic_checkpoint_issue_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::InvalidResponse,
        "domestic CNKI checkpoint issue is missing; reset the disposable control database",
    )
}

fn is_permanent_domestic_article_error(error: &DomesticCnkiSourceError) -> bool {
    matches!(error, DomesticCnkiSourceError::PermanentArticleMissing)
        || matches!(error.http_status(), Some(404 | 410))
}

fn validate_domestic_issue_page(
    page: &DomesticIssueArticlePage,
    expected_page_index: usize,
) -> Result<(), ProviderError> {
    if page.page_index != expected_page_index || page.article_count != page.articles.len() {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "domestic CNKI issue article page metadata is inconsistent",
        ));
    }
    Ok(())
}

fn resolve_domestic_cnki_article_redirect<T>(
    client: &mut DomesticCnkiClient<T>,
    article: &ArticleLocator,
) -> Result<ArticleRedirect, ProviderError>
where
    T: DomesticCnkiTransport,
{
    let locator = domestic_journal_locator_from_article(article);
    let journal = client
        .resolve_journal(&locator)
        .map_err(map_domestic_cnki_error)?
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::NotFound,
                "domestic CNKI provider could not resolve the journal",
            )
        })?;
    let issue_payloads = client
        .year_issues(&journal)
        .map_err(map_domestic_cnki_error)?;
    for issue_payload in issue_payloads {
        if !cnki_issue_matches_locator(&issue_payload, article) {
            continue;
        }
        let mut page_index = 0;
        loop {
            let page = client
                .issue_articles(&journal, &issue_payload, page_index)
                .map_err(map_domestic_cnki_error)?;
            validate_domestic_issue_page(&page, page_index)?;
            let has_next_page = page.has_next_page;
            for (article_index, summary) in page.articles.into_iter().enumerate() {
                let Some(summary_title) = json_text(summary.get("title")) else {
                    continue;
                };
                if normalize_bibliographic_text(&summary_title)
                    != normalize_bibliographic_text(&article.title)
                {
                    continue;
                }
                let article_url = json_text(summary.get("article_url")).ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::InvalidResponse,
                        "domestic CNKI matching article summary omitted its URL",
                    )
                })?;
                let platform_id = json_text(summary.get("platform_id"));
                let detail = match client.article_detail(&article_url, platform_id.as_deref()) {
                    Ok(detail) => detail,
                    Err(error) if is_permanent_domestic_article_error(&error) => {
                        tracing::warn!(
                            event = "index.provider.article.skipped",
                            component = "index",
                            provider = CNKI_PROVIDER_NAME,
                            reason = "permanent_missing",
                            article_ordinal = article_index + 1,
                            http_status = error.http_status().unwrap_or(0),
                            has_http_status = error.http_status().is_some(),
                        );
                        continue;
                    }
                    Err(error) => return Err(map_domestic_cnki_error(error)),
                };
                if !cnki_detail_matches_locator(&detail, article) {
                    continue;
                }
                let location = json_text(detail.get("permalink"))
                    .or_else(|| json_text(detail.get("article_url")))
                    .ok_or_else(|| {
                        ProviderError::new(
                            ProviderErrorKind::InvalidResponse,
                            "domestic CNKI detail response omitted its request-time destination",
                        )
                    })?;
                if !(location.starts_with("https://navi.cnki.net/")
                    || location.starts_with("https://kns.cnki.net/")
                    || location.starts_with("https://www.cnki.net/"))
                {
                    return Err(ProviderError::new(
                        ProviderErrorKind::InvalidResponse,
                        "domestic CNKI abstract destination is outside the allowlist",
                    ));
                }
                if location.to_ascii_lowercase().contains("oversea.cnki.net") {
                    return Err(ProviderError::new(
                        ProviderErrorKind::InvalidResponse,
                        "domestic CNKI abstract destination used overseas host",
                    ));
                }
                return Ok(ArticleRedirect { location });
            }
            if !has_next_page {
                break;
            }
            page_index = page_index.checked_add(1).ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "domestic CNKI abstract page index overflowed",
                )
            })?;
        }
    }
    Err(ProviderError::new(
        ProviderErrorKind::NotFound,
        "domestic CNKI provider could not find an exact article match",
    ))
}

fn map_domestic_cnki_error(error: DomesticCnkiSourceError) -> ProviderError {
    let kind = match error {
        DomesticCnkiSourceError::Request(_) | DomesticCnkiSourceError::Source(_) => {
            ProviderErrorKind::TemporarilyUnavailable
        }
        DomesticCnkiSourceError::Parse(_) | DomesticCnkiSourceError::MissingFixture(_) => {
            ProviderErrorKind::InvalidResponse
        }
        DomesticCnkiSourceError::PermanentArticleMissing => ProviderErrorKind::NotFound,
    };
    ProviderError::new(kind, "domestic CNKI provider request failed")
}
fn map_cnki_error(error: CnkiSourceError) -> ProviderError {
    let kind = match error {
        CnkiSourceError::Request(_) | CnkiSourceError::Source(_) => {
            ProviderErrorKind::TemporarilyUnavailable
        }
        CnkiSourceError::Parse(_) | CnkiSourceError::MissingFixture(_) => {
            ProviderErrorKind::InvalidResponse
        }
    };
    ProviderError::new(kind, "CNKI provider request failed")
}

fn emit_source_attempt_summary(provider: &str, attempts: &[SourceAttempt]) {
    let failures = attempts
        .iter()
        .filter(|attempt| !attempt.did_succeed)
        .count();
    let retries = attempts.iter().filter(|attempt| attempt.did_retry).count();
    tracing::info!(
        event = "index.provider.attempts",
        component = "index",
        provider,
        attempt_count = attempts.len(),
        failure_count = failures,
        retry_count = retries,
    );
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use litradar_domain::{
        ArticleAccessContext, ArticleId, ArticleLocator, IndexFetchContext, IndexSyncMode,
        JournalCatalogEntry, JournalRankings, ProviderBatch, ProviderCapabilityKind,
        ProviderProgress, DOMESTIC_CNKI_WORKER_COUNT_MAX,
    };
    use litradar_provider::{
        IndexContentProvider, ProviderError, ProviderErrorKind, ProviderRegistry,
        ProviderRegistryError,
    };
    use serde_json::{json, Value};

    use super::{
        built_in_provider_capabilities, cnki_access_registration, cnki_article_draft,
        cnki_index_registration, cnki_index_registration_with_workers, cnki_issue_draft,
        cnki_oversea_access_registration, cnki_oversea_index_registration,
        crossref_cursor_is_fresh, crossref_work_issue_anchor, decode_scholarly_anchor,
        decode_scholarly_checkpoint, encode_scholarly_anchor, encode_scholarly_checkpoint,
        fetch_scholarly_batch_for_context_with_clock_and_restart, fetch_scholarly_batch_with_clock,
        fetch_scholarly_batch_with_clock_and_restart, next_scholarly_source,
        normalize_empty_unbounded_replay_checkpoint_at, openalex_article_draft,
        scholarly_access_registration, scholarly_article_draft, scholarly_index_registration,
        scholarly_issue_anchor, scholarly_window_filter_at, scholarly_window_from_context,
        CnkiIndexProvider, DomesticCnkiIndexProvider, ScholarlyAnchor, ScholarlyCheckpoint,
        ScholarlyIndexProvider, ScholarlyIssueFingerprint, ScholarlyScanPhase,
        ScholarlySourceCheckpoint, ScholarlyWindowCheckpoint, CNKI_PROVIDER_NAME,
        CNKI_REDIRECT_HOSTS, CROSSREF_CURSOR_REUSE_SECONDS, DOMESTIC_CNKI_REDIRECT_HOSTS,
        SCHOLARLY_ANCHOR_VERSION, SCHOLARLY_CHECKPOINT_VERSION, SCHOLARLY_REDIRECT_HOSTS,
    };
    use crate::scholarly::test_support::CapturedLogs;
    use crate::{
        CnkiFixtureData, DomesticCnkiFixtureData, DomesticCnkiSourceError, DomesticCnkiTransport,
        DomesticIssueArticlePage, DomesticJournalLocator, FixtureCnkiTransport,
        FixtureDomesticCnkiTransport, FixtureScholarlyTransport, ScholarlyClient,
        ScholarlyFixtureData, ScholarlyRequest, ScholarlyRequestKind, ScholarlyTransport,
        SourceAttempt, SourceError,
    };

    struct ConcurrentDomesticTransport {
        inner: FixtureDomesticCnkiTransport,
        active_requests: Arc<AtomicUsize>,
        peak_requests: Arc<AtomicUsize>,
        clone_count: Arc<AtomicUsize>,
        drop_count: Arc<AtomicUsize>,
    }

    impl Clone for ConcurrentDomesticTransport {
        fn clone(&self) -> Self {
            self.clone_count.fetch_add(1, Ordering::SeqCst);
            Self {
                inner: self.inner.clone(),
                active_requests: Arc::clone(&self.active_requests),
                peak_requests: Arc::clone(&self.peak_requests),
                clone_count: Arc::clone(&self.clone_count),
                drop_count: Arc::clone(&self.drop_count),
            }
        }
    }

    impl Drop for ConcurrentDomesticTransport {
        fn drop(&mut self) {
            self.drop_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl DomesticCnkiTransport for ConcurrentDomesticTransport {
        fn resolve_journal(
            &mut self,
            locator: &DomesticJournalLocator,
        ) -> Result<Option<Value>, DomesticCnkiSourceError> {
            self.inner.resolve_journal(locator)
        }

        fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError> {
            self.inner.year_issues(journal)
        }

        fn issue_articles(
            &mut self,
            journal: &Value,
            issue: &Value,
            page_index: usize,
        ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
            self.inner.issue_articles(journal, issue, page_index)
        }

        fn article_detail(
            &mut self,
            article_url: &str,
            platform_id: Option<&str>,
        ) -> Result<Value, DomesticCnkiSourceError> {
            let active = self.active_requests.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_requests.fetch_max(active, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(30));
            let result = self.inner.article_detail(article_url, platform_id);
            self.active_requests.fetch_sub(1, Ordering::SeqCst);
            result
        }

        fn attempts(&self) -> &[SourceAttempt] {
            self.inner.attempts()
        }

        fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
            self.inner.drain_attempts()
        }
    }

    #[derive(Clone)]
    struct MetadataCountingDomesticTransport {
        inner: FixtureDomesticCnkiTransport,
        journal_resolution_count: Arc<AtomicUsize>,
        issue_tree_count: Arc<AtomicUsize>,
    }

    impl DomesticCnkiTransport for MetadataCountingDomesticTransport {
        fn resolve_journal(
            &mut self,
            locator: &DomesticJournalLocator,
        ) -> Result<Option<Value>, DomesticCnkiSourceError> {
            self.journal_resolution_count.fetch_add(1, Ordering::SeqCst);
            self.inner.resolve_journal(locator)
        }

        fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError> {
            self.issue_tree_count.fetch_add(1, Ordering::SeqCst);
            self.inner.year_issues(journal)
        }

        fn issue_articles(
            &mut self,
            journal: &Value,
            issue: &Value,
            page_index: usize,
        ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
            self.inner.issue_articles(journal, issue, page_index)
        }

        fn article_detail(
            &mut self,
            article_url: &str,
            platform_id: Option<&str>,
        ) -> Result<Value, DomesticCnkiSourceError> {
            self.inner.article_detail(article_url, platform_id)
        }

        fn attempts(&self) -> &[SourceAttempt] {
            self.inner.attempts()
        }

        fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
            self.inner.drain_attempts()
        }
    }

    #[derive(Clone)]
    struct BatchRecoveryDomesticTransport {
        inner: FixtureDomesticCnkiTransport,
        is_session_stale: bool,
        session_reset_count: Arc<AtomicUsize>,
        detail_attempt_count: Arc<AtomicUsize>,
    }

    impl DomesticCnkiTransport for BatchRecoveryDomesticTransport {
        fn reset_transient_state(&mut self) -> Result<(), DomesticCnkiSourceError> {
            self.is_session_stale = false;
            self.session_reset_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn resolve_journal(
            &mut self,
            locator: &DomesticJournalLocator,
        ) -> Result<Option<Value>, DomesticCnkiSourceError> {
            self.inner.resolve_journal(locator)
        }

        fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError> {
            self.inner.year_issues(journal)
        }

        fn issue_articles(
            &mut self,
            journal: &Value,
            issue: &Value,
            page_index: usize,
        ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
            self.inner.issue_articles(journal, issue, page_index)
        }

        fn article_detail(
            &mut self,
            article_url: &str,
            platform_id: Option<&str>,
        ) -> Result<Value, DomesticCnkiSourceError> {
            self.detail_attempt_count.fetch_add(1, Ordering::SeqCst);
            if self.is_session_stale {
                return Err(DomesticCnkiSourceError::Parse(
                    "temporary article detail response".to_string(),
                ));
            }
            self.inner.article_detail(article_url, platform_id)
        }

        fn attempts(&self) -> &[SourceAttempt] {
            self.inner.attempts()
        }

        fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
            self.inner.drain_attempts()
        }
    }

    #[derive(Debug, Clone)]
    enum CrossrefFixtureResponse {
        Page {
            items: Vec<serde_json::Value>,
            next_cursor: Option<String>,
        },
        HttpStatus(u16),
        RequestFailure,
    }

    #[derive(Debug)]
    struct CursorRecoveryTransport {
        responses: VecDeque<CrossrefFixtureResponse>,
        attempts: Vec<SourceAttempt>,
        requested_cursors: Vec<Option<String>>,
        requested_sync_dates: Vec<Option<String>>,
    }

    impl CursorRecoveryTransport {
        fn new(responses: Vec<CrossrefFixtureResponse>) -> Self {
            Self {
                responses: responses.into(),
                attempts: Vec::new(),
                requested_cursors: Vec::new(),
                requested_sync_dates: Vec::new(),
            }
        }

        fn record_attempt(
            &mut self,
            request: &ScholarlyRequest,
            status_code: Option<u16>,
            did_succeed: bool,
            error: Option<&str>,
        ) {
            self.attempts.push(SourceAttempt {
                service: request.service.clone(),
                endpoint: request.endpoint.clone(),
                method: request.method.clone(),
                url: request.url.clone(),
                status_code,
                did_succeed,
                did_retry: false,
                error: error.map(str::to_string),
            });
        }
    }

    impl ScholarlyTransport for CursorRecoveryTransport {
        fn request(&mut self, request: ScholarlyRequest) -> Result<serde_json::Value, SourceError> {
            match &request.kind {
                ScholarlyRequestKind::CrossrefJournalWorks {
                    from_sync_date,
                    cursor,
                    ..
                } => {
                    self.requested_cursors.push(cursor.clone());
                    self.requested_sync_dates.push(from_sync_date.clone());
                    match self.responses.pop_front().ok_or_else(|| {
                        SourceError::InvalidFixture(
                            "cursor recovery response script exhausted".to_string(),
                        )
                    })? {
                        CrossrefFixtureResponse::Page { items, next_cursor } => {
                            self.record_attempt(&request, Some(200), true, None);
                            Ok(json!({
                                "message": {
                                    "items": items,
                                    "next-cursor": next_cursor,
                                }
                            }))
                        }
                        CrossrefFixtureResponse::HttpStatus(status_code) => {
                            self.record_attempt(
                                &request,
                                Some(status_code),
                                false,
                                Some("http_status"),
                            );
                            Err(SourceError::HttpStatus {
                                service: request.service,
                                endpoint: request.endpoint,
                                status_code,
                                body: json!({"error": "fixture-response-body-sentinel"}),
                            })
                        }
                        CrossrefFixtureResponse::RequestFailure => {
                            self.record_attempt(&request, None, false, Some("transport"));
                            Err(SourceError::Request {
                                service: request.service,
                                endpoint: request.endpoint,
                                message: "fixture-transport-sentinel".to_string(),
                            })
                        }
                    }
                }
                ScholarlyRequestKind::OpenAlexSourceByIssn { .. }
                | ScholarlyRequestKind::OpenAlexSourceByTitle { .. } => {
                    self.record_attempt(&request, Some(200), true, None);
                    Ok(json!({"results": []}))
                }
                ScholarlyRequestKind::OpenAlexWorksBySource { .. }
                | ScholarlyRequestKind::OpenAlexWorksByDoi { .. } => {
                    self.record_attempt(&request, Some(200), true, None);
                    Ok(json!({"results": [], "meta": {"next_cursor": null}}))
                }
                ScholarlyRequestKind::SemanticScholarBatch { .. } => {
                    self.record_attempt(&request, Some(200), true, None);
                    Ok(json!([]))
                }
            }
        }

        fn attempts(&self) -> &[SourceAttempt] {
            &self.attempts
        }

        fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
            std::mem::take(&mut self.attempts)
        }
    }

    fn catalog() -> litradar_domain::JournalCatalogEntry {
        litradar_domain::JournalCatalogEntry {
            catalog_id: "issn-1234-5679".to_string(),
            catalog_aliases: vec!["legacy-journal".to_string()],
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
    }

    fn fetch_context(traversal_checkpoint: Option<&str>) -> IndexFetchContext<'_> {
        sync_context(IndexSyncMode::Bootstrap, None, traversal_checkpoint)
    }

    fn sync_context<'a>(
        mode: IndexSyncMode,
        committed_anchor: Option<&'a str>,
        traversal_checkpoint: Option<&'a str>,
    ) -> IndexFetchContext<'a> {
        IndexFetchContext {
            mode,
            committed_anchor,
            traversal_checkpoint,
        }
    }

    fn batch_is_complete(batch: &ProviderBatch) -> bool {
        matches!(&batch.progress, ProviderProgress::Complete { .. })
    }

    fn batch_checkpoint(batch: &ProviderBatch) -> Option<&str> {
        match &batch.progress {
            ProviderProgress::Continue { checkpoint } => Some(checkpoint),
            ProviderProgress::Complete { .. } => None,
        }
    }

    fn batch_anchor(batch: &ProviderBatch) -> Option<&str> {
        match &batch.progress {
            ProviderProgress::Complete { next_anchor } => next_anchor.as_deref(),
            ProviderProgress::Continue { .. } => None,
        }
    }

    fn into_batch_checkpoint(batch: ProviderBatch) -> Option<String> {
        match batch.progress {
            ProviderProgress::Continue { checkpoint } => Some(checkpoint),
            ProviderProgress::Complete { .. } => None,
        }
    }

    fn fetch_all_batches(
        provider: &dyn IndexContentProvider,
        catalog: &JournalCatalogEntry,
        mode: IndexSyncMode,
        committed_anchor: Option<&str>,
    ) -> Vec<ProviderBatch> {
        let mut batches = Vec::new();
        let mut checkpoint = None;
        for _ in 0..100 {
            let batch = provider
                .fetch(
                    catalog,
                    sync_context(mode, committed_anchor, checkpoint.as_deref()),
                )
                .expect("domestic batch should fetch");
            checkpoint = batch_checkpoint(&batch).map(str::to_string);
            let is_complete = batch_is_complete(&batch);
            batches.push(batch);
            if is_complete {
                return batches;
            }
        }
        panic!("domestic fixture exceeded its batch bound")
    }

    fn assert_domestic_state_is_safe(state: &str) {
        assert!(state.len() <= 65_536);
        let lowered = state.to_ascii_lowercase();
        for forbidden in [
            "captcha",
            "secretkey",
            "pointjson",
            "jfbym",
            "session",
            "cookie",
            "token",
            "http://",
            "https://",
            "url",
        ] {
            assert!(!lowered.contains(forbidden));
        }
    }

    fn article_locator(title: &str, journal_title: &str) -> ArticleLocator {
        ArticleLocator {
            article_id: ArticleId(1),
            catalog_id: "issn-1234-5679".to_string(),
            journal_title: journal_title.to_string(),
            journal_issns: vec!["1234-5679".to_string()],
            title: title.to_string(),
            publication_year: Some(2026),
            date: None,
            authors: Vec::new(),
            volume: None,
            issue_number: Some("01".to_string()),
            start_page: None,
            end_page: None,
            doi: None,
            pmid: None,
        }
    }

    fn crossref_works(start: usize, count: usize) -> Vec<serde_json::Value> {
        (start..start + count)
            .map(|index| {
                json!({
                    "DOI": format!("10.1000/stateful-{index}"),
                    "title": [format!("Stateful cursor article {index}")],
                    "published": {"date-parts": [[2026, 7, 18]]}
                })
            })
            .collect()
    }

    fn recovery_page(next_cursor: Option<&str>) -> CrossrefFixtureResponse {
        CrossrefFixtureResponse::Page {
            items: vec![json!({
                "title": ["Recovery article"],
                "published": {"date-parts": [[2026, 7, 19]]},
                "volume": "1"
            })],
            next_cursor: next_cursor.map(str::to_string),
        }
    }

    fn crossref_checkpoint(
        cursor: &str,
        page_index: u64,
        cursor_refreshed_at_epoch_seconds: Option<u64>,
    ) -> String {
        encode_scholarly_checkpoint(&ScholarlyCheckpoint {
            version: SCHOLARLY_CHECKPOINT_VERSION,
            window: bootstrap_scholarly_window(),
            source: ScholarlySourceCheckpoint::Crossref {
                issn: "1234-5679".to_string(),
                cursor: Some(cursor.to_string()),
                page_index,
                cursor_refreshed_at_epoch_seconds,
            },
        })
        .expect("Crossref checkpoint should encode")
    }

    fn bootstrap_scholarly_window() -> ScholarlyWindowCheckpoint {
        ScholarlyWindowCheckpoint {
            sync_mode: IndexSyncMode::Bootstrap,
            phase: ScholarlyScanPhase::Unbounded,
            base_anchor: None,
            candidate_anchor: None,
            has_reached_candidate: false,
            has_seen_base: false,
        }
    }

    fn scholarly_volume_anchor(issue: &str) -> String {
        encode_scholarly_anchor(&ScholarlyAnchor {
            version: SCHOLARLY_ANCHOR_VERSION,
            issue: ScholarlyIssueFingerprint::VolumeIssue {
                publication_year: 2026,
                volume: Some("1".to_string()),
                issue: Some(issue.to_string()),
            },
            from_sync_date: Some("2026-01-01".to_string()),
        })
        .expect("Scholarly anchor should encode")
    }

    fn crossref_issue_work(issue: &str, suffix: &str) -> Value {
        json!({
            "DOI": format!("10.1000/{suffix}"),
            "title": [format!("Issue {issue} article {suffix}")],
            "published": {"date-parts": [[2026, 7, 18]]},
            "volume": "1",
            "issue": issue
        })
    }

    fn openalex_issue_work(issue: &str, suffix: &str) -> Value {
        json!({
            "doi": format!("https://doi.org/10.1000/{suffix}"),
            "display_name": format!("Issue {issue} article {suffix}"),
            "publication_year": 2026,
            "publication_date": "2026-07-18",
            "biblio": {"volume": "1", "issue": issue}
        })
    }

    fn fetch_cursor_recovery(
        catalog: &JournalCatalogEntry,
        responses: Vec<CrossrefFixtureResponse>,
        checkpoint: &str,
        clock_values: Vec<u64>,
    ) -> (
        Result<ProviderBatch, ProviderError>,
        CursorRecoveryTransport,
    ) {
        let transport = CursorRecoveryTransport::new(responses);
        let mut client = ScholarlyClient::new(transport, false);
        let mut clock_values = VecDeque::from(clock_values);
        let fallback_clock_value = *clock_values.back().unwrap_or(&1_000);
        let mut clock = || Ok(clock_values.pop_front().unwrap_or(fallback_clock_value));
        let mut restart = |_: &'static str, _: u64| {};
        let result = fetch_scholarly_batch_with_clock_and_restart(
            &mut client,
            catalog,
            Some(checkpoint),
            false,
            &mut clock,
            &mut restart,
        );
        (result, client.into_transport())
    }

    fn fetch_cursor_recovery_with_logging(
        catalog: &JournalCatalogEntry,
        responses: Vec<CrossrefFixtureResponse>,
        checkpoint: &str,
        current_epoch_seconds: u64,
    ) -> Result<ProviderBatch, ProviderError> {
        let transport = CursorRecoveryTransport::new(responses);
        let mut client = ScholarlyClient::new(transport, false);
        let mut clock = || Ok(current_epoch_seconds);
        fetch_scholarly_batch_with_clock(&mut client, catalog, Some(checkpoint), false, &mut clock)
    }

    #[test]
    fn built_in_registrations_declare_only_indexing() {
        let scholarly = scholarly_index_registration(
            FixtureScholarlyTransport::new(ScholarlyFixtureData::default()),
            true,
        )
        .expect("Scholarly registration should pass");
        let cnki =
            cnki_oversea_index_registration(FixtureCnkiTransport::new(CnkiFixtureData::default()))
                .expect("CNKI registration should pass");
        let mut registry = ProviderRegistry::default();
        registry
            .register(scholarly)
            .expect("Scholarly should register");
        registry.register(cnki).expect("CNKI should register");
        assert_eq!(
            registry
                .providers_with(ProviderCapabilityKind::IndexContent)
                .len(),
            2
        );
        assert!(registry
            .providers_with(ProviderCapabilityKind::ArticleAbstract)
            .is_empty());
    }

    #[test]
    fn access_registrations_declare_only_optional_online_capabilities() {
        let scholarly = scholarly_access_registration().expect("Scholarly access should register");
        assert!(scholarly.index_content().is_none());
        assert!(scholarly.article_full_text().is_none());
        assert_eq!(
            scholarly.descriptor().allowed_redirect_hosts,
            SCHOLARLY_REDIRECT_HOSTS
        );
        let scholarly_redirect = scholarly
            .article_abstract()
            .expect("abstract capability should exist")
            .resolve_abstract(
                &ArticleLocator {
                    doi: Some("10.1000/article".to_string()),
                    ..article_locator("Article", "Canonical Journal")
                },
                ArticleAccessContext::default(),
            )
            .expect("Scholarly abstract should resolve online");
        assert_eq!(
            scholarly_redirect.location,
            "https://doi.org/10.1000/article"
        );
        let fixture = CnkiFixtureData {
            journal_detail_html: r#"
                <html><head><title>CNKI Test Journal - 中国知网</title></head>
                <body>
                  <input id="pykm" value="TEST" />
                  <input id="pCode" value="CJFD" />
                  <input id="shareChName" value="CNKI Test Journal" />
                  <input id="issn" value="1234-5679" />
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
            fail_endpoint: None,
        };
        let cnki = cnki_oversea_access_registration(FixtureCnkiTransport::new(fixture))
            .expect("CNKI access should register");
        assert!(cnki.index_content().is_none());
        assert!(cnki.article_full_text().is_none());
        assert_eq!(
            cnki.descriptor().allowed_redirect_hosts,
            CNKI_REDIRECT_HOSTS
        );
        let cnki_redirect = cnki
            .article_abstract()
            .expect("abstract capability should exist")
            .resolve_abstract(
                &article_locator("CNKI article", "CNKI Test Journal"),
                ArticleAccessContext::default(),
            )
            .expect("CNKI abstract should resolve online");
        assert!(cnki_redirect
            .location
            .starts_with("https://oversea.cnki.net/"));
    }

    #[test]
    fn provider_payload_variants_produce_the_same_canonical_article() {
        let catalog = catalog();
        let scholarly = scholarly_article_draft(
            &catalog,
            &json!({
                "DOI": "https://doi.org/10.1000/SAME",
                "title": ["Shared Article"],
                "published": {"date-parts": [[2026, 7, 18]]},
                "volume": "1",
                "issue": "2",
                "page": "1-8",
                "author": [{"given": "Ada", "family": "Lovelace"}]
            }),
            None,
            None,
        )
        .expect("Scholarly article should convert");
        let issue = cnki_issue_draft(
            &catalog,
            &json!({"year": 2026, "volume": "1", "number": "2", "title": "2026 No.2"}),
        )
        .expect("CNKI issue should convert");
        let cnki = cnki_article_draft(
            &catalog,
            &issue,
            &json!({"title": "Shared Article", "authors": "Ada Lovelace", "pages": "1-8"}),
            &json!({
                "title": "Shared Article",
                "authors": "Ada Lovelace",
                "doi": "10.1000/same",
                "date": "2026-07-18",
                "pages": "1-8"
            }),
        )
        .expect("CNKI article should convert");
        assert_eq!(scholarly.title, cnki.title);
        assert_eq!(scholarly.doi, cnki.doi);
        assert_eq!(scholarly.publication_year, cnki.publication_year);
        assert_eq!(scholarly.date, cnki.date);
        assert_eq!(scholarly.volume, cnki.volume);
        assert_eq!(scholarly.issue_number, cnki.issue_number);
        assert_eq!(scholarly.start_page, cnki.start_page);
        assert_eq!(scholarly.end_page, cnki.end_page);
        assert_eq!(scholarly.authors, cnki.authors);
    }

    #[test]
    fn provider_dates_preserve_partial_precision_and_reject_impossible_days() {
        let catalog = catalog();
        for (parts, expected) in [
            (json!([2026]), Some("2026")),
            (json!([2026, 2]), Some("2026-02")),
            (json!([2024, 2, 29]), Some("2024-02-29")),
            (json!([2026, 2, 29]), None),
            (json!([2026, 2, 31]), None),
        ] {
            let article = scholarly_article_draft(
                &catalog,
                &json!({
                    "DOI": "10.1000/partial-date",
                    "title": ["Partial date"],
                    "published": {"date-parts": [parts]}
                }),
                None,
                None,
            )
            .expect("DOI should keep the article identifiable");
            assert_eq!(article.date.as_deref(), expected);
        }

        let invalid_openalex = openalex_article_draft(
            &catalog,
            &json!({
                "doi": "10.1000/invalid-openalex-date",
                "display_name": "Invalid OpenAlex date",
                "publication_year": 2026,
                "publication_date": "2026-02-31"
            }),
        )
        .expect("DOI should keep the article identifiable");
        assert_eq!(invalid_openalex.date, None);
    }

    #[test]
    fn scholarly_retractions_ignore_generic_relations_and_use_typed_updates() {
        let catalog = catalog();
        let generic_relation = scholarly_article_draft(
            &catalog,
            &json!({
                "DOI": "10.1000/article",
                "title": ["Article with a generic relation"],
                "published": {"date-parts": [[2026, 7, 18]]},
                "relation": {
                    "references": [{"id": "10.1000/not-a-retraction"}]
                }
            }),
            None,
            None,
        )
        .expect("Scholarly article should convert");
        assert!(generic_relation.retraction_dois.is_empty());

        let typed_updates = scholarly_article_draft(
            &catalog,
            &json!({
                "DOI": "10.1000/article",
                "title": ["Article with typed updates"],
                "published": {"date-parts": [[2026, 7, 18]]},
                "updated-by": [
                    {"type": "correction", "DOI": "10.1000/correction"},
                    {"type": "retraction", "DOI": "10.1000/retraction-b"},
                    {"type": "Retraction", "DOI": "https://doi.org/10.1000/RETRACTION-A"},
                    {"type": "retraction", "DOI": "10.1000/retraction-a"}
                ]
            }),
            None,
            None,
        )
        .expect("Scholarly article should convert");
        assert_eq!(
            typed_updates.retraction_dois,
            ["10.1000/retraction-a", "10.1000/retraction-b"]
        );
    }

    #[test]
    fn provider_types_are_constructible_without_storage_dependencies() {
        let _ = ScholarlyIndexProvider::new(
            FixtureScholarlyTransport::new(ScholarlyFixtureData::default()),
            true,
        );
        let _ = CnkiIndexProvider::new(FixtureCnkiTransport::new(CnkiFixtureData::default()));
    }

    #[test]
    fn scholarly_registration_fetches_a_canonical_crossref_batch() {
        let registration = scholarly_index_registration(
            FixtureScholarlyTransport::new(ScholarlyFixtureData {
                crossref_works: vec![json!({
                    "DOI": "10.1000/crossref",
                    "title": ["Crossref Article"],
                    "published": {"date-parts": [[2026, 7, 18]]},
                    "volume": "1",
                    "issue": "2",
                    "page": "1-8"
                })],
                ..ScholarlyFixtureData::default()
            }),
            false,
        )
        .expect("Scholarly registration should pass");
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&catalog(), fetch_context(None))
            .expect("Crossref fixture should fetch");

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 1);
        assert_eq!(batch.articles[0].doi.as_deref(), Some("10.1000/crossref"));
    }

    #[test]
    fn scholarly_registration_traverses_stateful_crossref_cursor() {
        let registration = scholarly_index_registration(
            FixtureScholarlyTransport::new(ScholarlyFixtureData {
                crossref_work_pages: vec![
                    crossref_works(0, 225),
                    crossref_works(225, 225),
                    crossref_works(450, 1),
                ],
                ..ScholarlyFixtureData::default()
            }),
            false,
        )
        .expect("Scholarly registration should pass");
        let provider = registration
            .index_content()
            .expect("indexing capability should exist");
        let catalog = catalog();
        let mut checkpoint = None;
        let mut checkpoints = Vec::new();
        let mut dois = BTreeSet::new();
        let mut batch_count = 0;

        loop {
            let batch = provider
                .fetch(&catalog, fetch_context(checkpoint.as_deref()))
                .expect("stateful Crossref page should fetch");
            batch_count += 1;
            let is_complete = batch_is_complete(&batch);
            let next_checkpoint = batch_checkpoint(&batch).map(str::to_string);
            for article in batch.articles {
                dois.insert(article.doi.expect("fixture article should have a DOI"));
            }
            if is_complete {
                assert!(next_checkpoint.is_none());
                break;
            }
            let next_checkpoint =
                next_checkpoint.expect("incomplete batch should have a checkpoint");
            checkpoints.push(next_checkpoint.clone());
            checkpoint = Some(next_checkpoint);
        }

        assert_eq!(batch_count, 3);
        assert_eq!(dois.len(), 451);
        assert_eq!(checkpoints.len(), 2);
        assert_ne!(checkpoints[0], checkpoints[1]);
        let parsed = checkpoints
            .iter()
            .map(|checkpoint| {
                serde_json::from_str::<ScholarlyCheckpoint>(checkpoint)
                    .expect("checkpoint should decode")
            })
            .collect::<Vec<_>>();
        let cursor_pages = parsed
            .iter()
            .map(|checkpoint| match &checkpoint.source {
                ScholarlySourceCheckpoint::Crossref {
                    cursor, page_index, ..
                } => (
                    cursor
                        .as_deref()
                        .expect("continued page should have a cursor"),
                    *page_index,
                ),
                ScholarlySourceCheckpoint::OpenAlex { .. } => {
                    panic!("Crossref fixture should not emit an OpenAlex checkpoint")
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(cursor_pages[0].0, cursor_pages[1].0);
        assert_eq!(
            cursor_pages
                .iter()
                .map(|(_, page_index)| *page_index)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn scholarly_incremental_crossref_keeps_one_filter_and_completes_the_split_base_issue() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_work_pages: vec![
                vec![
                    crossref_issue_work("3", "head"),
                    crossref_issue_work("2", "base-first"),
                ],
                vec![
                    crossref_issue_work("2", "base-second"),
                    crossref_issue_work("1", "older"),
                ],
            ],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);
        let base_anchor = scholarly_volume_anchor("2");
        let mut checkpoint = None;
        let mut batches = Vec::new();
        let mut now = 1_000_u64;
        let mut clock = || {
            now += 1;
            Ok(now)
        };
        let mut restart = |_: &'static str, _: u64| {};
        for _ in 0..3 {
            let batch = fetch_scholarly_batch_for_context_with_clock_and_restart(
                &mut client,
                &catalog(),
                sync_context(
                    IndexSyncMode::Incremental,
                    Some(&base_anchor),
                    checkpoint.as_deref(),
                ),
                true,
                &mut clock,
                &mut restart,
            )
            .expect("bounded Crossref page should fetch");
            checkpoint = batch_checkpoint(&batch).map(str::to_string);
            let is_complete = batch_is_complete(&batch);
            batches.push(batch);
            if is_complete {
                break;
            }
        }
        let transport = client.into_transport();

        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches
                .iter()
                .flat_map(|batch| batch.articles.iter())
                .filter_map(|article| article.doi.as_deref())
                .collect::<Vec<_>>(),
            ["10.1000/head", "10.1000/base-first", "10.1000/base-second"]
        );
        assert_eq!(
            transport.journal_work_requests(),
            &[
                ("1234-5679".to_string(), Some("2026-01-01".to_string())),
                ("1234-5679".to_string(), Some("2026-01-01".to_string()))
            ]
        );
        let expected_enrichment_batches = [
            vec!["10.1000/head".to_string(), "10.1000/base-first".to_string()],
            vec!["10.1000/base-second".to_string()],
        ];
        assert_eq!(
            transport.openalex_doi_batches(),
            expected_enrichment_batches
        );
        assert_eq!(
            transport.semantic_scholar_batches(),
            expected_enrichment_batches
        );
        let anchor = decode_scholarly_anchor(
            batch_anchor(batches.last().expect("completion batch should exist"))
                .expect("completion should advance the anchor"),
        )
        .expect("next anchor should decode");
        assert_eq!(
            anchor.issue,
            ScholarlyIssueFingerprint::VolumeIssue {
                publication_year: 2026,
                volume: Some("1".to_string()),
                issue: Some("3".to_string()),
            }
        );
    }

    #[test]
    fn scholarly_bounded_crossref_filters_two_hundred_twenty_five_works_before_enrichment() {
        let mut works = Vec::with_capacity(225);
        works.extend((0..5).map(|index| crossref_issue_work("3", &format!("candidate-{index}"))));
        works.extend((0..5).map(|index| crossref_issue_work("2", &format!("base-{index}"))));
        works.extend((0..215).map(|index| crossref_issue_work("1", &format!("older-{index}"))));
        assert_eq!(works.len(), 225);
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_work_pages: vec![works],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);
        let base_anchor = scholarly_volume_anchor("2");
        let mut clock = || Ok(10_000);
        let mut restart = |_: &'static str, _: u64| {};

        let batch = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(IndexSyncMode::Incremental, Some(&base_anchor), None),
            true,
            &mut clock,
            &mut restart,
        )
        .expect("bounded 225-work page should complete");
        let transport = client.into_transport();
        let expected_dois = (0..5)
            .map(|index| format!("10.1000/candidate-{index}"))
            .chain((0..5).map(|index| format!("10.1000/base-{index}")))
            .collect::<Vec<_>>();

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), expected_dois.len());
        assert_eq!(
            batch
                .articles
                .iter()
                .filter_map(|article| article.doi.clone())
                .collect::<Vec<_>>(),
            expected_dois
        );
        assert_eq!(
            transport
                .openalex_doi_batches()
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            expected_dois
        );
        assert_eq!(
            transport
                .semantic_scholar_batches()
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            expected_dois
        );
    }

    #[test]
    fn scholarly_unfingerprintable_bounded_page_replays_before_enrichment() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_work_pages: vec![vec![json!({
                "DOI": "10.1000/unfingerprintable",
                "title": ["Unfingerprintable bounded article"]
            })]],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, true);
        let base_anchor = scholarly_volume_anchor("2");
        let mut clock = || Ok(20_000);
        let mut restart = |_: &'static str, _: u64| {};

        let batch = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(IndexSyncMode::Incremental, Some(&base_anchor), None),
            true,
            &mut clock,
            &mut restart,
        )
        .expect("unfingerprintable bounded page should schedule replay");
        let checkpoint = decode_scholarly_checkpoint(
            batch_checkpoint(&batch).expect("unfingerprintable page should continue"),
        )
        .expect("unbounded replay checkpoint should decode");
        let transport = client.into_transport();

        assert!(batch.articles.is_empty());
        assert_eq!(checkpoint.window.phase, ScholarlyScanPhase::Unbounded);
        assert!(transport.openalex_doi_batches().is_empty());
        assert!(transport.semantic_scholar_batches().is_empty());
    }

    #[test]
    fn scholarly_missing_bounded_base_replays_once_without_a_date_filter() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_work_pages: vec![
                vec![crossref_issue_work("4", "new-head")],
                vec![crossref_issue_work("3", "new-tail")],
            ],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, false);
        let base_anchor = scholarly_volume_anchor("2");
        let mut checkpoint = None;
        let mut batches = Vec::new();
        let mut now = 2_000_u64;
        let mut clock = || {
            now += 1;
            Ok(now)
        };
        let mut restart = |_: &'static str, _: u64| {};
        for _ in 0..6 {
            let batch = fetch_scholarly_batch_for_context_with_clock_and_restart(
                &mut client,
                &catalog(),
                sync_context(
                    IndexSyncMode::Incremental,
                    Some(&base_anchor),
                    checkpoint.as_deref(),
                ),
                false,
                &mut clock,
                &mut restart,
            )
            .expect("bounded fallback should remain resumable");
            checkpoint = batch_checkpoint(&batch).map(str::to_string);
            let is_complete = batch_is_complete(&batch);
            batches.push(batch);
            if is_complete {
                break;
            }
        }
        let transport = client.into_transport();

        assert_eq!(batches.len(), 4);
        assert_eq!(
            transport
                .journal_work_requests()
                .iter()
                .map(|(_, date)| date.as_deref())
                .collect::<Vec<_>>(),
            [Some("2026-01-01"), Some("2026-01-01"), None, None]
        );
        let anchor = decode_scholarly_anchor(
            batch_anchor(batches.last().expect("completion batch should exist"))
                .expect("full replay should establish the new head"),
        )
        .expect("next anchor should decode");
        assert!(matches!(
            anchor.issue,
            ScholarlyIssueFingerprint::VolumeIssue {
                issue: Some(issue),
                ..
            } if issue == "4"
        ));
    }

    #[test]
    fn scholarly_empty_future_window_normalizes_persisted_unbounded_replay() {
        let future_work = json!({
            "DOI": "10.1000/future-base",
            "title": ["Future base article"],
            "published": {"date-parts": [[2027, 1, 1]]},
            "volume": "186"
        });
        let transport = CursorRecoveryTransport::new(vec![
            CrossrefFixtureResponse::Page {
                items: Vec::new(),
                next_cursor: None,
            },
            CrossrefFixtureResponse::Page {
                items: vec![future_work],
                next_cursor: None,
            },
        ]);
        let mut client = ScholarlyClient::new(transport, false);
        let base_anchor = encode_scholarly_anchor(&ScholarlyAnchor {
            version: SCHOLARLY_ANCHOR_VERSION,
            issue: ScholarlyIssueFingerprint::VolumeIssue {
                publication_year: 2027,
                volume: Some("186".to_string()),
                issue: None,
            },
            from_sync_date: Some("2027-01-01".to_string()),
        })
        .expect("future anchor should encode");
        let (bounded_window, source) = scholarly_window_from_context(sync_context(
            IndexSyncMode::Incremental,
            Some(&base_anchor),
            None,
        ))
        .expect("future bounded window should decode");
        assert!(source.is_none());
        assert_eq!(
            scholarly_window_filter_at(&bounded_window, Some("2026-07-29")),
            None
        );
        let mut now = 3_000_u64;
        let mut clock = || {
            now += 1;
            Ok(now)
        };
        let mut restart = |_: &'static str, _: u64| {};

        let fallback = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(IndexSyncMode::Incremental, Some(&base_anchor), None),
            false,
            &mut clock,
            &mut restart,
        )
        .expect("empty future window should enter resumable replay");
        assert!(fallback.articles.is_empty());
        let checkpoint = batch_checkpoint(&fallback).expect("fallback checkpoint should exist");
        let decoded = decode_scholarly_checkpoint(checkpoint)
            .expect("new fallback checkpoint should remain valid");
        assert_eq!(decoded.window.phase, ScholarlyScanPhase::Unbounded);
        assert!(decoded.window.candidate_anchor.is_none());
        assert!(!decoded.window.has_reached_candidate);

        let mut persisted = serde_json::from_str::<Value>(checkpoint)
            .expect("fallback checkpoint JSON should decode");
        persisted["window"]["has_reached_candidate"] = Value::Bool(true);
        let persisted =
            serde_json::to_string(&persisted).expect("legacy persisted checkpoint should encode");
        let mut deterministic = serde_json::from_str::<ScholarlyCheckpoint>(&persisted)
            .expect("legacy persisted checkpoint should decode structurally");
        normalize_empty_unbounded_replay_checkpoint_at(&mut deterministic, Some("2026-07-29"));
        assert_eq!(deterministic.window.phase, ScholarlyScanPhase::Bounded);
        assert!(!deterministic.window.has_reached_candidate);
        let normalized = decode_scholarly_checkpoint(&persisted)
            .expect("legacy empty replay checkpoint should normalize");
        assert!(!normalized.window.has_reached_candidate);

        let completed = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(
                IndexSyncMode::Incremental,
                Some(&base_anchor),
                Some(&persisted),
            ),
            false,
            &mut clock,
            &mut restart,
        )
        .expect("normalized replay should fetch from the unfiltered head");
        assert!(batch_is_complete(&completed));
        assert_eq!(completed.articles.len(), 1);
        let next_anchor = decode_scholarly_anchor(
            batch_anchor(&completed).expect("replay should retain the future head"),
        )
        .expect("next anchor should decode");
        assert_eq!(
            next_anchor.issue,
            ScholarlyIssueFingerprint::VolumeIssue {
                publication_year: 2027,
                volume: Some("186".to_string()),
                issue: None,
            }
        );
        let requested_sync_dates = client.into_transport().requested_sync_dates;
        assert_eq!(requested_sync_dates.len(), 2);
        assert!(requested_sync_dates[1].is_none());
    }

    #[test]
    fn scholarly_crossref_cursor_restart_preserves_the_frozen_window() {
        let responses = vec![
            CrossrefFixtureResponse::Page {
                items: vec![crossref_issue_work("3", "frozen-head")],
                next_cursor: Some("stateful".to_string()),
            },
            CrossrefFixtureResponse::HttpStatus(500),
            CrossrefFixtureResponse::Page {
                items: vec![
                    crossref_issue_work("4", "inserted-head"),
                    crossref_issue_work("3", "frozen-head"),
                    crossref_issue_work("2", "base"),
                ],
                next_cursor: None,
            },
        ];
        let transport = CursorRecoveryTransport::new(responses);
        let mut client = ScholarlyClient::new(transport, false);
        let base_anchor = scholarly_volume_anchor("2");
        let mut clock_values = VecDeque::from([900_u64, 1_000]);
        let mut clock = || Ok(clock_values.pop_front().unwrap_or(1_000));
        let mut restart_count = 0;
        let mut restart = |_: &'static str, _: u64| restart_count += 1;
        let first = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(IndexSyncMode::Incremental, Some(&base_anchor), None),
            false,
            &mut clock,
            &mut restart,
        )
        .expect("first bounded page should fetch");
        let checkpoint = batch_checkpoint(&first)
            .expect("first page should continue")
            .to_string();
        let second = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(
                IndexSyncMode::Incremental,
                Some(&base_anchor),
                Some(&checkpoint),
            ),
            false,
            &mut clock,
            &mut restart,
        )
        .expect("cursor restart should replay the frozen window");
        let transport = client.into_transport();

        assert!(batch_is_complete(&second));
        assert_eq!(restart_count, 1);
        assert_eq!(
            transport.requested_cursors,
            [None, Some("stateful".to_string()), None]
        );
        assert_eq!(
            transport
                .requested_sync_dates
                .iter()
                .map(|date| date.as_deref())
                .collect::<Vec<_>>(),
            [Some("2026-01-01"), Some("2026-01-01"), Some("2026-01-01")]
        );
        assert_eq!(
            second
                .articles
                .iter()
                .filter_map(|article| article.doi.as_deref())
                .collect::<Vec<_>>(),
            ["10.1000/frozen-head", "10.1000/base"]
        );
        let anchor = decode_scholarly_anchor(
            batch_anchor(&second).expect("completion should retain the frozen head"),
        )
        .expect("next anchor should decode");
        assert!(matches!(
            anchor.issue,
            ScholarlyIssueFingerprint::VolumeIssue {
                issue: Some(issue),
                ..
            } if issue == "3"
        ));
    }

    #[test]
    fn scholarly_openalex_plan_restriction_switches_once_to_unfiltered_pages() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_status: Some(404),
            openalex_source_by_issns: Some(json!({
                "id": "https://openalex.org/S1",
                "display_name": "Canonical Journal",
                "issn_l": "1234-5679",
                "issn": ["1234-5679"]
            })),
            openalex_source_work_pages: vec![
                vec![openalex_issue_work("3", "openalex-head")],
                vec![
                    openalex_issue_work("2", "openalex-base"),
                    openalex_issue_work("1", "openalex-older"),
                ],
            ],
            openalex_source_works_plan_restricted_after_page: Some(1),
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, false);
        let base_anchor = scholarly_volume_anchor("2");
        let mut checkpoint = None;
        let mut batches = Vec::new();
        let mut clock = || Ok(1_000_u64);
        let mut restart = |_: &'static str, _: u64| {};
        for _ in 0..4 {
            let batch = fetch_scholarly_batch_for_context_with_clock_and_restart(
                &mut client,
                &catalog(),
                sync_context(
                    IndexSyncMode::Incremental,
                    Some(&base_anchor),
                    checkpoint.as_deref(),
                ),
                false,
                &mut clock,
                &mut restart,
            )
            .expect("OpenAlex plan fallback should remain provider-local");
            checkpoint = batch_checkpoint(&batch).map(str::to_string);
            let is_complete = batch_is_complete(&batch);
            batches.push(batch);
            if is_complete {
                break;
            }
        }
        let transport = client.into_transport();

        assert_eq!(batches.len(), 3);
        assert!(batch_is_complete(
            batches.last().expect("completion batch should exist")
        ));
        assert_eq!(
            transport.source_work_requests(),
            &[
                (
                    "https://openalex.org/S1".to_string(),
                    Some("2026-01-01".to_string())
                ),
                (
                    "https://openalex.org/S1".to_string(),
                    Some("2026-01-01".to_string())
                ),
                ("https://openalex.org/S1".to_string(), None),
                ("https://openalex.org/S1".to_string(), None)
            ]
        );
        let anchor = decode_scholarly_anchor(
            batch_anchor(batches.last().expect("completion batch should exist"))
                .expect("fallback should establish a new anchor"),
        )
        .expect("next anchor should decode");
        assert!(matches!(
            anchor.issue,
            ScholarlyIssueFingerprint::VolumeIssue {
                issue: Some(issue),
                ..
            } if issue == "3"
        ));
    }

    #[test]
    fn scholarly_incremental_openalex_keeps_the_anchor_filter_across_pages() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData {
            crossref_status: Some(404),
            openalex_source_by_issns: Some(json!({
                "id": "https://openalex.org/S1",
                "display_name": "Canonical Journal",
                "issn_l": "1234-5679",
                "issn": ["1234-5679"]
            })),
            openalex_source_work_pages: vec![
                vec![
                    openalex_issue_work("3", "openalex-bounded-head"),
                    openalex_issue_work("2", "openalex-bounded-base-first"),
                ],
                vec![
                    openalex_issue_work("2", "openalex-bounded-base-second"),
                    openalex_issue_work("1", "openalex-bounded-older"),
                ],
            ],
            ..ScholarlyFixtureData::default()
        });
        let mut client = ScholarlyClient::new(transport, false);
        let base_anchor = scholarly_volume_anchor("2");
        let mut checkpoint = None;
        let mut batches = Vec::new();
        let mut clock = || Ok(1_000_u64);
        let mut restart = |_: &'static str, _: u64| {};
        for _ in 0..3 {
            let batch = fetch_scholarly_batch_for_context_with_clock_and_restart(
                &mut client,
                &catalog(),
                sync_context(
                    IndexSyncMode::Incremental,
                    Some(&base_anchor),
                    checkpoint.as_deref(),
                ),
                false,
                &mut clock,
                &mut restart,
            )
            .expect("bounded OpenAlex page should fetch");
            checkpoint = batch_checkpoint(&batch).map(str::to_string);
            let is_complete = batch_is_complete(&batch);
            batches.push(batch);
            if is_complete {
                break;
            }
        }
        let transport = client.into_transport();

        assert_eq!(batches.len(), 2);
        assert_eq!(
            transport.source_work_requests(),
            &[
                (
                    "https://openalex.org/S1".to_string(),
                    Some("2026-01-01".to_string())
                ),
                (
                    "https://openalex.org/S1".to_string(),
                    Some("2026-01-01".to_string())
                )
            ]
        );
        assert_eq!(
            batches
                .iter()
                .flat_map(|batch| batch.articles.iter())
                .filter_map(|article| article.doi.as_deref())
                .collect::<Vec<_>>(),
            [
                "10.1000/openalex-bounded-head",
                "10.1000/openalex-bounded-base-first",
                "10.1000/openalex-bounded-base-second"
            ]
        );
        let next_anchor = batch_anchor(batches.last().expect("completion batch should exist"))
            .expect("completion should advance the anchor");
        assert!(!next_anchor.contains("openalex"));
        assert!(!next_anchor.contains("10.1000"));
        let anchor = decode_scholarly_anchor(next_anchor).expect("next anchor should decode");
        assert!(matches!(
            anchor.issue,
            ScholarlyIssueFingerprint::VolumeIssue {
                issue: Some(issue),
                ..
            } if issue == "3"
        ));
    }

    #[test]
    fn scholarly_crossref_and_openalex_share_canonical_issue_fingerprints() {
        let crossref_work = json!({
            "DOI": "10.1000/crossref-fingerprint",
            "title": ["Crossref fingerprint"],
            "published": {"date-parts": [[2026, 7, 18]]},
            "volume": "01",
            "issue": "002"
        });
        let crossref = scholarly_article_draft(&catalog(), &crossref_work, None, None)
            .expect("Crossref fixture should map");
        let openalex = openalex_article_draft(
            &catalog(),
            &json!({
                "doi": "https://doi.org/10.1000/openalex-fingerprint",
                "display_name": "OpenAlex fingerprint",
                "publication_year": 2026,
                "publication_date": "2026-07-18",
                "biblio": {"volume": "1", "issue": "2"}
            }),
        )
        .expect("OpenAlex fixture should map");

        assert_eq!(
            crossref_work_issue_anchor(&crossref_work)
                .expect("raw Crossref issue should fingerprint"),
            scholarly_issue_anchor(&crossref).expect("Crossref issue should fingerprint")
        );
        assert_eq!(
            scholarly_issue_anchor(&crossref)
                .expect("Crossref issue should fingerprint")
                .issue,
            scholarly_issue_anchor(&openalex)
                .expect("OpenAlex issue should fingerprint")
                .issue
        );
    }

    #[test]
    fn scholarly_malformed_anchor_fails_before_any_source_request() {
        let transport = FixtureScholarlyTransport::new(ScholarlyFixtureData::default());
        let mut client = ScholarlyClient::new(transport, false);
        let mut clock = || Ok(1_000_u64);
        let mut restart = |_: &'static str, _: u64| {};
        let error = fetch_scholarly_batch_for_context_with_clock_and_restart(
            &mut client,
            &catalog(),
            sync_context(IndexSyncMode::Incremental, Some("{}"), None),
            false,
            &mut clock,
            &mut restart,
        )
        .expect_err("malformed anchor must fail closed");
        let transport = client.into_transport();

        assert_eq!(error.kind(), ProviderErrorKind::InvalidResponse);
        assert!(transport.journal_work_requests().is_empty());
        assert!(transport.source_work_requests().is_empty());
    }

    #[test]
    fn crossref_cursor_freshness_has_an_exact_240_second_boundary() {
        assert_eq!(CROSSREF_CURSOR_REUSE_SECONDS, 240);
        assert!(crossref_cursor_is_fresh(Some(761), 1_000));
        assert!(!crossref_cursor_is_fresh(Some(760), 1_000));
        assert!(!crossref_cursor_is_fresh(Some(1_001), 1_000));
        assert!(!crossref_cursor_is_fresh(None, 1_000));
    }

    #[test]
    fn old_scholarly_checkpoint_is_rejected_and_stale_cursors_restart_before_use() {
        let legacy =
            r#"{"mode":"crossref","issn":"1234-5679","cursor":"legacy-secret"}"#.to_string();
        let (legacy_result, legacy_transport) =
            fetch_cursor_recovery(&catalog(), Vec::new(), &legacy, vec![1_000]);
        assert_eq!(
            legacy_result
                .expect_err("old checkpoint must fail closed")
                .kind(),
            ProviderErrorKind::InvalidResponse
        );
        assert!(legacy_transport.requested_cursors.is_empty());
        let checkpoints = [
            crossref_checkpoint("expired-secret", 4, Some(700)),
            crossref_checkpoint("boundary-secret", 5, Some(760)),
            crossref_checkpoint("future-secret", 6, Some(1_001)),
        ];
        for checkpoint in checkpoints {
            let (result, transport) = fetch_cursor_recovery(
                &catalog(),
                vec![recovery_page(None)],
                &checkpoint,
                vec![1_000],
            );
            let batch = result.expect("stale checkpoint should restart successfully");
            assert!(batch_is_complete(&batch));
            assert_eq!(transport.requested_cursors, vec![None]);
            assert!(transport.responses.is_empty());
        }
    }

    #[test]
    fn fresh_crossref_checkpoint_reuses_cursor_and_refreshes_epoch() {
        let checkpoint = crossref_checkpoint("stateful", 7, Some(761));
        let (result, transport) = fetch_cursor_recovery(
            &catalog(),
            vec![recovery_page(Some("stateful"))],
            &checkpoint,
            vec![1_000, 1_001],
        );
        let batch = result.expect("fresh checkpoint should continue");
        let next = serde_json::from_str::<ScholarlyCheckpoint>(
            batch_checkpoint(&batch).expect("continued page should retain a checkpoint"),
        )
        .expect("continued checkpoint should decode");

        assert_eq!(
            transport.requested_cursors,
            vec![Some("stateful".to_string())]
        );
        assert!(matches!(
            next.source,
            ScholarlySourceCheckpoint::Crossref {
                issn,
                cursor: Some(cursor),
                page_index: 8,
                cursor_refreshed_at_epoch_seconds: Some(1_001),
            } if issn == "1234-5679" && cursor == "stateful"
        ));
    }

    #[test]
    fn crossref_cursor_http_500_uses_one_bounded_fresh_session_fallback() {
        let checkpoint = crossref_checkpoint("stored-cursor", 9, Some(900));
        let (success, success_transport) = fetch_cursor_recovery(
            &catalog(),
            vec![
                CrossrefFixtureResponse::HttpStatus(500),
                recovery_page(None),
            ],
            &checkpoint,
            vec![1_000],
        );
        assert!(batch_is_complete(
            &success.expect("fresh fallback should succeed")
        ));
        assert_eq!(
            success_transport.requested_cursors,
            vec![Some("stored-cursor".to_string()), None]
        );

        let (failure, failure_transport) = fetch_cursor_recovery(
            &catalog(),
            vec![
                CrossrefFixtureResponse::HttpStatus(500),
                CrossrefFixtureResponse::HttpStatus(500),
            ],
            &checkpoint,
            vec![1_000],
        );
        let error = failure.expect_err("failing fresh fallback should fail loud");
        assert_eq!(error.kind(), ProviderErrorKind::TemporarilyUnavailable);
        assert_eq!(
            failure_transport.requested_cursors,
            vec![Some("stored-cursor".to_string()), None]
        );
        assert!(failure_transport.responses.is_empty());
    }

    #[test]
    fn non_500_and_transport_cursor_failures_do_not_restart() {
        let checkpoint = crossref_checkpoint("stored-cursor", 10, Some(900));
        let responses = [
            CrossrefFixtureResponse::HttpStatus(429),
            CrossrefFixtureResponse::HttpStatus(502),
            CrossrefFixtureResponse::HttpStatus(503),
            CrossrefFixtureResponse::HttpStatus(504),
            CrossrefFixtureResponse::RequestFailure,
        ];
        for response in responses {
            let (result, transport) =
                fetch_cursor_recovery(&catalog(), vec![response], &checkpoint, vec![1_000]);
            let error = result.expect_err("non-500 cursor failure should fail loud");
            assert_eq!(error.kind(), ProviderErrorKind::TemporarilyUnavailable);
            assert_eq!(
                transport.requested_cursors,
                vec![Some("stored-cursor".to_string())]
            );
            assert!(transport.responses.is_empty());
        }
    }

    #[test]
    fn crossref_restart_events_are_symbolic_and_private() {
        let logs = CapturedLogs::default();
        let mut private_catalog = catalog();
        private_catalog.catalog_id = "catalog-private-sentinel".to_string();
        private_catalog.title = "title-private-sentinel".to_string();
        private_catalog.issn = Some("9876-5432".to_string());
        private_catalog.all_issns = vec!["9876-5432".to_string()];
        tracing::subscriber::with_default(logs.subscriber(), || {
            let expired = crossref_checkpoint("expired-cursor-sentinel", 17, Some(700));
            let expired_result = fetch_cursor_recovery_with_logging(
                &private_catalog,
                vec![recovery_page(None)],
                &expired,
                1_000,
            );
            expired_result.expect("expired checkpoint should recover");

            let fresh = crossref_checkpoint("http-500-cursor-sentinel", 18, Some(900));
            let private_page = CrossrefFixtureResponse::Page {
                items: vec![json!({
                    "DOI": "10.1000/private-doi-sentinel",
                    "title": ["private-article-title-sentinel"],
                    "published": {"date-parts": [[2026, 7, 19]]},
                    "updated-by": [{
                        "type": "retraction",
                        "DOI": "10.1000/private-retraction-sentinel",
                        "source": "private-update-source-sentinel"
                    }]
                })],
                next_cursor: None,
            };
            let fallback_result = fetch_cursor_recovery_with_logging(
                &private_catalog,
                vec![CrossrefFixtureResponse::HttpStatus(500), private_page],
                &fresh,
                1_000,
            );
            fallback_result.expect("HTTP 500 checkpoint should recover");
        });
        let restart_events = logs
            .events()
            .into_iter()
            .filter(|event| event["event"] == "source.crossref.cursor_restarted")
            .collect::<Vec<_>>();

        assert_eq!(restart_events.len(), 2, "captured logs: {}", logs.text());
        assert_eq!(restart_events[0]["provider"], "crossref");
        assert_eq!(restart_events[0]["reason"], "expired_or_legacy");
        assert_eq!(restart_events[0]["prior_page_index"], 17);
        assert_eq!(restart_events[1]["provider"], "crossref");
        assert_eq!(restart_events[1]["reason"], "cursor_http_500");
        assert_eq!(restart_events[1]["prior_page_index"], 18);
        for private_value in [
            "catalog-private-sentinel",
            "title-private-sentinel",
            "9876-5432",
            "expired-cursor-sentinel",
            "http-500-cursor-sentinel",
            "10.1000/private-doi-sentinel",
            "10.1000/private-retraction-sentinel",
            "private-article-title-sentinel",
            "private-update-source-sentinel",
            "fixture-response-body-sentinel",
            "fixture-transport-sentinel",
            "private@example.invalid",
            "https://api.crossref.org",
        ] {
            assert!(!logs.text().contains(private_value));
        }
    }

    #[test]
    fn scholarly_checkpoint_rejects_overflow_and_repeated_openalex_cursor() {
        let overflow = next_scholarly_source(
            &ScholarlySourceCheckpoint::Crossref {
                issn: "1234-5679".to_string(),
                cursor: Some("stateful".to_string()),
                page_index: u64::MAX,
                cursor_refreshed_at_epoch_seconds: Some(1_000),
            },
            Some("stateful".to_string()),
            false,
            Some(1_001),
        )
        .expect_err("Crossref page index should not wrap");
        assert_eq!(overflow.kind(), ProviderErrorKind::InvalidResponse);
        assert_eq!(
            overflow.to_string(),
            "scholarly Crossref checkpoint page index overflowed"
        );

        let repeated_openalex = next_scholarly_source(
            &ScholarlySourceCheckpoint::OpenAlex {
                source_id: "S1".to_string(),
                cursor: Some("fixture-page-1".to_string()),
            },
            Some("fixture-page-1".to_string()),
            false,
            None,
        )
        .expect_err("OpenAlex cursor should advance textually");
        assert_eq!(repeated_openalex.kind(), ProviderErrorKind::InvalidResponse);
        assert_eq!(
            repeated_openalex.to_string(),
            "scholarly provider returned a repeated cursor"
        );
    }

    #[test]
    fn crossref_checkpoint_stays_within_the_provider_contract_limit() {
        let cursor = "c".repeat(4_096);
        let source = next_scholarly_source(
            &ScholarlySourceCheckpoint::Crossref {
                issn: "1234-5679".to_string(),
                cursor: Some("previous".to_string()),
                page_index: 22,
                cursor_refreshed_at_epoch_seconds: Some(1_000),
            },
            Some(cursor),
            false,
            Some(1_001),
        )
        .expect("Crossref source should advance")
        .expect("non-terminal page should have a source");
        let checkpoint = encode_scholarly_checkpoint(&ScholarlyCheckpoint {
            version: SCHOLARLY_CHECKPOINT_VERSION,
            window: bootstrap_scholarly_window(),
            source,
        })
        .expect("Crossref checkpoint should encode");

        assert!(checkpoint.len() < 65_536);
    }

    #[test]
    fn openalex_checkpoint_resume_is_unchanged() {
        let registration = scholarly_index_registration(
            FixtureScholarlyTransport::new(ScholarlyFixtureData {
                openalex_source_work_pages: vec![
                    vec![json!({"display_name": "Ignored first page"})],
                    vec![json!({
                        "doi": "https://doi.org/10.1000/openalex-resume",
                        "display_name": "OpenAlex resumed article",
                        "publication_year": 2026,
                        "publication_date": "2026-07-19"
                    })],
                    vec![json!({"display_name": "Later page"})],
                ],
                ..ScholarlyFixtureData::default()
            }),
            false,
        )
        .expect("Scholarly registration should pass");
        let checkpoint = encode_scholarly_checkpoint(&ScholarlyCheckpoint {
            version: SCHOLARLY_CHECKPOINT_VERSION,
            window: bootstrap_scholarly_window(),
            source: ScholarlySourceCheckpoint::OpenAlex {
                source_id: "S1".to_string(),
                cursor: Some("fixture-page-1".to_string()),
            },
        })
        .expect("OpenAlex checkpoint should encode");
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&catalog(), fetch_context(Some(&checkpoint)))
            .expect("OpenAlex checkpoint should resume");

        assert_eq!(batch.articles.len(), 1);
        assert_eq!(
            batch.articles[0].doi.as_deref(),
            Some("10.1000/openalex-resume")
        );
        assert!(matches!(
            serde_json::from_str::<ScholarlyCheckpoint>(
                batch_checkpoint(&batch).expect("resumed OpenAlex page should continue")
            )
            .expect("OpenAlex checkpoint should decode"),
            ScholarlyCheckpoint {
                source: ScholarlySourceCheckpoint::OpenAlex { .. },
                ..
            }
        ));
    }

    fn run_cursor_recovery_pressure_instance() -> [usize; 7] {
        const CASE_COUNT: usize = 200;
        let mut responses = Vec::with_capacity(300);
        for case_index in 0..CASE_COUNT {
            match case_index % 4 {
                0 | 1 => responses.push(recovery_page(None)),
                2 => {
                    responses.push(CrossrefFixtureResponse::HttpStatus(500));
                    responses.push(recovery_page(None));
                }
                3 => {
                    responses.push(CrossrefFixtureResponse::HttpStatus(500));
                    responses.push(CrossrefFixtureResponse::HttpStatus(500));
                }
                _ => unreachable!("modulo four should stay bounded"),
            }
        }
        let transport = CursorRecoveryTransport::new(responses);
        let mut client = ScholarlyClient::new(transport, false);
        let mut successes = 0;
        let mut failures = 0;
        let mut restart_count = 0;
        let mut clock = || Ok(1_000);
        let mut restart = |_: &'static str, _: u64| restart_count += 1;
        for case_index in 0..CASE_COUNT {
            let refreshed_at = if case_index % 4 == 0 { 700 } else { 900 };
            let checkpoint = crossref_checkpoint(
                &format!("pressure-cursor-{case_index}"),
                case_index as u64,
                Some(refreshed_at),
            );
            match fetch_scholarly_batch_with_clock_and_restart(
                &mut client,
                &catalog(),
                Some(&checkpoint),
                false,
                &mut clock,
                &mut restart,
            ) {
                Ok(batch) => {
                    assert!(batch_is_complete(&batch));
                    successes += 1;
                }
                Err(error) => {
                    assert_eq!(error.kind(), ProviderErrorKind::TemporarilyUnavailable);
                    failures += 1;
                }
            }
        }
        let transport = client.into_transport();
        let request_count = transport.requested_cursors.len();
        let cursor_request_count = transport
            .requested_cursors
            .iter()
            .filter(|cursor| cursor.is_some())
            .count();
        let fresh_request_count = request_count - cursor_request_count;
        [
            CASE_COUNT,
            successes,
            failures,
            request_count,
            restart_count,
            cursor_request_count,
            fresh_request_count,
        ]
    }

    #[test]
    fn crossref_cursor_recovery_pressure_is_bounded_across_three_instances() {
        let handles = (0..3)
            .map(|_| thread::spawn(run_cursor_recovery_pressure_instance))
            .collect::<Vec<_>>();
        let mut totals = [0_usize; 7];
        for handle in handles {
            let result = handle.join().expect("pressure instance should not panic");
            for (total, value) in totals.iter_mut().zip(result) {
                *total += value;
            }
        }

        assert_eq!(totals[0], 600);
        assert_eq!(totals[1], 450);
        assert_eq!(totals[2], 150);
        assert_eq!(totals[3], 900);
        assert_eq!(totals[4], 450);
        assert_eq!(totals[5], 450);
        assert_eq!(totals[6], 450);
    }

    #[test]
    fn scholarly_registration_returns_openalex_fallback_directly() {
        let registration = scholarly_index_registration(
            FixtureScholarlyTransport::new(ScholarlyFixtureData {
                crossref_status: Some(404),
                openalex_source_by_issns: Some(json!({
                    "id": "https://openalex.org/S1",
                    "display_name": "Canonical Journal",
                    "issn_l": "1234-5679",
                    "issn": ["1234-5679"]
                })),
                openalex_source_works: vec![json!({
                    "doi": "https://doi.org/10.1000/openalex",
                    "display_name": "OpenAlex Article",
                    "publication_year": 2026,
                    "publication_date": "2026-07-18",
                    "biblio": {"volume": "1", "issue": "2", "first_page": "9", "last_page": "12"}
                })],
                ..ScholarlyFixtureData::default()
            }),
            false,
        )
        .expect("Scholarly registration should pass");
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&catalog(), fetch_context(None))
            .expect("OpenAlex fallback should fetch");

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 1);
        assert_eq!(batch.articles[0].doi.as_deref(), Some("10.1000/openalex"));
    }

    #[test]
    fn scholarly_registration_falls_back_to_openalex_after_empty_crossref_page() {
        let registration = scholarly_index_registration(
            FixtureScholarlyTransport::new(ScholarlyFixtureData {
                openalex_source_by_issns: Some(json!({
                    "id": "https://openalex.org/S1",
                    "display_name": "Canonical Journal",
                    "issn_l": "1234-5679",
                    "issn": ["1234-5679"]
                })),
                openalex_source_works: vec![json!({
                    "doi": "https://doi.org/10.1000/openalex-empty-crossref",
                    "display_name": "OpenAlex Article After Empty Crossref",
                    "type": "book-chapter",
                    "publication_year": 2026,
                    "publication_date": "2026-08-01",
                    "biblio": {"volume": "2", "issue": "3", "first_page": "1", "last_page": "8"}
                })],
                ..ScholarlyFixtureData::default()
            }),
            false,
        )
        .expect("Scholarly registration should pass");
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&catalog(), fetch_context(None))
            .expect("empty Crossref page should fall back to OpenAlex");

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 1);
        assert_eq!(
            batch.articles[0].doi.as_deref(),
            Some("10.1000/openalex-empty-crossref")
        );
    }

    #[test]
    fn cnki_registration_keeps_transport_handles_inside_the_adapter() {
        let fixture = CnkiFixtureData {
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
            fail_endpoint: None,
        };
        let registration = cnki_oversea_index_registration(FixtureCnkiTransport::new(fixture))
            .expect("CNKI registration should pass");
        let mut cnki_catalog = catalog();
        cnki_catalog.title = "CNKI Test Journal".to_string();
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&cnki_catalog, fetch_context(None))
            .expect("CNKI fixture should fetch");

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 1);
        let serialized = serde_json::to_string(&batch).expect("batch should serialize");
        assert!(!serialized.contains("CNKI202601001"));
        assert!(!serialized.contains("/kcms"));
        assert!(!serialized.contains("http"));
    }

    #[test]
    fn cnki_oversea_all_modes_full_scan_and_return_no_incremental_anchor() {
        for mode in [
            IndexSyncMode::Bootstrap,
            IndexSyncMode::Incremental,
            IndexSyncMode::FullRescan,
        ] {
            let registration =
                cnki_oversea_index_registration(FixtureCnkiTransport::new(overseas_cnki_fixture()))
                    .expect("overseas registration");
            let mut cnki_catalog = catalog();
            cnki_catalog.title = "CNKI Test Journal".to_string();
            let batch = registration
                .index_content()
                .expect("overseas provider")
                .fetch(
                    &cnki_catalog,
                    sync_context(mode, Some("opaque-ignored-anchor"), None),
                )
                .expect("overseas full scan");

            assert!(batch_is_complete(&batch));
            assert_eq!(batch_anchor(&batch), None);
            assert_eq!(batch.articles.len(), 1);
        }

        let registration =
            cnki_oversea_index_registration(FixtureCnkiTransport::new(overseas_cnki_fixture()))
                .expect("overseas traversal rejection registration");
        let mut cnki_catalog = catalog();
        cnki_catalog.title = "CNKI Test Journal".to_string();
        let error = registration
            .index_content()
            .expect("overseas provider")
            .fetch(
                &cnki_catalog,
                sync_context(
                    IndexSyncMode::Incremental,
                    Some("opaque-ignored-anchor"),
                    Some("unsupported-traversal"),
                ),
            )
            .expect_err("overseas traversal should be rejected");
        assert_eq!(error.kind(), ProviderErrorKind::InvalidResponse);
    }
    #[test]
    fn domestic_cnki_journal_locators_keep_all_catalog_and_article_identities() {
        let catalog = JournalCatalogEntry {
            catalog_id: "domestic".to_string(),
            catalog_aliases: Vec::new(),
            title: "Canonical title".to_string(),
            issn: Some("1002-9621".to_string()),
            eissn: Some("2049-3630".to_string()),
            all_issns: vec![
                "1002-9621".to_string(),
                "2049-3630".to_string(),
                "1234-5679".to_string(),
            ],
            title_aliases: vec!["Alias title".to_string(), " canonical title ".to_string()],
            area: None,
            rankings: JournalRankings::default(),
        };

        let catalog_locator = super::domestic_journal_locator_from_catalog(&catalog);
        assert_eq!(catalog_locator.titles(), ["Canonical title", "Alias title"]);
        assert_eq!(
            catalog_locator.issns(),
            ["1002-9621", "2049-3630", "1234-5679"]
        );

        let mut article = article_locator("Article", "Canonical title");
        article.journal_issns = vec!["2049-3630".to_string(), "1002-9621".to_string()];
        let article_locator = super::domestic_journal_locator_from_article(&article);
        assert_eq!(article_locator.titles(), ["Canonical title"]);
        assert_eq!(article_locator.issns(), ["2049-3630", "1002-9621"]);
    }

    #[test]
    fn domestic_cnki_declares_index_and_abstract_without_fulltext() {
        let fixture = DomesticCnkiFixtureData {
            journal_detail_html: r#"
                <html><head><title>世界经济 - 中国知网</title></head>
                <body>
                  <input id="pykm" type="hidden" value="SJJJ"/>
                  <input id="pCode" type="hidden" value="CJFD,CCJD"/>
                  <input type="hidden" id="shareChName" name="shareChName" value="世界经济"/>
                  <span>ISSN：1002-9621</span>
                </body></html>
            "#
            .to_string(),
            year_issues_html: r#"
                <div id="YearIssueTree">
                  <a id="yq202512" onclick="JournalDetail.BindIssueClick(this)" value="opaque-issue-token">No.12</a>
                  <a id="yq202511" onclick="JournalDetail.BindIssueClick(this)" value="opaque-issue-token-11">No.11</a>
                </div>
            "#
            .to_string(),
            issue_article_pages: BTreeMap::from([
                (
                    "202512".to_string(),
                    vec![r#"
                <dt class="tit">Articles</dt>
                <dd class="row clearfix">
                  <span class="name">
                    <a target="_blank"
                       href="https://kns.cnki.net/kcms2/article/abstract?v=TOKEN&amp;uniplatform=NZKPT&amp;language=CHS">
                      建立互利共赢的标准化合作伙伴关系
                    </a>
                    <b name="encrypt" id="SJJJ202512002"></b>
                  </span>
                  <span class="author" title="侯俊军;丁琪琪;">侯俊军;丁琪琪;</span>
                  <span class="company" title="3-31">3-31</span>
                </dd>
                <input id="articleCount" value="1">
            "#
                    .to_string()],
                ),
                (
                    "202511".to_string(),
                    vec![r#"
                <dt class="tit">Articles</dt>
                <dd class="row clearfix">
                  <span class="name">
                    <a target="_blank"
                       href="https://kns.cnki.net/kcms2/article/abstract?v=TOKEN2&amp;uniplatform=NZKPT&amp;language=CHS">
                      第二期文章
                    </a>
                    <b name="encrypt" id="SJJJ202511001"></b>
                  </span>
                  <span class="author" title="作者;">作者;</span>
                  <span class="company" title="1-2">1-2</span>
                </dd>
                <input id="articleCount" value="1">
            "#
                    .to_string()],
                ),
            ]),
            article_detail_html: BTreeMap::from([
                (
                    "SJJJ202512002".to_string(),
                    r#"
                <html><head><title>建立互利共赢的标准化合作伙伴关系 - 中国知网</title></head>
                <body>
                  <input type="hidden" id="param-dbcode" value="CJFQ">
                  <input type="hidden" id="param-dbname" value="CJFDLAST2026">
                  <input type="hidden" id="param-filename" value="SJJJ202512002">
                  <h1 class="title">建立互利共赢的标准化合作伙伴关系</h1>
                  <input id="abstract_text" type="hidden" value="摘要正文样本"/>
                  <span class="rowtit">摘要：</span>
                  <span id="ChDivSummary" name="ChDivSummary" class="abstract-text">摘要正文样本</span>
                  <span class="rowtit">DOI：</span><p>10.1000/domestic.sample</p>
                </body></html>
            "#
                    .to_string(),
                ),
                (
                    "SJJJ202511001".to_string(),
                    r#"
                <html><head><title>第二期文章 - 中国知网</title></head>
                <body>
                  <input type="hidden" id="param-filename" value="SJJJ202511001">
                  <h1 class="title">第二期文章</h1>
                  <input id="abstract_text" type="hidden" value="第二期摘要"/>
                </body></html>
            "#
                    .to_string(),
                ),
            ]),
            ..DomesticCnkiFixtureData::default()
        };

        let index = cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture.clone()))
            .expect("domestic index registration");
        assert!(index.article_full_text().is_none());
        assert!(index.article_abstract().is_none());
        assert!(index.index_content().is_some());
        assert_eq!(index.descriptor().name, CNKI_PROVIDER_NAME);

        let catalog = JournalCatalogEntry {
            catalog_id: "sjjj".to_string(),
            catalog_aliases: Vec::new(),
            title: "World Economy".to_string(),
            issn: Some("2049-3630".to_string()),
            eissn: Some("1002-9621".to_string()),
            all_issns: vec!["2049-3630".to_string(), "1002-9621".to_string()],
            title_aliases: vec!["世界经济".to_string()],
            area: None,
            rankings: JournalRankings::default(),
        };
        let first = index
            .index_content()
            .expect("index")
            .fetch(&catalog, fetch_context(None))
            .expect("first batch");
        assert!(!batch_is_complete(&first));
        assert!(batch_checkpoint(&first).is_some());
        assert_eq!(first.articles.len(), 1);
        assert_eq!(first.articles[0].title, "建立互利共赢的标准化合作伙伴关系");
        let checkpoint = batch_checkpoint(&first).unwrap();
        assert!(!checkpoint.to_ascii_lowercase().contains("captcha"));
        assert!(!checkpoint.contains("secretKey"));
        let second = index
            .index_content()
            .expect("index")
            .fetch(&catalog, fetch_context(Some(checkpoint)))
            .expect("second batch");
        assert!(batch_is_complete(&second));
        assert!(batch_checkpoint(&second).is_none());
        assert_eq!(second.articles.len(), 1);
        assert_eq!(second.articles[0].title, "第二期文章");

        let access = cnki_access_registration(FixtureDomesticCnkiTransport::new(fixture))
            .expect("domestic access registration");
        assert!(access.index_content().is_none());
        assert!(access.article_full_text().is_none());
        assert_eq!(
            access.descriptor().allowed_redirect_hosts,
            DOMESTIC_CNKI_REDIRECT_HOSTS
        );
        let mut locator =
            article_locator("建立互利共赢的标准化合作伙伴关系", "Provider title variant");
        locator.journal_issns = vec!["2049-3630".to_string(), "1002-9621".to_string()];
        locator.publication_year = Some(2025);
        locator.issue_number = Some("12".to_string());
        locator.doi = Some("10.1000/domestic.sample".to_string());
        let redirect = access
            .article_abstract()
            .expect("abstract")
            .resolve_abstract(&locator, ArticleAccessContext::default())
            .expect("abstract resolve");
        assert!(redirect.location.starts_with("https://kns.cnki.net/"));
        assert!(!redirect.location.contains("oversea.cnki.net"));

        let capabilities = built_in_provider_capabilities();
        let domestic = capabilities
            .iter()
            .find(|item| item.name == CNKI_PROVIDER_NAME)
            .expect("domestic capability");
        assert!(domestic.index_content);
        assert!(domestic.article_abstract);
        assert!(!domestic.article_full_text);
    }

    #[test]
    fn domestic_cnki_rejects_captcha_shaped_checkpoint() {
        let registration = cnki_index_registration(FixtureDomesticCnkiTransport::new(
            DomesticCnkiFixtureData::default(),
        ))
        .expect("registration");
        let error = registration
            .index_content()
            .expect("index")
            .fetch(
                &catalog(),
                fetch_context(Some(
                    r#"{"issue_index":0,"article_index":0,"captchaId":"x"}"#,
                )),
            )
            .expect_err("captcha checkpoint");
        assert_eq!(error.kind(), ProviderErrorKind::InvalidResponse);
    }

    #[test]
    fn domestic_cnki_traverses_all_pages_with_stable_checkpoint() {
        let first_issue_articles = (0..12)
            .map(|index| {
                (
                    format!("SJJJ202512{index:03}"),
                    format!("Paged article {index}"),
                )
            })
            .collect::<Vec<_>>();
        let first_issue_pages = vec![
            first_issue_articles[..10].to_vec(),
            first_issue_articles[10..].to_vec(),
        ];
        let second_issue_pages = vec![vec![(
            "SJJJ202511001".to_string(),
            "Following issue article".to_string(),
        )]];
        let fixture = domestic_paged_fixture(vec![
            ("202512".to_string(), first_issue_pages.clone()),
            ("202511".to_string(), second_issue_pages.clone()),
        ]);
        let registration = cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture))
            .expect("domestic registration");
        let index = registration.index_content().expect("index provider");
        let catalog = domestic_test_catalog();

        let first = index
            .fetch(&catalog, fetch_context(None))
            .expect("first page");
        assert_eq!(first.articles.len(), 10);
        assert!(!batch_is_complete(&first));
        let first_checkpoint = batch_checkpoint(&first)
            .expect("page checkpoint")
            .to_string();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&first_checkpoint).expect("checkpoint JSON"),
            json!({
                "version": 2,
                "base_anchor_issue_id": null,
                "candidate_head_issue_id": "202512",
                "current_issue_id": "202512",
                "page_index": 1
            })
        );
        assert!(!first_checkpoint.contains("issue_index"));
        assert!(!first_checkpoint.contains("article_index"));

        let reordered_fixture = domestic_paged_fixture(vec![
            ("202513".to_string(), vec![Vec::new()]),
            ("202512".to_string(), first_issue_pages),
            ("202511".to_string(), second_issue_pages),
        ]);
        let reordered =
            cnki_index_registration(FixtureDomesticCnkiTransport::new(reordered_fixture))
                .expect("reordered registration");
        let reordered_index = reordered.index_content().expect("reordered index");
        let resumed = reordered_index
            .fetch(&catalog, fetch_context(Some(&first_checkpoint)))
            .expect("stable resume");
        assert_eq!(resumed.articles.len(), 2);
        assert_eq!(resumed.articles[0].title, "Paged article 10");
        let next_issue_checkpoint = batch_checkpoint(&resumed)
            .expect("next issue checkpoint")
            .to_string();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&next_issue_checkpoint)
                .expect("next checkpoint JSON"),
            json!({
                "version": 2,
                "base_anchor_issue_id": null,
                "candidate_head_issue_id": "202512",
                "current_issue_id": "202511",
                "page_index": 0
            })
        );
        let final_batch = reordered_index
            .fetch(&catalog, fetch_context(Some(&next_issue_checkpoint)))
            .expect("following issue");
        assert!(batch_is_complete(&final_batch));
        assert_eq!(final_batch.articles.len(), 1);
        assert_eq!(final_batch.articles[0].title, "Following issue article");

        let missing = cnki_index_registration(FixtureDomesticCnkiTransport::new(
            domestic_paged_fixture(vec![(
                "202511".to_string(),
                vec![vec![(
                    "SJJJ202511001".to_string(),
                    "Following issue article".to_string(),
                )]],
            )]),
        ))
        .expect("missing issue registration");
        let error = missing
            .index_content()
            .expect("missing issue index")
            .fetch(&catalog, fetch_context(Some(&first_checkpoint)))
            .expect_err("missing checkpoint issue should fail");
        assert_eq!(error.kind(), ProviderErrorKind::InvalidResponse);
        assert!(error.to_string().contains("reset"));
    }

    #[test]
    fn domestic_cnki_incremental_covers_every_base_page_and_stops_before_older_issues() {
        let base_entries = (0..10)
            .map(|index| {
                (
                    format!("BASE{index:02}"),
                    format!("Boundary article {index}"),
                )
            })
            .collect::<Vec<_>>();
        let mut fixture = domestic_paged_fixture(vec![
            (
                "202601".to_string(),
                vec![vec![("HEAD".to_string(), "Newest article".to_string())]],
            ),
            ("202512".to_string(), vec![base_entries, Vec::new()]),
            (
                "202511".to_string(),
                vec![vec![("OLDER".to_string(), "Older article".to_string())]],
            ),
        ]);
        fixture.issue_article_pages.remove("202511");
        let registration = cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture))
            .expect("incremental registration");
        let anchor = json!({"version": 1, "year_issue_id": "202512"}).to_string();
        let batches = fetch_all_batches(
            registration
                .index_content()
                .expect("incremental provider")
                .as_ref(),
            &domestic_test_catalog(),
            IndexSyncMode::Incremental,
            Some(&anchor),
        );

        assert_eq!(batches.len(), 3);
        for batch in &batches {
            if let Some(checkpoint) = batch_checkpoint(batch) {
                assert_domestic_state_is_safe(checkpoint);
            }
            if let Some(anchor) = batch_anchor(batch) {
                assert_domestic_state_is_safe(anchor);
            }
        }
        assert!(batches.last().expect("final batch").articles.is_empty());
        let titles = batches
            .iter()
            .flat_map(|batch| batch.articles.iter().map(|article| article.title.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(titles.len(), 11);
        assert_eq!(titles[0], "Newest article");
        assert!(!titles.contains(&"Older article"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                batch_anchor(batches.last().expect("final batch")).expect("next anchor")
            )
            .expect("anchor JSON"),
            json!({"version": 1, "year_issue_id": "202601"})
        );
        let boundary_terminal_checkpoint = batch_checkpoint(&batches[1])
            .expect("exact boundary page should require its empty terminal page");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(boundary_terminal_checkpoint)
                .expect("boundary checkpoint JSON"),
            json!({
                "version": 2,
                "base_anchor_issue_id": "202512",
                "candidate_head_issue_id": "202601",
                "current_issue_id": "202512",
                "page_index": 1
            })
        );
    }

    #[test]
    fn domestic_cnki_resume_ignores_a_newer_inserted_head_until_the_next_update() {
        let initial_fixture = domestic_paged_fixture(vec![
            (
                "202601".to_string(),
                vec![vec![("HEAD".to_string(), "Original head".to_string())]],
            ),
            (
                "202512".to_string(),
                vec![vec![("MIDDLE".to_string(), "Middle article".to_string())]],
            ),
            (
                "202511".to_string(),
                vec![vec![("BASE".to_string(), "Base article".to_string())]],
            ),
        ]);
        let initial = cnki_index_registration(FixtureDomesticCnkiTransport::new(initial_fixture))
            .expect("initial registration");
        let base_anchor = json!({"version": 1, "year_issue_id": "202511"}).to_string();
        let first = initial
            .index_content()
            .expect("initial provider")
            .fetch(
                &domestic_test_catalog(),
                sync_context(IndexSyncMode::Incremental, Some(&base_anchor), None),
            )
            .expect("initial head batch");
        assert_eq!(first.articles[0].title, "Original head");
        let mut checkpoint = batch_checkpoint(&first)
            .expect("head batch should continue")
            .to_string();

        let mut resumed_fixture = domestic_paged_fixture(vec![
            (
                "202602".to_string(),
                vec![vec![("NEW".to_string(), "Inserted head".to_string())]],
            ),
            (
                "202601".to_string(),
                vec![vec![("HEAD".to_string(), "Original head".to_string())]],
            ),
            (
                "202512".to_string(),
                vec![vec![("MIDDLE".to_string(), "Middle article".to_string())]],
            ),
            (
                "202511".to_string(),
                vec![vec![("BASE".to_string(), "Base article".to_string())]],
            ),
        ]);
        resumed_fixture.issue_article_pages.remove("202602");
        let resumed = cnki_index_registration(FixtureDomesticCnkiTransport::new(resumed_fixture))
            .expect("resumed registration");
        let provider = resumed.index_content().expect("resumed provider");
        let mut resumed_batches = Vec::new();
        loop {
            let batch = provider
                .fetch(
                    &domestic_test_catalog(),
                    sync_context(
                        IndexSyncMode::Incremental,
                        Some(&base_anchor),
                        Some(&checkpoint),
                    ),
                )
                .expect("resumed batch");
            if let Some(next) = batch_checkpoint(&batch) {
                checkpoint = next.to_string();
            }
            let is_complete = batch_is_complete(&batch);
            resumed_batches.push(batch);
            if is_complete {
                break;
            }
        }
        let resumed_titles = resumed_batches
            .iter()
            .flat_map(|batch| batch.articles.iter().map(|article| article.title.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(resumed_titles, vec!["Middle article", "Base article"]);
        let frozen_head_anchor = batch_anchor(resumed_batches.last().expect("final resumed batch"))
            .expect("frozen head anchor")
            .to_string();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&frozen_head_anchor)
                .expect("frozen anchor JSON"),
            json!({"version": 1, "year_issue_id": "202601"})
        );

        let mut next_fixture = domestic_paged_fixture(vec![
            (
                "202602".to_string(),
                vec![vec![("NEW".to_string(), "Inserted head".to_string())]],
            ),
            (
                "202601".to_string(),
                vec![vec![("HEAD".to_string(), "Original head".to_string())]],
            ),
            (
                "202512".to_string(),
                vec![vec![("MIDDLE".to_string(), "Middle article".to_string())]],
            ),
        ]);
        next_fixture.issue_article_pages.remove("202512");
        let next = cnki_index_registration(FixtureDomesticCnkiTransport::new(next_fixture))
            .expect("next registration");
        let next_batches = fetch_all_batches(
            next.index_content().expect("next provider").as_ref(),
            &domestic_test_catalog(),
            IndexSyncMode::Incremental,
            Some(&frozen_head_anchor),
        );
        let next_titles = next_batches
            .iter()
            .flat_map(|batch| batch.articles.iter().map(|article| article.title.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(next_titles, vec!["Inserted head", "Original head"]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                batch_anchor(next_batches.last().expect("final next batch"))
                    .expect("new head anchor")
            )
            .expect("new anchor JSON"),
            json!({"version": 1, "year_issue_id": "202602"})
        );
    }

    #[test]
    fn domestic_cnki_missing_base_falls_back_but_malformed_state_fails_closed() {
        let fixture = domestic_paged_fixture(vec![
            (
                "202601".to_string(),
                vec![vec![("HEAD".to_string(), "Head article".to_string())]],
            ),
            (
                "202512".to_string(),
                vec![vec![("OLD".to_string(), "Old article".to_string())]],
            ),
        ]);
        let missing_base = json!({"version": 1, "year_issue_id": "199901"}).to_string();
        let registration =
            cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture.clone()))
                .expect("fallback registration");
        let batches = fetch_all_batches(
            registration
                .index_content()
                .expect("fallback provider")
                .as_ref(),
            &domestic_test_catalog(),
            IndexSyncMode::Incremental,
            Some(&missing_base),
        );
        assert_eq!(
            batches
                .iter()
                .flat_map(|batch| batch.articles.iter())
                .map(|article| article.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Head article", "Old article"]
        );

        for malformed_anchor in [
            r#"{"version":2,"year_issue_id":"202512"}"#,
            r#"{"version":1,"year_issue_id":"https://secret.example"}"#,
        ] {
            let invalid =
                cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture.clone()))
                    .expect("invalid anchor registration");
            let error = invalid
                .index_content()
                .expect("invalid anchor provider")
                .fetch(
                    &domestic_test_catalog(),
                    sync_context(IndexSyncMode::Incremental, Some(malformed_anchor), None),
                )
                .expect_err("malformed anchor should fail");
            assert_eq!(error.kind(), ProviderErrorKind::InvalidResponse);
        }

        let valid_anchor = json!({"version": 1, "year_issue_id": "202512"}).to_string();
        let missing_current = json!({
            "version": 2,
            "base_anchor_issue_id": "202512",
            "candidate_head_issue_id": "202601",
            "current_issue_id": "202599",
            "page_index": 0
        })
        .to_string();
        let invalid = cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture))
            .expect("invalid checkpoint registration");
        let error = invalid
            .index_content()
            .expect("invalid checkpoint provider")
            .fetch(
                &domestic_test_catalog(),
                sync_context(
                    IndexSyncMode::Incremental,
                    Some(&valid_anchor),
                    Some(&missing_current),
                ),
            )
            .expect_err("missing active issue should fail");
        assert_eq!(error.kind(), ProviderErrorKind::InvalidResponse);
    }

    #[test]
    fn domestic_cnki_full_rescan_ignores_the_committed_anchor_boundary() {
        let fixture = domestic_paged_fixture(vec![
            (
                "202601".to_string(),
                vec![vec![("HEAD".to_string(), "Head article".to_string())]],
            ),
            (
                "202512".to_string(),
                vec![vec![("OLD".to_string(), "Historical article".to_string())]],
            ),
        ]);
        let anchor = json!({"version": 1, "year_issue_id": "202601"}).to_string();
        let registration = cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture))
            .expect("full-rescan registration");
        let batches = fetch_all_batches(
            registration
                .index_content()
                .expect("full-rescan provider")
                .as_ref(),
            &domestic_test_catalog(),
            IndexSyncMode::FullRescan,
            Some(&anchor),
        );

        assert_eq!(
            batches
                .iter()
                .flat_map(|batch| batch.articles.iter())
                .map(|article| article.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Head article", "Historical article"]
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                batch_anchor(batches.last().expect("final batch")).expect("next anchor")
            )
            .expect("anchor JSON"),
            json!({"version": 1, "year_issue_id": "202601"})
        );
    }

    #[test]
    fn domestic_cnki_reuses_journal_metadata_until_completion() {
        let entries = (0..11)
            .map(|index| {
                (
                    format!("CACHE{index:02}"),
                    format!("Cached metadata article {index}"),
                )
            })
            .collect::<Vec<_>>();
        let journal_resolution_count = Arc::new(AtomicUsize::new(0));
        let issue_tree_count = Arc::new(AtomicUsize::new(0));
        let transport = MetadataCountingDomesticTransport {
            inner: FixtureDomesticCnkiTransport::new(domestic_paged_fixture(vec![(
                "202601".to_string(),
                vec![entries[..10].to_vec(), entries[10..].to_vec()],
            )])),
            journal_resolution_count: Arc::clone(&journal_resolution_count),
            issue_tree_count: Arc::clone(&issue_tree_count),
        };
        let registration = cnki_index_registration(transport).expect("metadata cache registration");
        let index = registration
            .index_content()
            .expect("metadata cache provider");
        let catalog = domestic_test_catalog();

        let first = index
            .fetch(&catalog, fetch_context(None))
            .expect("first cached page");
        let checkpoint = into_batch_checkpoint(first).expect("cached page checkpoint");
        let second = index
            .fetch(&catalog, fetch_context(Some(&checkpoint)))
            .expect("second cached page");

        assert!(batch_is_complete(&second));
        assert_eq!(journal_resolution_count.load(Ordering::SeqCst), 1);
        assert_eq!(issue_tree_count.load(Ordering::SeqCst), 1);

        index
            .fetch(&catalog, fetch_context(None))
            .expect("completed journal should load a fresh snapshot");
        assert_eq!(journal_resolution_count.load(Ordering::SeqCst), 2);
        assert_eq!(issue_tree_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn domestic_cnki_exact_multiple_requires_empty_terminal_page() {
        let entries = (0..10)
            .map(|index| {
                (
                    format!("EXACT{index:02}"),
                    format!("Exact page article {index}"),
                )
            })
            .collect::<Vec<_>>();
        let registration = cnki_index_registration(FixtureDomesticCnkiTransport::new(
            domestic_paged_fixture(vec![("202512".to_string(), vec![entries, Vec::new()])]),
        ))
        .expect("exact-multiple registration");
        let index = registration.index_content().expect("exact-multiple index");
        let catalog = domestic_test_catalog();

        let articles = index
            .fetch(&catalog, fetch_context(None))
            .expect("full page");
        assert_eq!(articles.articles.len(), 10);
        assert!(!batch_is_complete(&articles));
        let checkpoint = into_batch_checkpoint(articles).expect("terminal checkpoint");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&checkpoint).expect("checkpoint JSON"),
            json!({
                "version": 2,
                "base_anchor_issue_id": null,
                "candidate_head_issue_id": "202512",
                "current_issue_id": "202512",
                "page_index": 1
            })
        );

        let terminal = index
            .fetch(&catalog, fetch_context(Some(&checkpoint)))
            .expect("validated empty terminal page");
        assert!(terminal.articles.is_empty());
        assert!(batch_is_complete(&terminal));
        assert!(batch_checkpoint(&terminal).is_none());
    }

    #[test]
    fn domestic_cnki_rebuilds_detail_workers_before_batch_replay() {
        let session_reset_count = Arc::new(AtomicUsize::new(0));
        let detail_attempt_count = Arc::new(AtomicUsize::new(0));
        let transport = BatchRecoveryDomesticTransport {
            inner: FixtureDomesticCnkiTransport::new(domestic_paged_fixture(vec![(
                "202512".to_string(),
                vec![vec![
                    (
                        "RECOVER1".to_string(),
                        "First recovered article".to_string(),
                    ),
                    (
                        "RECOVER2".to_string(),
                        "Second recovered article".to_string(),
                    ),
                ]],
            )])),
            is_session_stale: true,
            session_reset_count: Arc::clone(&session_reset_count),
            detail_attempt_count: Arc::clone(&detail_attempt_count),
        };
        let registration =
            cnki_index_registration_with_workers(transport, 2).expect("recovery registration");

        let batch = registration
            .index_content()
            .expect("recovery index")
            .fetch(&domestic_test_catalog(), fetch_context(None))
            .expect("transient batch should replay");

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 2);
        assert_eq!(batch.articles[0].title, "First recovered article");
        assert_eq!(batch.articles[1].title, "Second recovered article");
        assert_eq!(session_reset_count.load(Ordering::SeqCst), 1);
        assert_eq!(detail_attempt_count.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn domestic_cnki_skips_permanent_details_and_replays_temporary_page() {
        let entries = vec![
            ("DELETED".to_string(), "Deleted article".to_string()),
            ("NOT_FOUND".to_string(), "Not found article".to_string()),
            ("GONE".to_string(), "Gone article".to_string()),
            ("LATER".to_string(), "Later article".to_string()),
        ];
        let mut permanent_fixture =
            domestic_paged_fixture(vec![("202512".to_string(), vec![entries.clone()])]);
        permanent_fixture.article_detail_html.insert(
            "DELETED".to_string(),
            "<html><body>该文献不存在</body></html>".to_string(),
        );
        permanent_fixture
            .article_detail_status_codes
            .insert("NOT_FOUND".to_string(), 404);
        permanent_fixture
            .article_detail_status_codes
            .insert("GONE".to_string(), 410);
        let permanent =
            cnki_index_registration(FixtureDomesticCnkiTransport::new(permanent_fixture))
                .expect("permanent fixture registration");
        let batch = permanent
            .index_content()
            .expect("permanent index")
            .fetch(&domestic_test_catalog(), fetch_context(None))
            .expect("permanent misses should skip");
        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 1);
        assert_eq!(batch.articles[0].title, "Later article");

        let temporary_entries = vec![
            ("TEMP".to_string(), "Temporary article".to_string()),
            ("AFTER".to_string(), "After temporary article".to_string()),
        ];
        let mut temporary_fixture = domestic_paged_fixture(vec![(
            "202512".to_string(),
            vec![temporary_entries.clone()],
        )]);
        temporary_fixture
            .article_detail_status_codes
            .insert("TEMP".to_string(), 500);
        let temporary =
            cnki_index_registration(FixtureDomesticCnkiTransport::new(temporary_fixture))
                .expect("temporary fixture registration");
        let error = temporary
            .index_content()
            .expect("temporary index")
            .fetch(&domestic_test_catalog(), fetch_context(None))
            .expect_err("temporary failure should abort the page");
        assert_eq!(error.kind(), ProviderErrorKind::TemporarilyUnavailable);

        let replay = cnki_index_registration(FixtureDomesticCnkiTransport::new(
            domestic_paged_fixture(vec![("202512".to_string(), vec![temporary_entries])]),
        ))
        .expect("replay fixture registration");
        let replayed = replay
            .index_content()
            .expect("replay index")
            .fetch(&domestic_test_catalog(), fetch_context(None))
            .expect("page should replay from its original checkpoint");
        assert_eq!(replayed.articles.len(), 2);
    }

    #[test]
    fn domestic_cnki_filter_requires_missing_authors_and_doi() {
        let no_metadata = json!({});
        assert!(super::domestic_cnki_lacks_authors_and_doi(
            &json!({"title": "Concluding note", "section": "Articles"}),
            &no_metadata,
        ));
        assert!(!super::domestic_cnki_lacks_authors_and_doi(
            &json!({"title": "Reference book review 书评", "authors": "Reviewer;", "section": "书评"}),
            &no_metadata,
        ));
        assert!(!super::domestic_cnki_lacks_authors_and_doi(
            &json!({"title": "Research article"}),
            &json!({"authors": "Researcher;"}),
        ));
        assert!(!super::domestic_cnki_lacks_authors_and_doi(
            &json!({"title": "《世界经济》征稿启事"}),
            &json!({"doi": "10.1000/call-for-papers"}),
        ));
    }

    #[test]
    fn domestic_cnki_discards_only_articles_without_authors_or_doi() {
        let entries = vec![
            ("DROP".to_string(), "Concluding note".to_string()),
            ("CALL".to_string(), "《世界经济》征稿启事".to_string()),
            (
                "REVIEW".to_string(),
                "Reference book review 书评".to_string(),
            ),
        ];
        let mut fixture = domestic_paged_fixture(vec![("202601".to_string(), vec![entries])]);
        fixture.issue_article_pages.insert(
            "202601".to_string(),
            vec![r#"
                <dd class="row clearfix">
                  <a href="https://kns.cnki.net/kcms2/article/abstract?v=DROP">Concluding note</a>
                  <b name="encrypt" id="DROP"></b>
                </dd>
                <dd class="row clearfix">
                  <a href="https://kns.cnki.net/kcms2/article/abstract?v=CALL">《世界经济》征稿启事</a>
                  <b name="encrypt" id="CALL"></b>
                </dd>
                <dt class="tit">书评</dt>
                <dd class="row clearfix">
                  <a href="https://kns.cnki.net/kcms2/article/abstract?v=REVIEW">Reference book review 书评</a>
                  <b name="encrypt" id="REVIEW"></b>
                  <span class="author" title="Reviewer;">Reviewer;</span>
                </dd>
                <input id="articleCount" value="3">
            "#
            .to_string()],
        );
        fixture.article_detail_html.insert(
            "DROP".to_string(),
            domestic_test_article_detail_without_doi("DROP", "Concluding note"),
        );
        fixture.article_detail_html.insert(
            "REVIEW".to_string(),
            domestic_test_article_detail_without_doi("REVIEW", "Reference book review 书评"),
        );
        let registration = cnki_index_registration(FixtureDomesticCnkiTransport::new(fixture))
            .expect("filtered registration");

        let batch = registration
            .index_content()
            .expect("filtered index")
            .fetch(&domestic_test_catalog(), fetch_context(None))
            .expect("metadata rule should complete");

        assert!(batch_is_complete(&batch));
        assert_eq!(batch.articles.len(), 2);
        assert_eq!(batch.articles[0].title, "《世界经济》征稿启事");
        assert_eq!(batch.articles[1].title, "Reference book review 书评");
    }

    #[test]
    fn domestic_cnki_concurrency_pool_is_bounded_reused_and_ordered() {
        let first_entries = (0..10)
            .map(|index| (format!("FIRST{index}"), format!("First article {index}")))
            .collect::<Vec<_>>();
        let second_entries = (0..6)
            .map(|index| (format!("SECOND{index}"), format!("Second article {index}")))
            .collect::<Vec<_>>();
        let active_requests = Arc::new(AtomicUsize::new(0));
        let peak_requests = Arc::new(AtomicUsize::new(0));
        let clone_count = Arc::new(AtomicUsize::new(0));
        let drop_count = Arc::new(AtomicUsize::new(0));
        let transport = ConcurrentDomesticTransport {
            inner: FixtureDomesticCnkiTransport::new(domestic_paged_fixture(vec![(
                "202601".to_string(),
                vec![first_entries, second_entries],
            )])),
            active_requests,
            peak_requests: Arc::clone(&peak_requests),
            clone_count: Arc::clone(&clone_count),
            drop_count: Arc::clone(&drop_count),
        };
        let provider = DomesticCnkiIndexProvider::with_worker_count(transport, 3)
            .expect("parallel domestic provider");

        let first_batch = provider
            .fetch(&domestic_test_catalog(), fetch_context(None))
            .expect("parallel details should complete");
        let checkpoint =
            batch_checkpoint(&first_batch).expect("the first papers page should continue");
        let second_batch = provider
            .fetch(&domestic_test_catalog(), fetch_context(Some(checkpoint)))
            .expect("the reused pool should complete a second page");

        assert_eq!(peak_requests.load(Ordering::SeqCst), 3);
        assert_eq!(clone_count.load(Ordering::SeqCst), 3);
        assert_eq!(first_batch.articles.len(), 10);
        assert_eq!(first_batch.articles[0].title, "First article 0");
        assert_eq!(first_batch.articles[9].title, "First article 9");
        assert_eq!(second_batch.articles.len(), 6);
        assert_eq!(second_batch.articles[0].title, "Second article 0");
        assert_eq!(second_batch.articles[5].title, "Second article 5");
        let state = provider.state.lock().expect("provider state should lock");
        assert_eq!(state.detail_pool.workers.len(), 3);
        assert_eq!(state.detail_pool.peak_requests.load(Ordering::SeqCst), 3);
        assert_eq!(state.detail_pool.active_requests.load(Ordering::SeqCst), 0);
        drop(state);
        drop(provider);
        assert_eq!(drop_count.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn domestic_cnki_concurrency_rejects_invalid_direct_pool_sizes_before_cloning() {
        for worker_count in [0, DOMESTIC_CNKI_WORKER_COUNT_MAX + 1] {
            let clone_count = Arc::new(AtomicUsize::new(0));
            let drop_count = Arc::new(AtomicUsize::new(0));
            let transport = ConcurrentDomesticTransport {
                inner: FixtureDomesticCnkiTransport::new(domestic_paged_fixture(vec![])),
                active_requests: Arc::new(AtomicUsize::new(0)),
                peak_requests: Arc::new(AtomicUsize::new(0)),
                clone_count: Arc::clone(&clone_count),
                drop_count: Arc::clone(&drop_count),
            };
            let error = match DomesticCnkiIndexProvider::with_worker_count(transport, worker_count)
            {
                Ok(_) => panic!("invalid direct workers should fail"),
                Err(error) => error,
            };
            assert!(matches!(
                error,
                ProviderRegistryError::InvalidConfiguration { provider, .. }
                    if provider == CNKI_PROVIDER_NAME
            ));
            assert_eq!(clone_count.load(Ordering::SeqCst), 0);
            assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn domestic_cnki_abstract_traverses_later_pages() {
        let first_page = (0..10)
            .map(|index| (format!("FIRST{index:02}"), format!("Unrelated {index}")))
            .collect::<Vec<_>>();
        let second_page = vec![
            ("SECOND00".to_string(), "Other later article".to_string()),
            ("TARGET".to_string(), "Target later article".to_string()),
        ];
        let access = cnki_access_registration(FixtureDomesticCnkiTransport::new(
            domestic_paged_fixture(vec![("202512".to_string(), vec![first_page, second_page])]),
        ))
        .expect("domestic access registration");
        let mut locator = article_locator("Target later article", "世界经济");
        locator.journal_issns = vec!["1002-9621".to_string()];
        locator.publication_year = Some(2025);
        locator.issue_number = Some("12".to_string());

        let redirect = access
            .article_abstract()
            .expect("abstract provider")
            .resolve_abstract(&locator, ArticleAccessContext::default())
            .expect("later-page abstract should resolve");

        assert!(redirect.location.contains("TARGET"));
    }

    fn domestic_test_catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "sjjj".to_string(),
            catalog_aliases: Vec::new(),
            title: "世界经济".to_string(),
            issn: Some("1002-9621".to_string()),
            eissn: None,
            all_issns: vec!["1002-9621".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
    }

    fn overseas_cnki_fixture() -> CnkiFixtureData {
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
            fail_endpoint: None,
        }
    }

    type DomesticPagedFixtureInput = Vec<(String, Vec<Vec<(String, String)>>)>;

    fn domestic_paged_fixture(issues: DomesticPagedFixtureInput) -> DomesticCnkiFixtureData {
        let mut year_rows = String::new();
        let mut issue_article_pages = BTreeMap::new();
        let mut article_detail_html = BTreeMap::new();
        for (year_issue_id, pages) in issues {
            let issue_number = year_issue_id.get(4..).unwrap_or("0");
            year_rows.push_str(&format!(
                r#"<a id="yq{year_issue_id}" value="opaque-{year_issue_id}">No.{issue_number}</a>"#
            ));
            let page_html = pages
                .into_iter()
                .map(|entries| {
                    for (platform_id, title) in &entries {
                        article_detail_html.insert(
                            platform_id.clone(),
                            domestic_test_article_detail(platform_id, title),
                        );
                    }
                    domestic_test_papers_page(&entries)
                })
                .collect::<Vec<_>>();
            issue_article_pages.insert(year_issue_id, page_html);
        }
        DomesticCnkiFixtureData {
            journal_detail_html: r#"
                <html><head><title>世界经济 - 中国知网</title></head><body>
                  <input id="pykm" value="SJJJ"><input id="pCode" value="CJFD">
                  <input id="shareChName" value="世界经济"><span>ISSN：1002-9621</span>
                </body></html>
            "#
            .to_string(),
            year_issues_html: format!("<div id=\"YearIssueTree\">{year_rows}</div>"),
            issue_article_pages,
            article_detail_html,
            ..DomesticCnkiFixtureData::default()
        }
    }

    fn domestic_test_papers_page(entries: &[(String, String)]) -> String {
        let rows = entries
            .iter()
            .map(|(platform_id, title)| {
                format!(
                    r#"<dd class="row clearfix"><a href="https://kns.cnki.net/kcms2/article/abstract?v={platform_id}" title="{title}">{title}</a><b name="encrypt" id="{platform_id}"></b></dd>"#
                )
            })
            .collect::<String>();
        format!(
            "<dt class=\"tit\">Articles</dt>{rows}<input id=\"articleCount\" value=\"{}\">",
            entries.len()
        )
    }

    fn domestic_test_article_detail(platform_id: &str, title: &str) -> String {
        format!(
            r#"<html><head><title>{title} - 中国知网</title></head><body><input id="param-filename" value="{platform_id}"><h1 class="title">{title}</h1><input id="abstract_text" value="Abstract"><span class="rowtit">DOI：</span><p>10.1000/{platform_id}</p></body></html>"#
        )
    }

    fn domestic_test_article_detail_without_doi(platform_id: &str, title: &str) -> String {
        format!(
            r#"<html><head><title>{title} - 中国知网</title></head><body><input id="param-filename" value="{platform_id}"><h1 class="title">{title}</h1><input id="abstract_text" value="Abstract"></body></html>"#
        )
    }
}
