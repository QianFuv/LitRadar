//! Built-in canonical indexing and request-time article access provider adapters.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_domain::{
    normalize_bibliographic_label, normalize_bibliographic_text, normalize_contract_doi,
    normalize_contract_pmid, normalize_contract_text, ArticleAccessContext, ArticleAuthorDraft,
    ArticleDraft, ArticleLocator, ArticleRedirect, IssueDraft, JournalCatalogEntry, JournalDraft,
    ProviderBatch,
};
use litradar_provider::{
    ArticleAbstractProvider, ArticleDetailProvider, IndexContentProvider, ProviderCapabilities,
    ProviderDescriptor, ProviderError, ProviderErrorKind, ProviderImplementations,
    ProviderRegistration, ProviderRegistryError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CnkiClient, CnkiSourceError, CnkiTransport, ScholarlyClient, ScholarlyTransport, SourceAttempt,
    SourceError, SEMANTIC_SCHOLAR_BATCH_SIZE,
};

/// Stable runtime name for the built-in Scholarly indexing provider.
pub const SCHOLARLY_PROVIDER_NAME: &str = "scholarly";

/// Stable runtime name for the built-in CNKI indexing provider.
pub const CNKI_PROVIDER_NAME: &str = "cnki";

/// Exact HTTPS hosts emitted by the Scholarly online access provider.
pub const SCHOLARLY_REDIRECT_HOSTS: &[&str] = &["doi.org", "pubmed.ncbi.nlm.nih.gov"];

/// Exact HTTPS hosts emitted by the CNKI online access provider.
pub const CNKI_REDIRECT_HOSTS: &[&str] = &["oversea.cnki.net", "kns.cnki.net", "www.cnki.net"];

const SCHOLARLY_ENRICHMENT_BATCH_SIZE: usize = 100;
const CROSSREF_CURSOR_REUSE_SECONDS: u64 = 240;

/// Stateless Scholarly access provider that derives live DOI or PubMed destinations.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScholarlyArticleAccessProvider;

impl ArticleDetailProvider for ScholarlyArticleAccessProvider {
    fn resolve_detail(
        &self,
        article: &ArticleLocator,
        _context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        scholarly_article_redirect(article)
    }
}

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
        emit_source_attempt_summary(CNKI_PROVIDER_NAME, &client.drain_attempts());
        result
    }
}

impl<T> ArticleDetailProvider for CnkiArticleAccessProvider<T>
where
    T: CnkiTransport + Send,
{
    fn resolve_detail(
        &self,
        article: &ArticleLocator,
        _context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        self.resolve(article)
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
        checkpoint: Option<&str>,
    ) -> Result<ProviderBatch, ProviderError> {
        let mut client = self.client.lock().map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Internal,
                "scholarly provider state is unavailable",
            )
        })?;
        let result = fetch_scholarly_batch(
            &mut client,
            catalog,
            checkpoint,
            self.has_semantic_scholar_key,
        );
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
        checkpoint: Option<&str>,
    ) -> Result<ProviderBatch, ProviderError> {
        if checkpoint.is_some() {
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
        emit_source_attempt_summary(CNKI_PROVIDER_NAME, &client.drain_attempts());
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
pub fn cnki_index_registration<T>(
    transport: T,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: CnkiTransport + Send + 'static,
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
            index_content: Some(Arc::new(CnkiIndexProvider::new(transport))),
            ..ProviderImplementations::default()
        },
    )
}

/// Register Scholarly detail and abstract-page access capabilities.
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
                article_detail: true,
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: SCHOLARLY_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_detail: Some(provider.clone()),
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

