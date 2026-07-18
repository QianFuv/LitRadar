//! Built-in canonical indexing provider adapters.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use litradar_domain::{
    normalize_contract_doi, normalize_contract_pmid, normalize_contract_text, ArticleAuthorDraft,
    ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft, ProviderBatch,
};
use litradar_provider::{
    IndexContentProvider, ProviderCapabilities, ProviderDescriptor, ProviderError,
    ProviderErrorKind, ProviderImplementations, ProviderRegistration, ProviderRegistryError,
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

const SCHOLARLY_ENRICHMENT_BATCH_SIZE: usize = 100;

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
        },
        ProviderImplementations {
            index_content: Some(Arc::new(CnkiIndexProvider::new(transport))),
            ..ProviderImplementations::default()
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum ScholarlyCheckpoint {
    Crossref { issn: String, cursor: String },
    OpenAlex { source_id: String, cursor: String },
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

fn fetch_scholarly_batch<T>(
    client: &mut ScholarlyClient<T>,
    catalog: &JournalCatalogEntry,
    checkpoint: Option<&str>,
    has_semantic_scholar_key: bool,
) -> Result<ProviderBatch, ProviderError>
where
    T: ScholarlyTransport,
{
    let (works, next_checkpoint) = if let Some(checkpoint) = checkpoint {
        let checkpoint = serde_json::from_str::<ScholarlyCheckpoint>(checkpoint).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::InvalidResponse,
                "scholarly checkpoint is invalid",
            )
        })?;
        match checkpoint {
            ScholarlyCheckpoint::Crossref { issn, cursor } => {
                let page = client
                    .fetch_journal_works_page(&issn, None, Some(&cursor))
                    .map_err(map_scholarly_error)?;
                let next = next_scholarly_checkpoint(
                    ScholarlyCheckpoint::Crossref {
                        issn,
                        cursor: cursor.clone(),
                    },
                    page.next_cursor,
                    &cursor,
                    page.items.is_empty(),
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
                )?;
                return Ok(batch_from_articles(catalog, items, next));
            }
        }
    } else {
        match first_scholarly_page(client, catalog)? {
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
) -> Result<FirstScholarlyPage, ProviderError>
where
    T: ScholarlyTransport,
{
    let issns = catalog_issns(catalog);
    for issn in &issns {
        match client.fetch_journal_works_page(issn, None, None) {
            Ok(page) => {
                let next = next_scholarly_checkpoint(
                    ScholarlyCheckpoint::Crossref {
                        issn: issn.clone(),
                        cursor: String::new(),
                    },
                    page.next_cursor,
                    "",
                    page.items.is_empty(),
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
) -> Result<Option<String>, ProviderError> {
    let Some(next_cursor) = next_cursor.filter(|_| !is_empty) else {
        return Ok(None);
    };
    if next_cursor == previous_cursor {
        return Err(ProviderError::new(
            ProviderErrorKind::InvalidResponse,
            "scholarly provider returned a repeated cursor",
        ));
    }
    let checkpoint = match current {
        ScholarlyCheckpoint::Crossref { issn, .. } => ScholarlyCheckpoint::Crossref {
            issn,
            cursor: next_cursor,
        },
        ScholarlyCheckpoint::OpenAlex { source_id, .. } => ScholarlyCheckpoint::OpenAlex {
            source_id,
            cursor: next_cursor,
        },
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
        retraction_doi: relation_doi(work.get("relation")),
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
        retraction_doi: None,
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
        retraction_doi: json_text(detail.get("retraction_doi"))
            .and_then(|value| normalize_contract_doi(&value)),
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

fn relation_doi(value: Option<&Value>) -> Option<String> {
    for relation in value?.as_object()?.values() {
        for item in relation.as_array()? {
            if let Some(doi) =
                json_text(item.get("id")).and_then(|value| normalize_contract_doi(&value))
            {
                return Some(doi);
            }
        }
    }
    None
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
    use std::collections::BTreeMap;

    use litradar_domain::{JournalRankings, ProviderCapabilityKind};
    use litradar_provider::ProviderRegistry;
    use serde_json::json;

    use super::{
        cnki_article_draft, cnki_index_registration, cnki_issue_draft, scholarly_article_draft,
        scholarly_index_registration, CnkiIndexProvider, ScholarlyIndexProvider,
    };
    use crate::{
        CnkiFixtureData, FixtureCnkiTransport, FixtureScholarlyTransport, ScholarlyFixtureData,
    };

    fn catalog() -> litradar_domain::JournalCatalogEntry {
        litradar_domain::JournalCatalogEntry {
            catalog_id: "issn-1234-5679".to_string(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
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
