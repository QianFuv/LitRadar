//! Reusable conformance checks for provider-neutral content batches.

use std::error::Error;
use std::fmt;

use litradar_domain::{
    normalize_bibliographic_text, normalize_contract_doi, normalize_contract_issn,
    normalize_contract_pmid, normalize_contract_text, ArticleAccessContext, ArticleDraft,
    ArticleFullTextResolution, ArticleLocator, ArticleRedirect, JournalCatalogEntry, ProviderBatch,
};

use crate::{
    ArticleAbstractProvider, ArticleDetailProvider, ArticleFullTextProvider, IndexContentProvider,
};

const MAX_CHECKPOINT_BYTES: usize = 65_536;

/// Provider contract validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    message: String,
}

impl ContractViolation {
    /// Build a contract violation with a safe diagnostic.
    ///
    /// # Arguments
    ///
    /// * `message` - Safe diagnostic without provider payload content.
    ///
    /// # Returns
    ///
    /// Contract violation.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ContractViolation {
    /// Format the safe validation diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ContractViolation {}

/// Validate one maintained journal catalog entry.
///
/// # Arguments
///
/// * `entry` - Canonical catalog entry.
///
/// # Returns
///
/// Success when every canonical field satisfies contract v1.
pub fn validate_catalog_entry(entry: &JournalCatalogEntry) -> Result<(), ContractViolation> {
    validate_catalog_id(&entry.catalog_id)?;
    require_canonical_text(&entry.title, "catalog title")?;
    validate_optional_issn(entry.issn.as_deref(), "print ISSN")?;
    validate_optional_issn(entry.eissn.as_deref(), "electronic ISSN")?;

    let mut all_issns = Vec::new();
    for value in &entry.all_issns {
        require_canonical_issn(value, "catalog ISSN")?;
        if all_issns.contains(value) {
            return Err(ContractViolation::new("catalog ISSNs must be unique"));
        }
        all_issns.push(value.clone());
    }
    for value in [entry.issn.as_ref(), entry.eissn.as_ref()]
        .into_iter()
        .flatten()
    {
        if !all_issns.contains(value) {
            return Err(ContractViolation::new(
                "primary catalog ISSNs must appear in all_issns",
            ));
        }
    }

    let canonical_title = normalize_bibliographic_text(&entry.title);
    let mut aliases = Vec::new();
    for alias in &entry.title_aliases {
        require_canonical_text(alias, "title alias")?;
        let normalized = normalize_bibliographic_text(alias);
        if normalized == canonical_title || aliases.contains(&normalized) {
            return Err(ContractViolation::new(
                "catalog title aliases must be distinct",
            ));
        }
        aliases.push(normalized);
    }
    validate_optional_canonical_text(entry.area.as_deref(), "catalog area")?;
    for (value, field) in [
        (entry.rankings.utd_rank.as_deref(), "UTD rank"),
        (entry.rankings.utd_rating.as_deref(), "UTD rating"),
        (entry.rankings.abs_rank.as_deref(), "ABS rank"),
        (entry.rankings.abs_rating.as_deref(), "ABS rating"),
        (entry.rankings.fms_rank.as_deref(), "FMS rank"),
        (entry.rankings.fms_rating.as_deref(), "FMS rating"),
        (entry.rankings.fmscn_rank.as_deref(), "FMS China rank"),
        (entry.rankings.fmscn_rating.as_deref(), "FMS China rating"),
    ] {
        validate_optional_canonical_text(value, field)?;
    }
    Ok(())
}

/// Execute and validate an indexing provider fixture.
///
/// # Arguments
///
/// * `provider` - Provider implementation under test.
/// * `catalog` - Canonical catalog fixture.
/// * `checkpoint` - Optional opaque checkpoint fixture.
///
/// # Returns
///
/// Validated provider batch.
pub fn validate_index_provider_fixture(
    provider: &dyn IndexContentProvider,
    catalog: &JournalCatalogEntry,
    checkpoint: Option<&str>,
) -> Result<ProviderBatch, ContractViolation> {
    let batch = provider.fetch(catalog, checkpoint).map_err(|error| {
        ContractViolation::new(format!(
            "index provider fixture failed with {:?}",
            error.kind()
        ))
    })?;
    validate_provider_batch(catalog, &batch)?;
    Ok(batch)
}

/// Execute and validate a detail capability fixture.
///
/// # Arguments
///
/// * `provider` - Detail provider implementation under test.
/// * `article` - Canonical article locator fixture.
/// * `context` - Request context fixture.
///
/// # Returns
///
/// Validated ephemeral redirect.
pub fn validate_detail_provider_fixture(
    provider: &dyn ArticleDetailProvider,
    article: &ArticleLocator,
    context: ArticleAccessContext,
) -> Result<ArticleRedirect, ContractViolation> {
    validate_article_locator(article)?;
    let redirect = provider.resolve_detail(article, context).map_err(|error| {
        ContractViolation::new(format!(
            "detail provider fixture failed with {:?}",
            error.kind()
        ))
    })?;
    validate_article_redirect(&redirect)?;
    Ok(redirect)
}

/// Execute and validate an abstract-page capability fixture.
///
/// # Arguments
///
/// * `provider` - Abstract-page provider implementation under test.
/// * `article` - Canonical article locator fixture.
/// * `context` - Request context fixture.
///
/// # Returns
///
/// Validated ephemeral redirect.
pub fn validate_abstract_provider_fixture(
    provider: &dyn ArticleAbstractProvider,
    article: &ArticleLocator,
    context: ArticleAccessContext,
) -> Result<ArticleRedirect, ContractViolation> {
    validate_article_locator(article)?;
    let redirect = provider
        .resolve_abstract(article, context)
        .map_err(|error| {
            ContractViolation::new(format!(
                "abstract provider fixture failed with {:?}",
                error.kind()
            ))
        })?;
    validate_article_redirect(&redirect)?;
    Ok(redirect)
}

/// Execute and validate a full-text capability fixture.
///
/// # Arguments
///
/// * `provider` - Full-text provider implementation under test.
/// * `article` - Canonical article locator fixture.
/// * `context` - Request context fixture.
/// * `maximum_document_bytes` - Maximum accepted in-memory document size.
///
/// # Returns
///
/// Validated ephemeral full-text result.
pub fn validate_full_text_provider_fixture(
    provider: &dyn ArticleFullTextProvider,
    article: &ArticleLocator,
    context: ArticleAccessContext,
    maximum_document_bytes: usize,
) -> Result<ArticleFullTextResolution, ContractViolation> {
    validate_article_locator(article)?;
    let resolution = provider
        .resolve_full_text(article, context)
        .map_err(|error| {
            ContractViolation::new(format!(
                "full-text provider fixture failed with {:?}",
                error.kind()
            ))
        })?;
    validate_full_text_resolution(&resolution, maximum_document_bytes)?;
    Ok(resolution)
}

/// Validate one provider-neutral article locator.
///
/// # Arguments
///
/// * `article` - Locator loaded from canonical content storage.
///
/// # Returns
///
/// Success when the locator satisfies the request-time provider contract.
pub fn validate_article_locator(article: &ArticleLocator) -> Result<(), ContractViolation> {
    if article.article_id.value() <= 0 {
        return Err(ContractViolation::new(
            "article locator must contain a positive internal ID",
        ));
    }
    validate_catalog_id(&article.catalog_id)?;
    require_canonical_text(&article.journal_title, "locator journal title")?;
    require_canonical_text(&article.title, "locator article title")?;
    validate_optional_year(article.publication_year, "locator publication year")?;
    validate_optional_date(article.date.as_deref())?;
    if let (Some(year), Some(date)) = (article.publication_year, article.date.as_deref()) {
        if date.get(..4).and_then(|value| value.parse::<i64>().ok()) != Some(year) {
            return Err(ContractViolation::new(
                "locator publication year and date must agree",
            ));
        }
    }
    let mut journal_issns = Vec::new();
    for issn in &article.journal_issns {
        require_canonical_issn(issn, "locator journal ISSN")?;
        if journal_issns.contains(issn) {
            return Err(ContractViolation::new(
                "locator journal ISSNs must be unique",
            ));
        }
        journal_issns.push(issn.clone());
    }
    for author in &article.authors {
        require_canonical_text(author, "locator author")?;
    }
    for (value, field) in [
        (article.volume.as_deref(), "locator volume"),
        (article.issue_number.as_deref(), "locator issue number"),
        (article.start_page.as_deref(), "locator start page"),
        (article.end_page.as_deref(), "locator end page"),
    ] {
        validate_optional_canonical_text(value, field)?;
    }
    validate_optional_doi(article.doi.as_deref(), "locator DOI")?;
    if let Some(pmid) = article.pmid.as_deref() {
        if normalize_contract_pmid(pmid).as_deref() != Some(pmid) {
            return Err(ContractViolation::new(
                "locator PMID must use canonical digits",
            ));
        }
    }
    Ok(())
}

/// Validate one ephemeral article redirect.
///
/// # Arguments
///
/// * `redirect` - Redirect returned by a live provider capability.
///
/// # Returns
///
/// Success when the destination has a safe HTTP(S) shape.
pub fn validate_article_redirect(redirect: &ArticleRedirect) -> Result<(), ContractViolation> {
    let location = redirect.location.as_str();
    if location.len() > 8_192
        || location != location.trim()
        || location.chars().any(char::is_control)
    {
        return Err(ContractViolation::new(
            "article redirect must be a bounded canonical HTTP(S) URL",
        ));
    }
    let remainder = location
        .strip_prefix("https://")
        .or_else(|| location.strip_prefix("http://"))
        .ok_or_else(|| ContractViolation::new("article redirect must use HTTP(S)"))?;
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace)
    {
        return Err(ContractViolation::new(
            "article redirect authority is not safe",
        ));
    }
    Ok(())
}