/// Register CNKI detail and abstract-page access capabilities.
///
/// # Arguments
///
/// * `transport` - CNKI source transport used only for request-time resolution.
///
/// # Returns
///
/// Access-only CNKI registration.
pub fn cnki_access_registration<T>(
    transport: T,
) -> Result<ProviderRegistration, ProviderRegistryError>
where
    T: CnkiTransport + Send + 'static,
{
    let provider = Arc::new(CnkiArticleAccessProvider::new(transport));
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_detail: true,
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: CNKI_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_detail: Some(provider.clone()),
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum ScholarlyCheckpoint {
    Crossref {
        issn: String,
        cursor: String,
        #[serde(default)]
        page_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor_refreshed_at_epoch_seconds: Option<u64>,
    },
    OpenAlex {
        source_id: String,
        cursor: String,
    },
}

enum FirstScholarlyPage {
    Crossref {
        works: Vec<Value>,
        next_checkpoint: Option<String>,
    },
    OpenAlex {
        articles: Vec<ArticleDraft>,
        next_checkpoint: Option<String>,
    },
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
    checkpoint: Option<&str>,
    has_semantic_scholar_key: bool,
) -> Result<ProviderBatch, ProviderError>
where
    T: ScholarlyTransport,
{
    let mut clock = current_epoch_seconds;
    fetch_scholarly_batch_with_clock(
        client,
        catalog,
        checkpoint,
        has_semantic_scholar_key,
        &mut clock,
    )
}

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
    fetch_scholarly_batch_with_clock_and_restart(
        client,
        catalog,
        checkpoint,
        has_semantic_scholar_key,
        clock,
        &mut restart,
    )
}

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
    let (works, next_checkpoint) = if let Some(checkpoint) = checkpoint {
        let checkpoint = serde_json::from_str::<ScholarlyCheckpoint>(checkpoint).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "scholarly checkpoint is invalid",
            )
        })?;
        match checkpoint {
            ScholarlyCheckpoint::Crossref {
                issn,
                cursor,
                page_index,
                cursor_refreshed_at_epoch_seconds,
            } => {
                if !crossref_cursor_is_fresh(cursor_refreshed_at_epoch_seconds, clock()?) {
                    restart("expired_or_legacy", page_index);
                    return fetch_scholarly_batch_with_clock_and_restart(
                        client,
                        catalog,
                        None,
                        has_semantic_scholar_key,
                        clock,
                        restart,
                    );
                }
                let page = match client.fetch_journal_works_page(&issn, None, Some(&cursor)) {
                    Ok(page) => page,
                    Err(error) if is_crossref_cursor_http_500(&error) => {
                        restart("cursor_http_500", page_index);
                        return fetch_scholarly_batch_with_clock_and_restart(
                            client,
                            catalog,
                            None,
                            has_semantic_scholar_key,
                            clock,
                            restart,
                        );
                    }
                    Err(error) => return Err(map_scholarly_error(error)),
                };
                let cursor_refreshed_at_epoch_seconds = crossref_checkpoint_epoch(
                    page.next_cursor.as_ref(),
                    page.items.is_empty(),
                    clock,
                )?;
                let next = next_scholarly_checkpoint(
                    ScholarlyCheckpoint::Crossref {
                        issn,
                        cursor: cursor.clone(),
                        page_index,
                        cursor_refreshed_at_epoch_seconds,
                    },
                    page.next_cursor,
                    &cursor,
                    page.items.is_empty(),
                    cursor_refreshed_at_epoch_seconds,
                )?;
                (page.items, next)
            }
            ScholarlyCheckpoint::OpenAlex { source_id, cursor } => {
                let page = client
                    .fetch_openalex_works_by_source_page(&source_id, None, Some(&cursor))
                    .map_err(map_scholarly_error)?;
                let items = page
                    .items
                    .iter()
                    .filter_map(|work| openalex_article_draft(catalog, work))
                    .collect::<Vec<_>>();
                let next = next_scholarly_checkpoint(
                    ScholarlyCheckpoint::OpenAlex {
                        source_id,
                        cursor: cursor.clone(),
                    },
                    page.next_cursor,
                    &cursor,
                    items.is_empty(),
                    None,
                )?;
                return Ok(batch_from_articles(catalog, items, next));
            }
        }
    } else {
        match first_scholarly_page(client, catalog, clock)? {
            FirstScholarlyPage::Crossref {
                works,
                next_checkpoint,
            } => (works, next_checkpoint),
            FirstScholarlyPage::OpenAlex {
                articles,
                next_checkpoint,
            } => return Ok(batch_from_articles(catalog, articles, next_checkpoint)),
        }
    };

    let dois = works
        .iter()
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
    let articles = works
        .iter()
        .filter_map(|work| {
            let doi = normalize_contract_doi(json_text(work.get("DOI"))?.as_str());
            scholarly_article_draft(
                catalog,
                work,
                doi.as_ref().and_then(|value| openalex.get(value)),
                doi.as_ref().and_then(|value| semantic_scholar.get(value)),
            )
        })
        .collect::<Vec<_>>();
    Ok(batch_from_articles(catalog, articles, next_checkpoint))
}

