//! Provider-neutral journal, issue, and article identity primitives.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use litradar_domain::{
    normalize_bibliographic_label, normalize_bibliographic_text, normalize_contract_doi,
    normalize_contract_pmid, stable_sqlite_id, ArticleAuthorDraft, ArticleDraft, IssueDraft,
};

/// Canonical article identity alias kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArticleIdentityKind {
    /// Bibliographic fingerprint derived from provider-neutral content.
    Bibliographic,
    /// Normalized DOI.
    Doi,
    /// Numeric PubMed identifier.
    Pmid,
}

impl ArticleIdentityKind {
    /// Return the stable storage label for this identity kind.
    ///
    /// # Returns
    ///
    /// Stable lowercase identity label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bibliographic => "bibliographic",
            Self::Doi => "doi",
            Self::Pmid => "pmid",
        }
    }
}

/// One canonical provider-neutral article identity alias.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArticleIdentityKey {
    /// Identity kind.
    pub kind: ArticleIdentityKind,
    /// Normalized identity value.
    pub value: String,
}

/// Successful identity resolution before a content upsert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleIdentityResolution {
    /// Immutable article identifier to reuse or insert.
    pub article_id: i64,
    /// Whether at least one supplied alias already resolved to this identifier.
    pub is_existing: bool,
    /// Strongest canonical alias used to allocate or confirm the identifier.
    pub identity_key: ArticleIdentityKey,
}

/// Article identity resolution failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleIdentityError {
    /// The draft has no accepted canonical identity basis.
    MissingIdentity,
    /// Supplied aliases resolve to more than one immutable article identifier.
    ConflictingAliases {
        /// Distinct conflicting article identifiers in deterministic order.
        article_ids: Vec<i64>,
    },
}

impl fmt::Display for ArticleIdentityError {
    /// Format an identity failure without provider payload data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIdentity => formatter.write_str("article has no canonical identity basis"),
            Self::ConflictingAliases { article_ids } => write!(
                formatter,
                "article aliases resolve to multiple IDs: {article_ids:?}"
            ),
        }
    }
}

impl Error for ArticleIdentityError {}

/// Deterministic canonical article merge failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleMergeError {
    /// Drafts refer to different immutable catalog entries.
    CatalogMismatch,
    /// Two non-empty canonical identifiers contradict each other.
    ConflictingIdentifier {
        /// Canonical identifier field that conflicted.
        field: &'static str,
    },
}

impl fmt::Display for ArticleMergeError {
    /// Format a deterministic merge failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CatalogMismatch => {
                formatter.write_str("article drafts use different catalog IDs")
            }
            Self::ConflictingIdentifier { field } => {
                write!(
                    formatter,
                    "article drafts contain conflicting {field} values"
                )
            }
        }
    }
}

impl Error for ArticleMergeError {}

/// Build a stable journal ID from an immutable catalog ID.
///
/// # Arguments
///
/// * `catalog_id` - Validated provider-neutral catalog identifier.
///
/// # Returns
///
/// Stable SQLite journal identifier.
pub fn journal_id_from_catalog_id(catalog_id: &str) -> i64 {
    stable_sqlite_id(catalog_id.trim().to_ascii_lowercase(), "journal:v1")
}

/// Build a canonical issue identity value.
///
/// # Arguments
///
/// * `journal_id` - Stable journal identifier.
/// * `issue` - Provider-neutral issue content.
///
/// # Returns
///
/// Canonical issue key when enough bibliographic information is present.
pub fn issue_identity_value(journal_id: i64, issue: &IssueDraft) -> Option<String> {
    let year = issue.publication_year.map(|value| value.to_string());
    let volume = issue.volume.as_deref().map(normalize_bibliographic_label);
    let number = issue.number.as_deref().map(normalize_bibliographic_label);
    if year.is_some()
        && (volume.as_deref().is_some_and(|value| !value.is_empty())
            || number.as_deref().is_some_and(|value| !value.is_empty()))
    {
        return Some(format!(
            "{journal_id}|{}|{}|{}",
            year.unwrap_or_default(),
            volume.unwrap_or_default(),
            number.unwrap_or_default()
        ));
    }
    issue
        .date
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|date| format!("{journal_id}|date|{date}"))
        .or_else(|| {
            issue
                .title
                .as_deref()
                .map(normalize_bibliographic_text)
                .filter(|value| !value.is_empty())
                .map(|title| format!("{journal_id}|title|{title}"))
        })
}