/// Validate one ephemeral full-text result.
///
/// # Arguments
///
/// * `resolution` - Result returned by a full-text provider.
/// * `maximum_document_bytes` - Maximum accepted document size.
///
/// # Returns
///
/// Success when redirects and documents satisfy the bounded contract.
pub fn validate_full_text_resolution(
    resolution: &ArticleFullTextResolution,
    maximum_document_bytes: usize,
) -> Result<(), ContractViolation> {
    match resolution {
        ArticleFullTextResolution::Redirect(redirect) => validate_article_redirect(redirect),
        ArticleFullTextResolution::Document(document) => {
            if maximum_document_bytes == 0
                || document.bytes.is_empty()
                || document.bytes.len() > maximum_document_bytes
            {
                return Err(ContractViolation::new(
                    "full-text document exceeds the configured size contract",
                ));
            }
            require_canonical_text(&document.content_type, "full-text content type")?;
            if document.content_type != document.content_type.to_ascii_lowercase()
                || !document.content_type.contains('/')
                || document.content_type.chars().any(char::is_whitespace)
            {
                return Err(ContractViolation::new(
                    "full-text content type must be canonical",
                ));
            }
            if let Some(filename) = document.filename.as_deref() {
                require_canonical_text(filename, "full-text filename")?;
                if filename.len() > 255
                    || filename.contains('/')
                    || filename.contains('\\')
                    || filename.chars().any(char::is_control)
                {
                    return Err(ContractViolation::new(
                        "full-text filename must be a safe basename",
                    ));
                }
            }
            Ok(())
        }
    }
}