fn first_scholarly_page<T>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    clock: &mut impl FnMut() -> Result<u64, ProviderError>,
) -> Result<FirstScholarlyPage, ProviderError>
where
    T: ScholarlyTransport,
{
    let issns = catalog_issns(catalog);
    for issn in &issns {
        match client.fetch_journal_works_page(issn, None, None) {
            Ok(page) => {
                let cursor_refreshed_at_epoch_seconds = crossref_checkpoint_epoch(
                    page.next_cursor.as_ref(),
                    page.items.is_empty(),
                    clock,
                )?;
                let next = next_scholarly_checkpoint(
                    ScholarlyCheckpoint::Crossref {
                        issn: issn.clone(),
                        cursor: String::new(),
                        page_index: 0,
                        cursor_refreshed_at_epoch_seconds,
                    },
                    page.next_cursor,
                    "",
                    page.items.is_empty(),
                    cursor_refreshed_at_epoch_seconds,
                )?;
                return Ok(FirstScholarlyPage::Crossref {
                    works: page.items,
                    next_checkpoint: next,
                });
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
        .fetch_openalex_works_by_source_page(&source_id, None, None)
        .map_err(map_scholarly_error)?;
    let articles = page
        .items
        .iter()
        .filter_map(|work| openalex_article_draft(catalog, work))
        .collect::<Vec<_>>();
    let next = next_scholarly_checkpoint(
        ScholarlyCheckpoint::OpenAlex {
            source_id,
            cursor: String::new(),
        },
        page.next_cursor,
        "",
        articles.is_empty(),
        None,
    )?;
    Ok(FirstScholarlyPage::OpenAlex {
        articles,
        next_checkpoint: next,
    })
}

fn next_scholarly_checkpoint(
    current: ScholarlyCheckpoint,
    next_cursor: Option<String>,
    previous_cursor: &str,
    is_empty: bool,
    cursor_refreshed_at_epoch_seconds: Option<u64>,
) -> Result<Option<String>, ProviderError> {
    let Some(next_cursor) = next_cursor.filter(|_| !is_empty) else {
        return Ok(None);
    };
    let checkpoint = match current {
        ScholarlyCheckpoint::Crossref {
            issn, page_index, ..
        } => ScholarlyCheckpoint::Crossref {
            issn,
            cursor: next_cursor,
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
        ScholarlyCheckpoint::OpenAlex { source_id, .. } => {
            if next_cursor == previous_cursor {
                return Err(ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "scholarly provider returned a repeated cursor",
                ));
            }
            ScholarlyCheckpoint::OpenAlex {
                source_id,
                cursor: next_cursor,
            }
        }
    };
    serde_json::to_string(&checkpoint).map(Some).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::Internal,
            "scholarly checkpoint could not be encoded",
        )
    })
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
        is_complete: true,
        next_checkpoint: None,
    })
}