/// Build a stable issue ID from canonical issue content.
///
/// # Arguments
///
/// * `journal_id` - Stable journal identifier.
/// * `issue` - Provider-neutral issue content.
///
/// # Returns
///
/// Stable issue ID when a canonical issue key can be built.
pub fn issue_id_from_draft(journal_id: i64, issue: &IssueDraft) -> Option<i64> {
    issue_identity_value(journal_id, issue).map(|value| stable_sqlite_id(value, "issue:v1"))
}

/// Build ordered canonical aliases for one article draft.
///
/// # Arguments
///
/// * `article` - Provider-neutral article content.
///
/// # Returns
///
/// Deduplicated aliases ordered from strongest to weakest new-ID seed.
pub fn article_identity_keys(article: &ArticleDraft) -> Vec<ArticleIdentityKey> {
    let mut keys = Vec::new();
    if let Some(value) = article.doi.as_deref().and_then(normalize_contract_doi) {
        keys.push(ArticleIdentityKey {
            kind: ArticleIdentityKind::Doi,
            value,
        });
    }
    if let Some(value) = article.pmid.as_deref().and_then(normalize_contract_pmid) {
        keys.push(ArticleIdentityKey {
            kind: ArticleIdentityKind::Pmid,
            value,
        });
    }
    if let Some(value) = bibliographic_fingerprint(article) {
        keys.push(ArticleIdentityKey {
            kind: ArticleIdentityKind::Bibliographic,
            value,
        });
    }

    let mut seen = BTreeSet::new();
    keys.into_iter()
        .filter(|key| seen.insert((key.kind, key.value.clone())))
        .collect()
}

/// Allocate a stable new article ID from the strongest available canonical alias.
///
/// # Arguments
///
/// * `article` - Provider-neutral article content.
///
/// # Returns
///
/// Stable article ID and selected seed alias when identity evidence is sufficient.
pub fn new_article_id(article: &ArticleDraft) -> Option<(i64, ArticleIdentityKey)> {
    let key = article_identity_keys(article).into_iter().next()?;
    let article_id = stable_sqlite_id(&key.value, format!("article:v1:{}", key.kind.as_str()));
    Some((article_id, key))
}

/// Resolve an article against an immutable alias map without fuzzy matching.
///
/// # Arguments
///
/// * `article` - Provider-neutral article content.
/// * `existing_aliases` - Canonical aliases already stored by the content writer.
///
/// # Returns
///
/// Existing immutable ID, deterministic new ID, or a typed conflict.
pub fn resolve_article_identity(
    article: &ArticleDraft,
    existing_aliases: &BTreeMap<ArticleIdentityKey, i64>,
) -> Result<ArticleIdentityResolution, ArticleIdentityError> {
    let keys = article_identity_keys(article);
    if keys.is_empty() {
        return Err(ArticleIdentityError::MissingIdentity);
    }

    let matched = keys
        .iter()
        .filter_map(|key| {
            existing_aliases
                .get(key)
                .copied()
                .map(|article_id| (key, article_id))
        })
        .collect::<Vec<_>>();
    let article_ids = matched
        .iter()
        .map(|(_, article_id)| *article_id)
        .collect::<BTreeSet<_>>();
    if article_ids.len() > 1 {
        return Err(ArticleIdentityError::ConflictingAliases {
            article_ids: article_ids.into_iter().collect(),
        });
    }
    if let Some(article_id) = article_ids.into_iter().next() {
        let identity_key = matched
            .into_iter()
            .find_map(|(key, matched_id)| (matched_id == article_id).then(|| key.clone()))
            .expect("one alias matched the resolved article ID");
        return Ok(ArticleIdentityResolution {
            article_id,
            is_existing: true,
            identity_key,
        });
    }

    let (article_id, identity_key) = new_article_id(article)
        .expect("non-empty canonical identity keys always allocate an article ID");
    Ok(ArticleIdentityResolution {
        article_id,
        is_existing: false,
        identity_key,
    })
}