/// Validate one provider batch against the requested catalog entry.
///
/// # Arguments
///
/// * `catalog` - LitRadar-owned catalog entry supplied to the provider.
/// * `batch` - Provider response to validate.
///
/// # Returns
///
/// Success when the provider returned only canonical, matching content.
pub fn validate_provider_batch(
    catalog: &JournalCatalogEntry,
    batch: &ProviderBatch,
) -> Result<(), ContractViolation> {
    validate_catalog_entry(catalog)?;
    if batch.catalog_id != catalog.catalog_id || batch.journal.catalog_id != catalog.catalog_id {
        return Err(ContractViolation::new(
            "provider batch must echo the requested catalog_id",
        ));
    }
    validate_journal_observation(catalog, batch)?;

    for issue in &batch.issues {
        if issue.catalog_id != catalog.catalog_id {
            return Err(ContractViolation::new(
                "issue must echo the requested catalog_id",
            ));
        }
        validate_issue_identity(
            issue.publication_year,
            issue.date.as_deref(),
            issue.volume.as_deref(),
            issue.number.as_deref(),
            issue.title.as_deref(),
        )?;
    }
    for article in &batch.articles {
        validate_article(catalog, article)?;
    }
    if batch
        .next_checkpoint
        .as_ref()
        .is_some_and(|value| value.len() > MAX_CHECKPOINT_BYTES)
    {
        return Err(ContractViolation::new(
            "provider checkpoint exceeds the contract limit",
        ));
    }
    if batch.is_complete && batch.next_checkpoint.is_some() {
        return Err(ContractViolation::new(
            "a complete provider batch cannot include a next checkpoint",
        ));
    }
    Ok(())
}