fn batch_from_articles(
    catalog: &JournalCatalogEntry,
    articles: Vec<ArticleDraft>,
    next_checkpoint: Option<String>,
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
        is_complete: next_checkpoint.is_none(),
        next_checkpoint,
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
    article.date = article.date.as_deref().and_then(normalize_partial_date);
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

fn normalize_partial_date(value: &str) -> Option<String> {
    let value = normalize_contract_text(value)?;
    let prefix = if value.len() >= 10 && value.as_bytes().get(4) == Some(&b'-') {
        &value[..10]
    } else if value.len() >= 7 && value.as_bytes().get(4) == Some(&b'-') {
        &value[..7]
    } else if value.len() >= 4 {
        &value[..4]
    } else {
        return None;
    };
    let parts = prefix.split('-').collect::<Vec<_>>();
    if parts[0].len() != 4 || !parts[0].bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    if parts.get(1).is_some_and(|value| {
        value
            .parse::<u8>()
            .map_or(true, |month| !(1..=12).contains(&month))
    }) || parts.get(2).is_some_and(|value| {
        value
            .parse::<u8>()
            .map_or(true, |day| !(1..=31).contains(&day))
    }) {
        return None;
    }
    Some(prefix.to_string())
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
            let month = parts.get(1).and_then(Value::as_i64).unwrap_or(1);
            let day = parts.get(2).and_then(Value::as_i64).unwrap_or(1);
            return Some(format!("{year:04}-{month:02}-{day:02}"));
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
    use std::thread;

    use litradar_domain::{
        ArticleAccessContext, ArticleId, ArticleLocator, JournalCatalogEntry, JournalRankings,
        ProviderBatch, ProviderCapabilityKind,
    };
    use litradar_provider::{ProviderError, ProviderErrorKind, ProviderRegistry};
    use serde_json::json;

    use super::{
        cnki_access_registration, cnki_article_draft, cnki_index_registration, cnki_issue_draft,
        crossref_cursor_is_fresh, fetch_scholarly_batch_with_clock,
        fetch_scholarly_batch_with_clock_and_restart, next_scholarly_checkpoint,
        scholarly_access_registration, scholarly_article_draft, scholarly_index_registration,
        CnkiIndexProvider, ScholarlyCheckpoint, ScholarlyIndexProvider, CNKI_REDIRECT_HOSTS,
        CROSSREF_CURSOR_REUSE_SECONDS, SCHOLARLY_REDIRECT_HOSTS,
    };
    use crate::scholarly::test_support::CapturedLogs;
    use crate::{
        CnkiFixtureData, FixtureCnkiTransport, FixtureScholarlyTransport, ScholarlyClient,
        ScholarlyFixtureData, ScholarlyRequest, ScholarlyRequestKind, ScholarlyTransport,
        SourceAttempt, SourceError,
    };

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
    }

    impl CursorRecoveryTransport {
        fn new(responses: Vec<CrossrefFixtureResponse>) -> Self {
            Self {
                responses: responses.into(),
                attempts: Vec::new(),
                requested_cursors: Vec::new(),
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
                ScholarlyRequestKind::CrossrefJournalWorks { cursor, .. } => {
                    self.requested_cursors.push(cursor.clone());
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
        serde_json::to_string(&ScholarlyCheckpoint::Crossref {
            issn: "1234-5679".to_string(),
            cursor: cursor.to_string(),
            page_index,
            cursor_refreshed_at_epoch_seconds,
        })
        .expect("Crossref checkpoint should encode")
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
        let cnki = cnki_index_registration(FixtureCnkiTransport::new(CnkiFixtureData::default()))
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
            .providers_with(ProviderCapabilityKind::ArticleDetail)
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
            .article_detail()
            .expect("detail capability should exist")
            .resolve_detail(
                &ArticleLocator {
                    doi: Some("10.1000/article".to_string()),
                    ..article_locator("Article", "Canonical Journal")
                },
                ArticleAccessContext::default(),
            )
            .expect("Scholarly detail should resolve online");
        assert_eq!(
            scholarly_redirect.location,
            "https://doi.org/10.1000/article"
        );
        assert!(scholarly.article_abstract().is_some());

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
        let cnki = cnki_access_registration(FixtureCnkiTransport::new(fixture))
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
        assert!(cnki.article_detail().is_some());
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
            .fetch(&catalog(), None)
            .expect("Crossref fixture should fetch");

        assert!(batch.is_complete);
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
                .fetch(&catalog, checkpoint.as_deref())
                .expect("stateful Crossref page should fetch");
            batch_count += 1;
            for article in batch.articles {
                dois.insert(article.doi.expect("fixture article should have a DOI"));
            }
            if batch.is_complete {
                assert!(batch.next_checkpoint.is_none());
                break;
            }
            let next_checkpoint = batch
                .next_checkpoint
                .expect("incomplete batch should have a checkpoint");
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
            .map(|checkpoint| match checkpoint {
                ScholarlyCheckpoint::Crossref {
                    cursor, page_index, ..
                } => (cursor.as_str(), *page_index),
                ScholarlyCheckpoint::OpenAlex { .. } => {
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
    fn crossref_cursor_freshness_has_an_exact_240_second_boundary() {
        assert_eq!(CROSSREF_CURSOR_REUSE_SECONDS, 240);
        assert!(crossref_cursor_is_fresh(Some(761), 1_000));
        assert!(!crossref_cursor_is_fresh(Some(760), 1_000));
        assert!(!crossref_cursor_is_fresh(Some(1_001), 1_000));
        assert!(!crossref_cursor_is_fresh(None, 1_000));
    }

    #[test]
    fn legacy_expired_boundary_and_future_crossref_checkpoints_restart_before_use() {
        let legacy =
            r#"{"mode":"crossref","issn":"1234-5679","cursor":"legacy-secret"}"#.to_string();
        let decoded = serde_json::from_str::<ScholarlyCheckpoint>(&legacy)
            .expect("legacy checkpoint should decode");
        assert!(matches!(
            decoded,
            ScholarlyCheckpoint::Crossref {
                cursor_refreshed_at_epoch_seconds: None,
                ..
            }
        ));
        let checkpoints = [
            legacy,
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
            assert!(batch.is_complete);
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
            batch
                .next_checkpoint
                .as_deref()
                .expect("continued page should retain a checkpoint"),
        )
        .expect("continued checkpoint should decode");

        assert_eq!(
            transport.requested_cursors,
            vec![Some("stateful".to_string())]
        );
        assert_eq!(
            next,
            ScholarlyCheckpoint::Crossref {
                issn: "1234-5679".to_string(),
                cursor: "stateful".to_string(),
                page_index: 8,
                cursor_refreshed_at_epoch_seconds: Some(1_001),
            }
        );
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
        assert!(success.expect("fresh fallback should succeed").is_complete);
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
        let overflow = next_scholarly_checkpoint(
            ScholarlyCheckpoint::Crossref {
                issn: "1234-5679".to_string(),
                cursor: "stateful".to_string(),
                page_index: u64::MAX,
                cursor_refreshed_at_epoch_seconds: Some(1_000),
            },
            Some("stateful".to_string()),
            "stateful",
            false,
            Some(1_001),
        )
        .expect_err("Crossref page index should not wrap");
        assert_eq!(overflow.kind(), ProviderErrorKind::InvalidResponse);
        assert_eq!(
            overflow.to_string(),
            "scholarly Crossref checkpoint page index overflowed"
        );

        let repeated_openalex = next_scholarly_checkpoint(
            ScholarlyCheckpoint::OpenAlex {
                source_id: "S1".to_string(),
                cursor: "fixture-page-1".to_string(),
            },
            Some("fixture-page-1".to_string()),
            "fixture-page-1",
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
        let checkpoint = next_scholarly_checkpoint(
            ScholarlyCheckpoint::Crossref {
                issn: "1234-5679".to_string(),
                cursor: "previous".to_string(),
                page_index: 22,
                cursor_refreshed_at_epoch_seconds: Some(1_000),
            },
            Some(cursor),
            "previous",
            false,
            Some(1_001),
        )
        .expect("Crossref checkpoint should encode")
        .expect("non-terminal page should have a checkpoint");

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
        let checkpoint = serde_json::to_string(&ScholarlyCheckpoint::OpenAlex {
            source_id: "S1".to_string(),
            cursor: "fixture-page-1".to_string(),
        })
        .expect("OpenAlex checkpoint should encode");
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&catalog(), Some(&checkpoint))
            .expect("OpenAlex checkpoint should resume");

        assert_eq!(batch.articles.len(), 1);
        assert_eq!(
            batch.articles[0].doi.as_deref(),
            Some("10.1000/openalex-resume")
        );
        assert!(matches!(
            serde_json::from_str::<ScholarlyCheckpoint>(
                batch
                    .next_checkpoint
                    .as_deref()
                    .expect("resumed OpenAlex page should continue")
            )
            .expect("OpenAlex checkpoint should decode"),
            ScholarlyCheckpoint::OpenAlex { .. }
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
                    assert!(batch.is_complete);
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
            .fetch(&catalog(), None)
            .expect("OpenAlex fallback should fetch");

        assert!(batch.is_complete);
        assert_eq!(batch.articles.len(), 1);
        assert_eq!(batch.articles[0].doi.as_deref(), Some("10.1000/openalex"));
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
        let registration = cnki_index_registration(FixtureCnkiTransport::new(fixture))
            .expect("CNKI registration should pass");
        let mut cnki_catalog = catalog();
        cnki_catalog.title = "CNKI Test Journal".to_string();
        let batch = registration
            .index_content()
            .expect("indexing capability should exist")
            .fetch(&cnki_catalog, None)
            .expect("CNKI fixture should fetch");

        assert!(batch.is_complete);
        assert_eq!(batch.articles.len(), 1);
        let serialized = serde_json::to_string(&batch).expect("batch should serialize");
        assert!(!serialized.contains("CNKI202601001"));
        assert!(!serialized.contains("/kcms"));
        assert!(!serialized.contains("http"));
    }
}