/// Merge two canonical article drafts with commutative provider-neutral rules.
///
/// # Arguments
///
/// * `left` - Existing or candidate canonical draft.
/// * `right` - Existing or candidate canonical draft.
///
/// # Returns
///
/// The same canonical result regardless of argument order, or an identifier conflict.
pub fn merge_article_drafts(
    left: &ArticleDraft,
    right: &ArticleDraft,
) -> Result<ArticleDraft, ArticleMergeError> {
    if left.catalog_id != right.catalog_id {
        return Err(ArticleMergeError::CatalogMismatch);
    }
    Ok(ArticleDraft {
        catalog_id: left.catalog_id.clone(),
        title: richer_text(&left.title, &right.title),
        publication_year: merge_ordered_option(left.publication_year, right.publication_year),
        date: richer_optional_text(left.date.as_ref(), right.date.as_ref()),
        issue_title: richer_optional_text(left.issue_title.as_ref(), right.issue_title.as_ref()),
        volume: canonical_optional_text(left.volume.as_ref(), right.volume.as_ref()),
        issue_number: canonical_optional_text(
            left.issue_number.as_ref(),
            right.issue_number.as_ref(),
        ),
        authors: richer_authors(&left.authors, &right.authors),
        start_page: canonical_optional_text(left.start_page.as_ref(), right.start_page.as_ref()),
        end_page: canonical_optional_text(left.end_page.as_ref(), right.end_page.as_ref()),
        abstract_text: richer_optional_text(
            left.abstract_text.as_ref(),
            right.abstract_text.as_ref(),
        ),
        doi: merge_identifier(left.doi.as_ref(), right.doi.as_ref(), "DOI")?,
        pmid: merge_identifier(left.pmid.as_ref(), right.pmid.as_ref(), "PMID")?,
        open_access: merge_true_wins(left.open_access, right.open_access),
        in_press: merge_false_wins(left.in_press, right.in_press),
        retraction_doi: merge_identifier(
            left.retraction_doi.as_ref(),
            right.retraction_doi.as_ref(),
            "retraction DOI",
        )?,
    })
}

/// Normalize a DOI-like string to canonical identifier form.
///
/// # Arguments
///
/// * `value` - DOI or DOI URL.
///
/// # Returns
///
/// Canonical lowercase DOI when valid enough for identity use.
pub fn normalize_doi(value: &str) -> Option<String> {
    normalize_contract_doi(value)
}

fn bibliographic_fingerprint(article: &ArticleDraft) -> Option<String> {
    let publication = article
        .publication_year
        .map(|value| value.to_string())
        .or_else(|| article.date.as_deref().and_then(publication_year_from_date))?;
    let title = normalize_bibliographic_text(&article.title);
    if title.is_empty() {
        return None;
    }
    let volume = article
        .volume
        .as_deref()
        .map(normalize_bibliographic_label)
        .unwrap_or_default();
    let issue = article
        .issue_number
        .as_deref()
        .map(normalize_bibliographic_label)
        .unwrap_or_default();
    let start_page = article
        .start_page
        .as_deref()
        .map(normalize_bibliographic_label)
        .unwrap_or_default();
    if volume.is_empty() && issue.is_empty() && start_page.is_empty() {
        return None;
    }
    Some(format!(
        "{}|{title}|{publication}|{volume}|{issue}|{start_page}",
        article.catalog_id.trim().to_ascii_lowercase()
    ))
}

fn publication_year_from_date(value: &str) -> Option<String> {
    let year = value.split('-').next()?.trim();
    (year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit())).then(|| year.to_string())
}

fn merge_identifier(
    left: Option<&String>,
    right: Option<&String>,
    field: &'static str,
) -> Result<Option<String>, ArticleMergeError> {
    match (left, right) {
        (Some(left), Some(right)) if left != right => {
            Err(ArticleMergeError::ConflictingIdentifier { field })
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value.clone())),
        (None, None) => Ok(None),
    }
}