fn validate_catalog_id(catalog_id: &str) -> Result<(), ContractViolation> {
    if !(3..=128).contains(&catalog_id.len())
        || !catalog_id.is_ascii()
        || !catalog_id
            .bytes()
            .enumerate()
            .all(|(index, byte)| match byte {
                b'a'..=b'z' | b'0'..=b'9' => true,
                b'.' | b'_' | b'-' => index > 0,
                _ => false,
            })
    {
        return Err(ContractViolation::new(
            "catalog_id must match the canonical ASCII format",
        ));
    }
    Ok(())
}

fn validate_journal_observation(
    catalog: &JournalCatalogEntry,
    batch: &ProviderBatch,
) -> Result<(), ContractViolation> {
    if let Some(title) = batch.journal.observed_title.as_deref() {
        require_canonical_text(title, "observed journal title")?;
        let observed = normalize_bibliographic_text(title);
        let is_known = observed == normalize_bibliographic_text(&catalog.title)
            || catalog
                .title_aliases
                .iter()
                .any(|alias| normalize_bibliographic_text(alias) == observed);
        if !is_known {
            return Err(ContractViolation::new(
                "observed journal title is not canonical or an accepted alias",
            ));
        }
    }
    for alias in &batch.journal.observed_title_aliases {
        require_canonical_text(alias, "observed title alias")?;
        let observed = normalize_bibliographic_text(alias);
        if observed != normalize_bibliographic_text(&catalog.title)
            && !catalog
                .title_aliases
                .iter()
                .any(|candidate| normalize_bibliographic_text(candidate) == observed)
        {
            return Err(ContractViolation::new(
                "observed title alias is not maintained by the catalog",
            ));
        }
    }
    for issn in &batch.journal.observed_issns {
        require_canonical_issn(issn, "observed ISSN")?;
    }
    if !batch.journal.observed_issns.is_empty()
        && !catalog.all_issns.is_empty()
        && !batch
            .journal
            .observed_issns
            .iter()
            .any(|issn| catalog.all_issns.contains(issn))
    {
        return Err(ContractViolation::new(
            "observed journal ISSNs contradict the maintained catalog",
        ));
    }
    Ok(())
}