fn merge_ordered_option<T: Copy + Ord>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn richer_optional_text(left: Option<&String>, right: Option<&String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(richer_text(left, right)),
        (Some(value), None) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    }
}

fn canonical_optional_text(left: Option<&String>, right: Option<&String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right).clone()),
        (Some(value), None) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    }
}

fn richer_text(left: &str, right: &str) -> String {
    match left.len().cmp(&right.len()) {
        std::cmp::Ordering::Greater => left.to_string(),
        std::cmp::Ordering::Less => right.to_string(),
        std::cmp::Ordering::Equal => left.min(right).to_string(),
    }
}

fn richer_authors(
    left: &[ArticleAuthorDraft],
    right: &[ArticleAuthorDraft],
) -> Vec<ArticleAuthorDraft> {
    match left.len().cmp(&right.len()) {
        std::cmp::Ordering::Greater => left.to_vec(),
        std::cmp::Ordering::Less => right.to_vec(),
        std::cmp::Ordering::Equal => {
            let left_key = left
                .iter()
                .map(|author| author.display_name.as_str())
                .collect::<Vec<_>>();
            let right_key = right
                .iter()
                .map(|author| author.display_name.as_str())
                .collect::<Vec<_>>();
            if left_key <= right_key {
                left.to_vec()
            } else {
                right.to_vec()
            }
        }
    }
}