fn validate_article(
    catalog: &JournalCatalogEntry,
    article: &ArticleDraft,
) -> Result<(), ContractViolation> {
    if article.catalog_id != catalog.catalog_id {
        return Err(ContractViolation::new(
            "article must echo the requested catalog_id",
        ));
    }
    require_canonical_text(&article.title, "article title")?;
    validate_optional_year(article.publication_year, "article publication year")?;
    validate_optional_date(article.date.as_deref())?;
    if let (Some(year), Some(date)) = (article.publication_year, article.date.as_deref()) {
        if date.get(..4).and_then(|value| value.parse::<i64>().ok()) != Some(year) {
            return Err(ContractViolation::new(
                "article publication year and date must agree",
            ));
        }
    }
    validate_optional_doi(article.doi.as_deref(), "DOI")?;
    validate_optional_doi(article.retraction_doi.as_deref(), "retraction DOI")?;
    if let Some(pmid) = article.pmid.as_deref() {
        if normalize_contract_pmid(pmid).as_deref() != Some(pmid) {
            return Err(ContractViolation::new("PMID must use canonical digits"));
        }
    }
    for author in &article.authors {
        require_canonical_text(&author.display_name, "author display name")?;
    }
    for (value, field) in [
        (article.issue_title.as_deref(), "article issue title"),
        (article.volume.as_deref(), "article volume"),
        (article.issue_number.as_deref(), "article issue number"),
        (article.start_page.as_deref(), "article start page"),
        (article.end_page.as_deref(), "article end page"),
        (article.abstract_text.as_deref(), "article abstract"),
    ] {
        validate_optional_canonical_text(value, field)?;
    }

    let has_external_identifier = article.doi.is_some() || article.pmid.is_some();
    let has_publication_time = article.publication_year.is_some() || article.date.is_some();
    let has_bibliographic_position =
        article.volume.is_some() || article.issue_number.is_some() || article.start_page.is_some();
    if !(has_external_identifier || has_publication_time && has_bibliographic_position) {
        return Err(ContractViolation::new(
            "article needs DOI, PMID, or a complete bibliographic identity basis",
        ));
    }
    Ok(())
}

fn validate_issue_identity(
    publication_year: Option<i64>,
    date: Option<&str>,
    volume: Option<&str>,
    number: Option<&str>,
    title: Option<&str>,
) -> Result<(), ContractViolation> {
    validate_optional_year(publication_year, "issue publication year")?;
    validate_optional_date(date)?;
    if let (Some(year), Some(date)) = (publication_year, date) {
        if date.get(..4).and_then(|value| value.parse::<i64>().ok()) != Some(year) {
            return Err(ContractViolation::new(
                "issue publication year and date must agree",
            ));
        }
    }
    for (value, field) in [
        (volume, "issue volume"),
        (number, "issue number"),
        (title, "issue title"),
    ] {
        validate_optional_canonical_text(value, field)?;
    }
    let has_numbered_identity =
        publication_year.is_some() && (volume.is_some() || number.is_some());
    if !has_numbered_identity && date.is_none() && title.is_none() {
        return Err(ContractViolation::new(
            "issue needs year plus volume/number, or a date/title fallback",
        ));
    }
    Ok(())
}

fn validate_optional_doi(value: Option<&str>, field: &str) -> Result<(), ContractViolation> {
    let Some(value) = value else {
        return Ok(());
    };
    if normalize_contract_doi(value).as_deref() != Some(value) {
        return Err(ContractViolation::new(format!(
            "{field} must use canonical DOI form"
        )));
    }
    Ok(())
}

fn validate_optional_issn(value: Option<&str>, field: &str) -> Result<(), ContractViolation> {
    if let Some(value) = value {
        require_canonical_issn(value, field)?;
    }
    Ok(())
}

fn require_canonical_issn(value: &str, field: &str) -> Result<(), ContractViolation> {
    if normalize_contract_issn(value).as_deref() != Some(value) {
        return Err(ContractViolation::new(format!(
            "{field} must use canonical NNNN-NNNX form"
        )));
    }
    Ok(())
}

fn validate_optional_date(value: Option<&str>) -> Result<(), ContractViolation> {
    let Some(value) = value else {
        return Ok(());
    };
    if normalize_contract_text(value).as_deref() != Some(value) {
        return Err(ContractViolation::new("date must use canonical text"));
    }
    let parts = value.split('-').collect::<Vec<_>>();
    if !(1..=3).contains(&parts.len())
        || parts[0].len() != 4
        || !parts[0].bytes().all(|byte| byte.is_ascii_digit())
        || parts
            .iter()
            .skip(1)
            .any(|part| part.len() != 2 || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(ContractViolation::new(
            "date must use YYYY, YYYY-MM, or YYYY-MM-DD",
        ));
    }
    let year = parts[0]
        .parse::<i64>()
        .map_err(|_| ContractViolation::new("date year is invalid"))?;
    validate_optional_year(Some(year), "date year")?;
    if parts.len() >= 2 {
        let month = parts[1]
            .parse::<u8>()
            .map_err(|_| ContractViolation::new("date month is invalid"))?;
        if !(1..=12).contains(&month) {
            return Err(ContractViolation::new("date month is invalid"));
        }
    }
    if parts.len() == 3 {
        let day = parts[2]
            .parse::<u8>()
            .map_err(|_| ContractViolation::new("date day is invalid"))?;
        if !(1..=31).contains(&day) {
            return Err(ContractViolation::new("date day is invalid"));
        }
    }
    Ok(())
}