fn merge_true_wins(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left || right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn merge_false_wins(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left && right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use litradar_domain::{ArticleAuthorDraft, ArticleDraft, IssueDraft};

    use super::{
        article_identity_keys, issue_id_from_draft, journal_id_from_catalog_id,
        merge_article_drafts, new_article_id, normalize_doi, resolve_article_identity,
        ArticleIdentityError, ArticleIdentityKind, ArticleMergeError,
    };

    fn article(title: &str, doi: Option<&str>) -> ArticleDraft {
        ArticleDraft {
            catalog_id: "issn-1234-5679".to_string(),
            title: title.to_string(),
            publication_year: Some(2026),
            date: Some("2026-07-18".to_string()),
            issue_title: None,
            volume: Some("01".to_string()),
            issue_number: Some("002".to_string()),
            authors: Vec::new(),
            start_page: Some("0001".to_string()),
            end_page: Some("8".to_string()),
            abstract_text: None,
            doi: doi.map(str::to_string),
            pmid: None,
            open_access: None,
            in_press: None,
            retraction_doi: None,
        }
    }

    #[test]
    fn provider_variants_share_bibliographic_identity() {
        let left = article("Caf\u{e9}: Canonical Article!", Some("10.1000/CANONICAL"));
        let right = article(" Cafe\u{301} canonical article ", None);

        let left_key = article_identity_keys(&left)
            .into_iter()
            .find(|key| key.kind == ArticleIdentityKind::Bibliographic)
            .expect("left bibliographic key");
        let right_key = article_identity_keys(&right)
            .into_iter()
            .find(|key| key.kind == ArticleIdentityKind::Bibliographic)
            .expect("right bibliographic key");
        assert_eq!(left_key, right_key);
    }

    #[test]
    fn normalized_identifier_variants_share_aliases() {
        assert_eq!(
            normalize_doi("HTTPS://DOI.ORG/10.1000/Canonical"),
            Some("10.1000/canonical".to_string())
        );
        assert_eq!(
            normalize_doi("doi:10.1000/CANONICAL"),
            Some("10.1000/canonical".to_string())
        );

        let mut left = article("Canonical Article", None);
        left.pmid = Some("000123".to_string());
        let mut right = article("Canonical Article", None);
        right.pmid = Some("123".to_string());
        let left_pmid = article_identity_keys(&left)
            .into_iter()
            .find(|key| key.kind == ArticleIdentityKind::Pmid);
        let right_pmid = article_identity_keys(&right)
            .into_iter()
            .find(|key| key.kind == ArticleIdentityKind::Pmid);
        assert_eq!(left_pmid, right_pmid);
    }

    #[test]
    fn journal_and_issue_ids_ignore_provider_details_and_formatting() {
        let journal_id = journal_id_from_catalog_id("issn-1234-5679");
        let issue = IssueDraft {
            catalog_id: "issn-1234-5679".to_string(),
            publication_year: Some(2026),
            title: None,
            volume: Some("01".to_string()),
            number: Some("002".to_string()),
            date: None,
        };
        let normalized = IssueDraft {
            volume: Some("1".to_string()),
            number: Some("2".to_string()),
            ..issue.clone()
        };
        assert_eq!(
            issue_id_from_draft(journal_id, &issue),
            issue_id_from_draft(journal_id, &normalized)
        );
        assert_eq!(journal_id, journal_id_from_catalog_id(" ISSN-1234-5679 "));
    }

    #[test]
    fn later_identifiers_reuse_the_existing_bibliographic_id() {
        let initial = article("Canonical Article", None);
        let first = resolve_article_identity(&initial, &BTreeMap::new())
            .expect("initial identity should allocate");
        let existing_aliases = article_identity_keys(&initial)
            .into_iter()
            .map(|key| (key, first.article_id))
            .collect::<BTreeMap<_, _>>();

        let enriched = article("Canonical Article", Some("10.1000/canonical"));
        let second = resolve_article_identity(&enriched, &existing_aliases)
            .expect("enriched identity should resolve");
        assert_eq!(second.article_id, first.article_id);
        assert!(second.is_existing);
        assert_eq!(
            new_article_id(&enriched).map(|(_, key)| key.kind),
            Some(ArticleIdentityKind::Doi)
        );
    }

    #[test]
    fn conflicting_aliases_fail_without_fuzzy_resolution() {
        let article = article("Canonical Article", Some("10.1000/canonical"));
        let keys = article_identity_keys(&article);
        let existing_aliases = BTreeMap::from([(keys[0].clone(), 11), (keys[1].clone(), 22)]);

        assert_eq!(
            resolve_article_identity(&article, &existing_aliases),
            Err(ArticleIdentityError::ConflictingAliases {
                article_ids: vec![11, 22]
            })
        );
    }

    #[test]
    fn canonical_merge_is_commutative_and_enriches_content() {
        let mut sparse = article("Canonical Article", Some("10.1000/canonical"));
        sparse.authors = vec![ArticleAuthorDraft {
            display_name: "Ada Lovelace".to_string(),
        }];
        sparse.abstract_text = Some("Short".to_string());
        sparse.open_access = Some(false);
        sparse.in_press = Some(true);

        let mut rich = sparse.clone();
        rich.authors.push(ArticleAuthorDraft {
            display_name: "Alan Turing".to_string(),
        });
        rich.abstract_text = Some("A substantially longer abstract".to_string());
        rich.open_access = Some(true);
        rich.in_press = Some(false);

        let left_then_right = merge_article_drafts(&sparse, &rich).expect("merge should pass");
        let right_then_left = merge_article_drafts(&rich, &sparse).expect("merge should pass");
        assert_eq!(left_then_right, right_then_left);
        assert_eq!(left_then_right.authors.len(), 2);
        assert_eq!(
            left_then_right.abstract_text.as_deref(),
            Some("A substantially longer abstract")
        );
        assert_eq!(left_then_right.open_access, Some(true));
        assert_eq!(left_then_right.in_press, Some(false));
    }

    #[test]
    fn canonical_merge_rejects_identifier_and_catalog_conflicts() {
        let left = article("Canonical Article", Some("10.1000/left"));
        let right = article("Canonical Article", Some("10.1000/right"));
        assert_eq!(
            merge_article_drafts(&left, &right),
            Err(ArticleMergeError::ConflictingIdentifier { field: "DOI" })
        );

        let mut other_catalog = left.clone();
        other_catalog.catalog_id = "issn-2049-3630".to_string();
        assert_eq!(
            merge_article_drafts(&left, &other_catalog),
            Err(ArticleMergeError::CatalogMismatch)
        );
    }
}