fn validate_optional_year(value: Option<i64>, field: &str) -> Result<(), ContractViolation> {
    if value.is_some_and(|year| !(1_000..=9_999).contains(&year)) {
        return Err(ContractViolation::new(format!(
            "{field} must use a four-digit positive year"
        )));
    }
    Ok(())
}

fn validate_optional_canonical_text(
    value: Option<&str>,
    field: &str,
) -> Result<(), ContractViolation> {
    if let Some(value) = value {
        require_canonical_text(value, field)?;
    }
    Ok(())
}

fn require_canonical_text(value: &str, field: &str) -> Result<(), ContractViolation> {
    if normalize_contract_text(value).as_deref() != Some(value) {
        return Err(ContractViolation::new(format!(
            "{field} must be non-empty, trimmed, and Unicode-normalized"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use litradar_domain::{
        ArticleAccessContext, ArticleDraft, ArticleFullTextDocument, ArticleFullTextResolution,
        ArticleId, ArticleLocator, ArticleRedirect, IssueDraft, JournalCatalogEntry, JournalDraft,
        JournalRankings, ProviderBatch,
    };

    use super::{
        validate_abstract_provider_fixture, validate_article_redirect, validate_catalog_entry,
        validate_detail_provider_fixture, validate_full_text_provider_fixture,
        validate_full_text_resolution, validate_index_provider_fixture, validate_provider_batch,
    };
    use crate::{
        ArticleAbstractProvider, ArticleDetailProvider, ArticleFullTextProvider,
        IndexContentProvider, ProviderError,
    };

    struct FakeProvider;

    impl IndexContentProvider for FakeProvider {
        fn fetch(
            &self,
            _catalog: &JournalCatalogEntry,
            _checkpoint: Option<&str>,
        ) -> Result<ProviderBatch, ProviderError> {
            Ok(batch())
        }
    }

    impl ArticleDetailProvider for FakeProvider {
        fn resolve_detail(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            Ok(ArticleRedirect {
                location: "https://example.test/article".to_string(),
            })
        }
    }

    impl ArticleAbstractProvider for FakeProvider {
        fn resolve_abstract(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            Ok(ArticleRedirect {
                location: "https://example.test/abstract".to_string(),
            })
        }
    }

    impl ArticleFullTextProvider for FakeProvider {
        fn resolve_full_text(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleFullTextResolution, ProviderError> {
            Ok(ArticleFullTextResolution::Document(
                ArticleFullTextDocument {
                    content_type: "application/pdf".to_string(),
                    filename: Some("article.pdf".to_string()),
                    bytes: b"fixture".to_vec(),
                },
            ))
        }
    }

    fn catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "issn-1234-5679".to_string(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: vec!["Canonical J.".to_string()],
            area: Some("Systems".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn batch() -> ProviderBatch {
        ProviderBatch {
            catalog_id: "issn-1234-5679".to_string(),
            journal: JournalDraft {
                catalog_id: "issn-1234-5679".to_string(),
                observed_title: Some("Canonical J.".to_string()),
                observed_issns: vec!["1234-5679".to_string()],
                observed_title_aliases: Vec::new(),
            },
            issues: vec![IssueDraft {
                catalog_id: "issn-1234-5679".to_string(),
                publication_year: Some(2026),
                title: None,
                volume: Some("1".to_string()),
                number: Some("2".to_string()),
                date: Some("2026-07".to_string()),
            }],
            articles: vec![ArticleDraft {
                catalog_id: "issn-1234-5679".to_string(),
                title: "Canonical Article".to_string(),
                publication_year: Some(2026),
                date: Some("2026-07-18".to_string()),
                issue_title: None,
                volume: Some("1".to_string()),
                issue_number: Some("2".to_string()),
                authors: Vec::new(),
                start_page: Some("1".to_string()),
                end_page: Some("8".to_string()),
                abstract_text: Some("Abstract".to_string()),
                doi: Some("10.1000/canonical".to_string()),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_doi: None,
            }],
            is_complete: true,
            next_checkpoint: None,
        }
    }

    fn article_locator() -> ArticleLocator {
        ArticleLocator {
            article_id: ArticleId(1),
            catalog_id: "issn-1234-5679".to_string(),
            journal_title: "Canonical Journal".to_string(),
            journal_issns: vec!["1234-5679".to_string()],
            title: "Canonical Article".to_string(),
            publication_year: Some(2026),
            date: Some("2026-07-18".to_string()),
            authors: Vec::new(),
            volume: Some("1".to_string()),
            issue_number: Some("2".to_string()),
            start_page: Some("1".to_string()),
            end_page: Some("8".to_string()),
            doi: Some("10.1000/canonical".to_string()),
            pmid: None,
        }
    }

    #[test]
    fn accepts_canonical_catalog_and_provider_batch() {
        let catalog = catalog();
        validate_catalog_entry(&catalog).expect("catalog should pass");
        validate_provider_batch(&catalog, &batch()).expect("batch should pass");
    }

    #[test]
    fn shared_fixture_harness_covers_every_declared_capability() {
        let provider = FakeProvider;
        let catalog = catalog();
        let article = article_locator();

        validate_index_provider_fixture(&provider, &catalog, None)
            .expect("index fixture should pass");
        validate_detail_provider_fixture(&provider, &article, ArticleAccessContext::default())
            .expect("detail fixture should pass");
        validate_abstract_provider_fixture(&provider, &article, ArticleAccessContext::default())
            .expect("abstract fixture should pass");
        validate_full_text_provider_fixture(
            &provider,
            &article,
            ArticleAccessContext::default(),
            1_024,
        )
        .expect("full-text fixture should pass");
    }

    #[test]
    fn rejects_unsafe_redirects_and_unbounded_documents() {
        let redirect = ArticleRedirect {
            location: "file:///tmp/article.pdf".to_string(),
        };
        assert!(validate_article_redirect(&redirect).is_err());

        let oversized = ArticleFullTextResolution::Document(ArticleFullTextDocument {
            content_type: "application/pdf".to_string(),
            filename: Some("article.pdf".to_string()),
            bytes: vec![0; 2],
        });
        assert!(validate_full_text_resolution(&oversized, 1).is_err());
    }

    #[test]
    fn rejects_provider_identity_and_journal_mismatches() {
        let catalog = catalog();
        let mut article_mismatch = batch();
        article_mismatch.articles[0].catalog_id = "issn-0000-0000".to_string();
        assert!(validate_provider_batch(&catalog, &article_mismatch)
            .expect_err("article catalog mismatch should fail")
            .to_string()
            .contains("article"));

        let mut journal_mismatch = batch();
        journal_mismatch.journal.observed_title = Some("Different Journal".to_string());
        assert!(validate_provider_batch(&catalog, &journal_mismatch)
            .expect_err("journal title mismatch should fail")
            .to_string()
            .contains("title"));
    }

    #[test]
    fn rejects_noncanonical_identifiers_and_incomplete_articles() {
        let catalog = catalog();
        let mut invalid_doi = batch();
        invalid_doi.articles[0].doi = Some("HTTPS://doi.org/10.1000/CANONICAL".to_string());
        assert!(validate_provider_batch(&catalog, &invalid_doi)
            .expect_err("URL DOI should fail")
            .to_string()
            .contains("DOI"));

        let mut incomplete_article = batch();
        let article = &mut incomplete_article.articles[0];
        article.doi = None;
        article.publication_year = None;
        article.date = None;
        assert!(validate_provider_batch(&catalog, &incomplete_article)
            .expect_err("incomplete identity should fail")
            .to_string()
            .contains("identity"));
    }
}
