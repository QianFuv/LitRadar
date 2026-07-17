//! Python-compatible transformations for scholarly index records.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use litradar_domain::{
    normalize_contract_issn, normalize_contract_text, stable_sqlite_id, JournalCatalogEntry,
    JournalRankings,
};
use litradar_provider::conformance::validate_catalog_entry;
use litradar_sources::normalize_doi;
use serde_json::{json, Map, Value};

/// Normalized CSV journal row.
pub type CsvRow = BTreeMap<String, String>;

/// Exact ordered column contract for maintained catalog CSV version 2.
pub const CATALOG_CSV_V2_COLUMNS: [&str; 15] = [
    "catalog_id",
    "title",
    "issn",
    "eissn",
    "all_issns",
    "title_aliases",
    "area",
    "utd_rank",
    "utd_rating",
    "abs_rank",
    "abs_rating",
    "fms_rank",
    "fms_rating",
    "fmscn_rank",
    "fmscn_rating",
];

/// Canonical catalog parsing or validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogContractError {
    message: String,
}

impl CatalogContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CatalogContractError {
    /// Format the catalog validation diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CatalogContractError {}

/// Build one canonical journal catalog entry from a version 2 CSV row.
///
/// # Arguments
///
/// * `csv_row` - CSV row keyed by its exact version 2 headers.
///
/// # Returns
///
/// Normalized canonical catalog entry or a validation failure.
pub fn build_catalog_entry(csv_row: &CsvRow) -> Result<JournalCatalogEntry, CatalogContractError> {
    validate_catalog_columns(csv_row)?;
    let catalog_id = required_catalog_id(csv_row)?;
    let title = required_catalog_text(csv_row, "title")?;
    let issn = optional_catalog_issn(csv_row, "issn")?;
    let eissn = optional_catalog_issn(csv_row, "eissn")?;
    let all_issns = catalog_issn_list(csv_row, "all_issns")?;
    let title_aliases = catalog_text_list(csv_row, "title_aliases")?;
    let entry = JournalCatalogEntry {
        catalog_id,
        title,
        issn,
        eissn,
        all_issns,
        title_aliases,
        area: optional_catalog_text(csv_row, "area"),
        rankings: JournalRankings {
            utd_rank: optional_catalog_text(csv_row, "utd_rank"),
            utd_rating: optional_catalog_text(csv_row, "utd_rating"),
            abs_rank: optional_catalog_text(csv_row, "abs_rank"),
            abs_rating: optional_catalog_text(csv_row, "abs_rating"),
            fms_rank: optional_catalog_text(csv_row, "fms_rank"),
            fms_rating: optional_catalog_text(csv_row, "fms_rating"),
            fmscn_rank: optional_catalog_text(csv_row, "fmscn_rank"),
            fmscn_rating: optional_catalog_text(csv_row, "fmscn_rating"),
        },
    };
    validate_catalog_entry(&entry).map_err(|error| CatalogContractError::new(error.to_string()))?;
    Ok(entry)
}

/// Validate and normalize all rows in one maintained catalog.
///
/// # Arguments
///
/// * `rows` - Version 2 CSV rows.
///
/// # Returns
///
/// Canonical entries with unique immutable catalog identifiers.
pub fn build_catalog_entries(
    rows: &[CsvRow],
) -> Result<Vec<JournalCatalogEntry>, CatalogContractError> {
    if rows.is_empty() {
        return Err(CatalogContractError::new(
            "canonical catalog must contain at least one journal",
        ));
    }
    let mut catalog_ids = BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let entry = build_catalog_entry(row).map_err(|error| {
                CatalogContractError::new(format!("catalog row {}: {error}", index + 2))
            })?;
            if !catalog_ids.insert(entry.catalog_id.clone()) {
                return Err(CatalogContractError::new(format!(
                    "catalog row {} duplicates catalog_id {}",
                    index + 2,
                    entry.catalog_id
                )));
            }
            Ok(entry)
        })
        .collect()
}

fn validate_catalog_columns(csv_row: &CsvRow) -> Result<(), CatalogContractError> {
    let expected = CATALOG_CSV_V2_COLUMNS
        .iter()
        .map(|column| (*column).to_string())
        .collect::<BTreeSet<_>>();
    let actual = csv_row.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(CatalogContractError::new(format!(
            "catalog row must use exact v2 columns; missing={missing:?}, unexpected={unexpected:?}"
        )));
    }
    Ok(())
}

fn required_catalog_id(csv_row: &CsvRow) -> Result<String, CatalogContractError> {
    let raw = csv_row
        .get("catalog_id")
        .expect("validated catalog row contains catalog_id");
    let normalized = normalize_contract_text(raw)
        .ok_or_else(|| CatalogContractError::new("catalog_id must not be blank"))?;
    if normalized != *raw {
        return Err(CatalogContractError::new(
            "catalog_id must already use canonical trimmed form",
        ));
    }
    Ok(normalized)
}

fn required_catalog_text(csv_row: &CsvRow, field: &str) -> Result<String, CatalogContractError> {
    optional_catalog_text(csv_row, field)
        .ok_or_else(|| CatalogContractError::new(format!("{field} must not be blank")))
}

fn optional_catalog_text(csv_row: &CsvRow, field: &str) -> Option<String> {
    csv_row
        .get(field)
        .and_then(|value| normalize_contract_text(value))
}

fn optional_catalog_issn(
    csv_row: &CsvRow,
    field: &str,
) -> Result<Option<String>, CatalogContractError> {
    let Some(value) = optional_catalog_text(csv_row, field) else {
        return Ok(None);
    };
    normalize_contract_issn(&value)
        .map(Some)
        .ok_or_else(|| CatalogContractError::new(format!("{field} contains an invalid ISSN")))
}

fn catalog_issn_list(csv_row: &CsvRow, field: &str) -> Result<Vec<String>, CatalogContractError> {
    let mut values = Vec::new();
    for value in csv_row
        .get(field)
        .expect("validated catalog row contains ISSN list")
        .split(';')
    {
        let Some(value) = normalize_contract_text(value) else {
            continue;
        };
        let issn = normalize_contract_issn(&value).ok_or_else(|| {
            CatalogContractError::new(format!("{field} contains an invalid ISSN"))
        })?;
        if !values.contains(&issn) {
            values.push(issn);
        }
    }
    Ok(values)
}

fn catalog_text_list(csv_row: &CsvRow, field: &str) -> Result<Vec<String>, CatalogContractError> {
    let mut values = Vec::new();
    for value in csv_row
        .get(field)
        .expect("validated catalog row contains text list")
        .split(';')
    {
        let Some(value) = normalize_contract_text(value) else {
            continue;
        };
        if values.contains(&value) {
            return Err(CatalogContractError::new(format!(
                "{field} contains a duplicate value"
            )));
        }
        values.push(value);
    }
    Ok(values)
}

/// Journal table record.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalRecord {
    /// Internal journal id.
    pub journal_id: i64,
    /// Source library id.
    pub library_id: String,
    /// Upstream platform journal id.
    pub platform_journal_id: Option<String>,
    /// Journal title.
    pub title: Option<String>,
    /// Primary ISSN.
    pub issn: Option<String>,
    /// Secondary ISSN.
    pub eissn: Option<String>,
    /// Scimago rank.
    pub scimago_rank: Option<f64>,
    /// Cover URL.
    pub cover_url: Option<String>,
    /// Availability flag.
    pub available: Option<i64>,
    /// TOC approval flag.
    pub toc_data_approved_and_live: Option<i64>,
    /// Article availability flag.
    pub has_articles: Option<i64>,
}

/// Journal metadata table record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaRecord {
    /// Internal journal id.
    pub journal_id: i64,
    /// Source CSV filename.
    pub source_csv: String,
    /// Journal area.
    pub area: Option<String>,
    /// CSV title.
    pub csv_title: Option<String>,
    /// CSV ISSN.
    pub csv_issn: Option<String>,
    /// CSV library id.
    pub csv_library: Option<String>,
    /// Resolved source name.
    pub resolved_source: Option<String>,
    /// Resolved source id.
    pub resolved_source_id: Option<String>,
    /// Resolved title.
    pub resolved_title: Option<String>,
    /// Resolved primary ISSN.
    pub resolved_issn: Option<String>,
    /// Resolved secondary ISSN.
    pub resolved_eissn: Option<String>,
}

/// Issue table record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRecord {
    /// Internal issue id.
    pub issue_id: i64,
    /// Internal journal id.
    pub journal_id: i64,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Issue title.
    pub title: Option<String>,
    /// Issue volume.
    pub volume: Option<String>,
    /// Issue number.
    pub number: Option<String>,
    /// Issue date.
    pub date: Option<String>,
    /// Valid issue flag.
    pub is_valid_issue: Option<i64>,
    /// Suppression flag.
    pub suppressed: Option<i64>,
    /// Embargo flag.
    pub embargoed: Option<i64>,
    /// Subscription flag.
    pub within_subscription: Option<i64>,
}

/// Article table record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleRecord {
    /// Internal article id.
    pub article_id: i64,
    /// Internal journal id.
    pub journal_id: i64,
    /// Internal issue id.
    pub issue_id: Option<i64>,
    /// Article title.
    pub title: Option<String>,
    /// Article date.
    pub date: Option<String>,
    /// Formatted authors.
    pub authors: Option<String>,
    /// Start page.
    pub start_page: Option<String>,
    /// End page.
    pub end_page: Option<String>,
    /// Abstract text.
    pub abstract_text: Option<String>,
    /// DOI.
    pub doi: Option<String>,
    /// PubMed id.
    pub pmid: Option<String>,
    /// Article permalink.
    pub permalink: Option<String>,
    /// Suppression flag.
    pub suppressed: Option<i64>,
    /// In-press flag.
    pub in_press: Option<i64>,
    /// Open access flag.
    pub open_access: Option<i64>,
    /// Upstream platform id.
    pub platform_id: Option<String>,
    /// Retraction DOI.
    pub retraction_doi: Option<String>,
    /// Library holdings flag.
    pub within_library_holdings: Option<i64>,
    /// Landing page URL.
    pub content_location: Option<String>,
    /// Full text URL.
    pub full_text_file: Option<String>,
}

/// Build a stable internal journal id from a CSV row.
///
/// # Arguments
///
/// * `csv_row` - Source CSV row.
///
/// # Returns
///
/// Stable SQLite journal id.
pub fn build_journal_id(csv_row: &CsvRow) -> Option<i64> {
    let source = source_from_row(csv_row);
    let source_id = csv_row
        .get("id")
        .filter(|value| !value.is_empty())
        .or_else(|| csv_row.get("issn").filter(|value| !value.is_empty()))
        .or_else(|| csv_row.get("title").filter(|value| !value.is_empty()))?;
    Some(stable_sqlite_id(source_id, format!("{source}:journal")))
}

/// Resolve the source library id from a CSV row.
///
/// # Arguments
///
/// * `csv_row` - Source CSV row.
///
/// # Returns
///
/// Normalized source id.
pub fn source_from_row(csv_row: &CsvRow) -> String {
    csv_row
        .get("source")
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "scholarly".to_string())
}

/// Build ordered ISSN candidates from a CSV row.
///
/// # Arguments
///
/// * `csv_row` - Source CSV row.
///
/// # Returns
///
/// Unique ISSN candidates.
pub fn candidate_issns_from_row(csv_row: &CsvRow) -> Vec<String> {
    let mut candidates = Vec::new();
    for key in ["issn", "all_issns", "id"] {
        let Some(value) = csv_row.get(key) else {
            continue;
        };
        for part in value.split(';') {
            let candidate = part.trim();
            if is_issn_candidate(candidate) && !candidates.iter().any(|item| item == candidate) {
                candidates.push(candidate.to_string());
            }
        }
    }
    candidates
}

/// Resolve a display title for a CSV journal row.
///
/// # Arguments
///
/// * `csv_row` - Source CSV row.
///
/// # Returns
///
/// Journal title fallback.
pub fn journal_title_from_row(csv_row: &CsvRow) -> String {
    csv_row
        .get("title")
        .filter(|value| !value.is_empty())
        .or_else(|| csv_row.get("id").filter(|value| !value.is_empty()))
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Build journal CSV metadata.
///
/// # Arguments
///
/// * `journal_id` - Internal journal id.
/// * `csv_file` - Source CSV filename.
/// * `csv_row` - Source CSV row.
///
/// # Returns
///
/// Journal metadata record.
pub fn build_meta_record(journal_id: i64, csv_file: &str, csv_row: &CsvRow) -> MetaRecord {
    MetaRecord {
        journal_id,
        source_csv: csv_file.to_string(),
        area: optional_row_value(csv_row, "area"),
        csv_title: optional_row_value(csv_row, "title"),
        csv_issn: optional_row_value(csv_row, "issn"),
        csv_library: Some(source_from_row(csv_row)),
        resolved_source: None,
        resolved_source_id: None,
        resolved_title: None,
        resolved_issn: None,
        resolved_eissn: None,
    }
}

/// Build a scholarly journal table record.
///
/// # Arguments
///
/// * `journal_id` - Internal journal id.
/// * `csv_row` - Source CSV row.
/// * `crossref_works` - Crossref-like works.
///
/// # Returns
///
/// Journal table record.
pub fn build_scholarly_journal_record(
    journal_id: i64,
    csv_row: &CsvRow,
    crossref_works: &[Value],
) -> JournalRecord {
    let issn = optional_row_value(csv_row, "issn");
    let mut eissn = None;
    for work in crossref_works {
        let Some(issns) = work.get("ISSN").and_then(Value::as_array) else {
            continue;
        };
        for candidate in issns {
            let text = clean_text(Some(candidate));
            if text.is_some() && text != issn {
                eissn = text;
                break;
            }
        }
        if eissn.is_some() {
            break;
        }
    }
    JournalRecord {
        journal_id,
        library_id: source_from_row(csv_row),
        platform_journal_id: optional_row_value(csv_row, "id").or_else(|| issn.clone()),
        title: optional_row_value(csv_row, "title"),
        issn,
        eissn,
        scimago_rank: None,
        cover_url: None,
        available: Some(1),
        toc_data_approved_and_live: None,
        has_articles: Some(if crossref_works.is_empty() { 0 } else { 1 }),
    }
}

/// Build a CNKI journal table record.
///
/// # Arguments
///
/// * `journal_id` - Internal journal id.
/// * `csv_row` - Source CSV row.
/// * `details` - Optional CNKI journal detail payload.
///
/// # Returns
///
/// Journal table record.
pub fn build_cnki_journal_record(
    journal_id: i64,
    csv_row: &CsvRow,
    details: Option<&Value>,
) -> JournalRecord {
    JournalRecord {
        journal_id,
        library_id: source_from_row(csv_row),
        platform_journal_id: details
            .and_then(|value| clean_text(value.get("pykm")))
            .or_else(|| optional_row_value(csv_row, "id")),
        title: details
            .and_then(|value| clean_text(value.get("title")))
            .or_else(|| optional_row_value(csv_row, "title")),
        issn: details
            .and_then(|value| clean_text(value.get("issn")))
            .or_else(|| optional_row_value(csv_row, "issn")),
        eissn: None,
        scimago_rank: details
            .and_then(|value| clean_text(value.get("impact_factor")))
            .and_then(|value| value.parse::<f64>().ok()),
        cover_url: details.and_then(|value| clean_text(value.get("cover_url"))),
        available: Some(i64::from(details.is_some())),
        toc_data_approved_and_live: None,
        has_articles: Some(i64::from(details.is_some())),
    }
}

/// Build a scholarly issue record from Crossref-like metadata.
///
/// # Arguments
///
/// * `journal_id` - Internal journal id.
/// * `work` - Crossref-like work.
///
/// # Returns
///
/// Issue record.
pub fn build_scholarly_issue_record(journal_id: i64, work: &Value) -> Option<IssueRecord> {
    let date = crossref_publication_date(work)?;
    let year = date.get(..4)?.parse::<i64>().ok()?;
    let volume = clean_text(work.get("volume"));
    let number = clean_text(work.get("issue")).unwrap_or_else(|| "in-press".to_string());
    let issue_id = stable_sqlite_id(
        format!(
            "{journal_id}:{year}:{}:{number}",
            volume.clone().unwrap_or_default()
        ),
        "scholarly:issue",
    );
    Some(IssueRecord {
        issue_id,
        journal_id,
        publication_year: Some(year),
        title: Some(
            format!("{year} {} {number}", volume.clone().unwrap_or_default())
                .trim()
                .to_string(),
        ),
        volume,
        number: (number != "in-press").then_some(number),
        date: Some(date),
        is_valid_issue: Some(1),
        suppressed: None,
        embargoed: None,
        within_subscription: None,
    })
}

/// Build a CNKI issue table record.
///
/// # Arguments
///
/// * `journal_id` - Internal journal id.
/// * `journal_code` - CNKI journal code.
/// * `issue` - CNKI issue payload.
///
/// # Returns
///
/// Issue table record.
pub fn build_cnki_issue_record(
    journal_id: i64,
    journal_code: &str,
    issue: &Value,
) -> Option<IssueRecord> {
    let year = issue.get("year")?.as_i64()?;
    let number = clean_text(issue.get("number"))?;
    let issue_id = stable_sqlite_id(format!("{journal_code}:{year}:{number}"), "cnki:issue");
    Some(IssueRecord {
        issue_id,
        journal_id,
        publication_year: Some(year),
        title: clean_text(issue.get("title")).or_else(|| Some(format!("{year}年第{number}期"))),
        volume: None,
        number: Some(number),
        date: Some(format!("{year:04}-01-01")),
        is_valid_issue: Some(1),
        suppressed: None,
        embargoed: None,
        within_subscription: None,
    })
}

/// Build resolved metadata fields for Crossref.
///
/// # Arguments
///
/// * `meta` - Mutable metadata record.
/// * `source_id` - Resolved source id.
/// * `journal` - Journal record.
pub fn apply_crossref_resolved_meta(
    meta: &mut MetaRecord,
    source_id: &str,
    journal: &JournalRecord,
) {
    meta.resolved_source = Some("crossref".to_string());
    meta.resolved_source_id = Some(source_id.to_string());
    meta.resolved_title = journal.title.clone();
    meta.resolved_issn = journal.issn.clone();
    meta.resolved_eissn = journal.eissn.clone();
}

/// Build a CSV row resolved through OpenAlex source metadata.
///
/// # Arguments
///
/// * `csv_row` - Original CSV row.
/// * `openalex_source` - OpenAlex source payload.
///
/// # Returns
///
/// Resolved CSV row.
pub fn build_openalex_journal_row(csv_row: &CsvRow, openalex_source: &Value) -> CsvRow {
    let mut row = csv_row.clone();
    let issns = openalex_issns(openalex_source);
    row.insert(
        "id".to_string(),
        openalex_short_id(openalex_source.get("id"))
            .or_else(|| optional_row_value(csv_row, "id"))
            .unwrap_or_default(),
    );
    row.insert(
        "issn".to_string(),
        clean_text(openalex_source.get("issn_l"))
            .or_else(|| issns.first().cloned())
            .unwrap_or_default(),
    );
    row.insert("all_issns".to_string(), issns.join(";"));
    row.insert(
        "title".to_string(),
        clean_text(openalex_source.get("display_name"))
            .or_else(|| optional_row_value(csv_row, "title"))
            .unwrap_or_default(),
    );
    row
}

/// Apply OpenAlex resolved metadata fields.
///
/// # Arguments
///
/// * `meta` - Mutable metadata record.
/// * `openalex_source` - OpenAlex source payload.
pub fn apply_openalex_resolved_meta(meta: &mut MetaRecord, openalex_source: &Value) {
    let issns = openalex_issns(openalex_source);
    let primary = clean_text(openalex_source.get("issn_l")).or_else(|| issns.first().cloned());
    let eissn = issns
        .iter()
        .find(|issn| Some((*issn).clone()) != primary)
        .cloned();
    meta.resolved_source = Some("openalex".to_string());
    meta.resolved_source_id = openalex_short_id(openalex_source.get("id"));
    meta.resolved_title = clean_text(openalex_source.get("display_name"));
    meta.resolved_issn = primary;
    meta.resolved_eissn = eissn;
}

/// Convert an OpenAlex work into the Crossref-like shape used by the indexer.
///
/// # Arguments
///
/// * `openalex_work` - OpenAlex work payload.
/// * `source_issns` - Resolved source ISSNs.
///
/// # Returns
///
/// Crossref-like work payload.
pub fn build_openalex_crossref_work(
    openalex_work: &Value,
    source_issns: &[String],
) -> Option<Value> {
    let openalex_id = clean_text(openalex_work.get("id"));
    let doi = normalize_doi(openalex_work.get("doi"));
    let platform_url = doi
        .as_ref()
        .map(|value| format!("https://doi.org/{value}"))
        .or(openalex_id.clone())?;
    let biblio = openalex_work
        .get("biblio")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let page = page_range_from_biblio(&biblio);
    let published = crossref_date_from_iso(openalex_work.get("publication_date"));
    let mut object = Map::new();
    object.insert(
        "DOI".to_string(),
        doi.map(Value::String).unwrap_or(Value::Null),
    );
    object.insert("URL".to_string(), Value::String(platform_url));
    object.insert(
        "title".to_string(),
        clean_text(openalex_work.get("title"))
            .map(|title| json!([title]))
            .unwrap_or_else(|| json!([])),
    );
    object.insert(
        "ISSN".to_string(),
        Value::Array(source_issns.iter().cloned().map(Value::String).collect()),
    );
    object.insert(
        "published".to_string(),
        published.clone().unwrap_or(Value::Null),
    );
    object.insert("issued".to_string(), published.unwrap_or(Value::Null));
    object.insert(
        "volume".to_string(),
        clean_text(biblio.get("volume"))
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "issue".to_string(),
        clean_text(biblio.get("issue"))
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "page".to_string(),
        page.map(Value::String).unwrap_or(Value::Null),
    );
    object.insert("_openalex_work".to_string(), openalex_work.clone());
    Some(Value::Object(object))
}

/// Build a scholarly article record from Crossref-like and enrichment payloads.
///
/// # Arguments
///
/// * `work` - Crossref-like work payload.
/// * `openalex_work` - OpenAlex enrichment payload.
/// * `semantic_scholar_work` - Semantic Scholar enrichment payload.
/// * `journal_id` - Internal journal id.
/// * `issue_id` - Internal issue id.
///
/// # Returns
///
/// Article record.
pub fn build_scholarly_article_record(
    work: &Value,
    openalex_work: Option<&Value>,
    semantic_scholar_work: Option<&Value>,
    journal_id: i64,
    issue_id: Option<i64>,
) -> Option<ArticleRecord> {
    let doi = normalize_doi(work.get("DOI"));
    let platform_id = doi.clone().or_else(|| clean_text(work.get("URL")))?;
    let article_id = stable_sqlite_id(&platform_id, "scholarly:article");
    let page = clean_text(work.get("page")).or_else(|| clean_text(work.get("article-number")));
    let (start_page, end_page) = split_page_range(page.as_deref());
    let openalex_abstract = openalex_work
        .and_then(|value| value.get("abstract_inverted_index"))
        .and_then(restore_openalex_abstract);
    let semantic_scholar_oa_pdf = semantic_scholar_work
        .and_then(|value| value.get("openAccessPdf"))
        .and_then(Value::as_object);
    let openalex_location = openalex_work
        .and_then(|value| value.get("best_oa_location"))
        .and_then(Value::as_object);
    let doi_url = doi.as_ref().map(|value| format!("https://doi.org/{value}"));
    let full_text_url = first_text_values([
        semantic_scholar_oa_pdf.and_then(|value| value.get("url")),
        openalex_location.and_then(|value| value.get("pdf_url")),
        openalex_location.and_then(|value| value.get("landing_page_url")),
    ]);
    let landing_page_url = openalex_location
        .and_then(|value| value.get("landing_page_url"))
        .and_then(clean_text_value)
        .or_else(|| clean_text(work.get("URL")))
        .or_else(|| doi_url.clone());
    let is_open_access = i64::from(
        bool_int(semantic_scholar_work.and_then(|value| value.get("isOpenAccess"))) == Some(1)
            || bool_int(
                openalex_work
                    .and_then(|value| value.get("open_access"))
                    .and_then(|value| value.get("is_oa")),
            ) == Some(1),
    );

    Some(ArticleRecord {
        article_id,
        journal_id,
        issue_id,
        title: first_text(work.get("title"))
            .or_else(|| openalex_work.and_then(|value| clean_text(value.get("title")))),
        date: crossref_publication_date(work)
            .or_else(|| openalex_work.and_then(|value| clean_text(value.get("publication_date")))),
        authors: format_crossref_authors(work.get("author")).or_else(|| {
            openalex_work.and_then(|value| format_openalex_authors(value.get("authorships")))
        }),
        start_page,
        end_page,
        abstract_text: strip_markup(clean_text(work.get("abstract")).as_deref())
            .or(openalex_abstract),
        doi,
        pmid: openalex_work
            .and_then(|value| value.get("ids"))
            .and_then(|value| value.get("pmid"))
            .and_then(clean_text_value),
        permalink: doi_url.clone().or(landing_page_url.clone()),
        suppressed: None,
        in_press: issue_id.is_none().then_some(1),
        open_access: Some(is_open_access),
        platform_id: Some(platform_id),
        retraction_doi: relation_doi(work.get("relation")),
        within_library_holdings: None,
        content_location: landing_page_url,
        full_text_file: full_text_url,
    })
}

/// Build a CNKI article table record.
///
/// # Arguments
///
/// * `detail` - Optional CNKI article detail payload.
/// * `summary` - CNKI article summary payload.
/// * `journal_id` - Internal journal id.
/// * `issue_id` - Internal issue id.
///
/// # Returns
///
/// Article table record.
pub fn build_cnki_article_record(
    detail: Option<&Value>,
    summary: &Value,
    journal_id: i64,
    issue_id: Option<i64>,
) -> Option<ArticleRecord> {
    let platform_id = detail
        .and_then(|value| clean_text(value.get("platform_id")))
        .or_else(|| clean_text(summary.get("platform_id")));
    let article_key = platform_id
        .clone()
        .or_else(|| clean_text(summary.get("article_url")))?;
    let article_id = stable_sqlite_id(article_key, "cnki:article");
    let page = detail
        .and_then(|value| clean_text(value.get("pages")))
        .or_else(|| clean_text(summary.get("pages")));
    let (start_page, end_page) = split_page_range(page.as_deref());
    let doi = detail.and_then(|value| normalize_doi(value.get("doi")));
    let permalink = detail
        .and_then(|value| clean_text(value.get("permalink")))
        .or_else(|| clean_text(summary.get("article_url")));
    let content_location = detail
        .and_then(|value| clean_text(value.get("content_location")))
        .or_else(|| permalink.clone());

    Some(ArticleRecord {
        article_id,
        journal_id,
        issue_id,
        title: detail
            .and_then(|value| clean_text(value.get("title")))
            .or_else(|| clean_text(summary.get("title"))),
        date: detail
            .and_then(|value| clean_text(value.get("online_release_date")))
            .or_else(|| clean_text(summary.get("date"))),
        authors: detail
            .and_then(|value| clean_text(value.get("authors")))
            .or_else(|| clean_text(summary.get("authors"))),
        start_page,
        end_page,
        abstract_text: detail.and_then(|value| clean_text(value.get("abstract"))),
        doi,
        pmid: None,
        permalink,
        suppressed: None,
        in_press: None,
        open_access: None,
        platform_id,
        retraction_doi: None,
        within_library_holdings: None,
        content_location,
        full_text_file: None,
    })
}

/// Split article records into writable and deleted-no-authors groups.
///
/// # Arguments
///
/// * `records` - Article records.
///
/// # Returns
///
/// Writable records and article ids to delete.
pub fn split_article_records_by_authors(
    records: Vec<ArticleRecord>,
) -> (Vec<ArticleRecord>, Vec<i64>) {
    let mut kept = Vec::new();
    let mut deleted = Vec::new();
    for record in records {
        if record
            .authors
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            kept.push(record);
        } else {
            deleted.push(record.article_id);
        }
    }
    (kept, deleted)
}

/// Extract normalized DOI values from works.
///
/// # Arguments
///
/// * `works` - Crossref-like works.
///
/// # Returns
///
/// Normalized DOI values in work order.
pub fn doi_values_from_works(works: &[Value]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    works
        .iter()
        .filter_map(|work| normalize_doi(work.get("DOI")))
        .filter(|doi| seen.insert(doi.clone()))
        .collect()
}

/// Extract the embedded OpenAlex work payload from a Crossref-like work.
///
/// # Arguments
///
/// * `work` - Crossref-like work.
///
/// # Returns
///
/// Embedded OpenAlex work.
pub fn embedded_openalex_work(work: &Value) -> Option<&Value> {
    work.get("_openalex_work").filter(|value| value.is_object())
}

/// Extract the publication year from an issue record.
///
/// # Arguments
///
/// * `issue` - Issue record.
///
/// # Returns
///
/// Publication year.
pub fn issue_year(issue: &IssueRecord) -> Option<i64> {
    issue.publication_year
}

/// Extract an OpenAlex short id from a JSON value.
///
/// # Arguments
///
/// * `value` - OpenAlex URL or id value.
///
/// # Returns
///
/// Short id.
pub fn openalex_short_id(value: Option<&Value>) -> Option<String> {
    clean_text(value).and_then(|text| text.rsplit('/').next().map(str::to_string))
}

/// Extract ordered ISSN values from an OpenAlex source payload.
///
/// # Arguments
///
/// * `openalex_source` - OpenAlex source payload.
///
/// # Returns
///
/// Ordered unique ISSN values.
pub fn openalex_issns(openalex_source: &Value) -> Vec<String> {
    let mut issns = Vec::new();
    if let Some(primary) = clean_text(openalex_source.get("issn_l")) {
        issns.push(primary);
    }
    if let Some(values) = openalex_source.get("issn").and_then(Value::as_array) {
        for value in values {
            let Some(issn) = clean_text(Some(value)) else {
                continue;
            };
            if !issns.contains(&issn) {
                issns.push(issn);
            }
        }
    }
    issns
}

/// Extract a normalized Crossref publication date.
///
/// # Arguments
///
/// * `work` - Crossref-like work.
///
/// # Returns
///
/// Date string.
pub fn crossref_publication_date(work: &Value) -> Option<String> {
    for key in ["published-print", "published-online", "published", "issued"] {
        if let Some(date) = crossref_date_parts(work.get(key)) {
            return Some(date);
        }
    }
    None
}

fn is_issn_candidate(value: &str) -> bool {
    let text = value.trim().replace('-', "").to_uppercase();
    text.len() == 8
        && text
            .chars()
            .take(7)
            .all(|character| character.is_ascii_digit())
        && text
            .chars()
            .nth(7)
            .map(|character| character.is_ascii_digit() || character == 'X')
            .unwrap_or(false)
}

fn optional_row_value(csv_row: &CsvRow, key: &str) -> Option<String> {
    csv_row.get(key).and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn crossref_date_parts(value: Option<&Value>) -> Option<String> {
    let first = value?.get("date-parts")?.as_array()?.first()?.as_array()?;
    let year = first.first()?.as_i64()?;
    let month = first.get(1).and_then(Value::as_i64).unwrap_or(1);
    let day = first.get(2).and_then(Value::as_i64).unwrap_or(1);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn crossref_date_from_iso(value: Option<&Value>) -> Option<Value> {
    let text = clean_text(value)?;
    let mut parts = text.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1);
    let day = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1);
    Some(json!({"date-parts": [[year, month, day]]}))
}

fn page_range_from_biblio(biblio: &Map<String, Value>) -> Option<String> {
    let first_page = clean_text(biblio.get("first_page"));
    let last_page = clean_text(biblio.get("last_page"));
    match (first_page, last_page) {
        (Some(first), Some(last)) if first != last => Some(format!("{first}-{last}")),
        (Some(first), _) => Some(first),
        (_, Some(last)) => Some(last),
        _ => None,
    }
}

fn split_page_range(value: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(text) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };
    for separator in ["-", "–", "—"] {
        if let Some((start, end)) = text.split_once(separator) {
            return (non_empty(start), non_empty(end));
        }
    }
    (Some(text.to_string()), None)
}

fn format_crossref_authors(value: Option<&Value>) -> Option<String> {
    let names = value?
        .as_array()?
        .iter()
        .filter_map(|item| {
            let given = clean_text(item.get("given"));
            let family = clean_text(item.get("family"));
            non_empty(
                &[given, family]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("; "))
}

fn format_openalex_authors(value: Option<&Value>) -> Option<String> {
    let names = value?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("author"))
        .filter_map(|author| clean_text(author.get("display_name")))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("; "))
}

fn restore_openalex_abstract(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let mut positions = Vec::new();
    for (word, indexes) in object {
        let Some(indexes) = indexes.as_array() else {
            continue;
        };
        for index in indexes {
            if let Some(index) = index.as_i64() {
                positions.push((index, word.clone()));
            }
        }
    }
    if positions.is_empty() {
        return None;
    }
    positions.sort_by_key(|(index, _)| *index);
    Some(
        positions
            .into_iter()
            .map(|(_, word)| word)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn strip_markup(value: Option<&str>) -> Option<String> {
    let text = value?;
    let mut output = String::with_capacity(text.len());
    let mut inside_tag = false;
    for character in text.chars() {
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
    non_empty(&collapse_whitespace(&decode_basic_html_entities(&output)))
}

fn relation_doi(value: Option<&Value>) -> Option<String> {
    for relation in value?.as_object()?.values() {
        let Some(items) = relation.as_array() else {
            continue;
        };
        for item in items {
            if let Some(doi) = normalize_doi(item.get("id")) {
                return Some(doi);
            }
        }
    }
    None
}

fn bool_int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(value) => Some(i64::from(value.as_i64().unwrap_or(0) != 0)),
        Value::String(value) => match value.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" => Some(1),
            "false" | "0" | "no" => Some(0),
            _ => None,
        },
        _ => None,
    }
}

fn first_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(values) = value.as_array() {
        for item in values {
            if let Some(text) = clean_text(Some(item)) {
                return Some(text);
            }
        }
        return None;
    }
    clean_text(Some(value))
}

fn first_text_values<const N: usize>(values: [Option<&Value>; N]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find_map(|value| clean_text(Some(value)))
}

fn clean_text(value: Option<&Value>) -> Option<String> {
    clean_text_value(value?)
}

fn clean_text_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => non_empty(text),
        other => non_empty(&other.to_string()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_basic_html_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{
        build_catalog_entries, build_catalog_entry, build_cnki_article_record,
        build_cnki_issue_record, build_cnki_journal_record, build_scholarly_article_record,
        build_scholarly_issue_record, crossref_publication_date, CsvRow, CATALOG_CSV_V2_COLUMNS,
    };

    fn catalog_row() -> CsvRow {
        CsvRow::from([
            ("catalog_id".to_string(), "issn-1234-5679".to_string()),
            ("title".to_string(), "Canonical Journal".to_string()),
            ("issn".to_string(), "12345679".to_string()),
            ("eissn".to_string(), String::new()),
            ("all_issns".to_string(), "1234-5679".to_string()),
            (
                "title_aliases".to_string(),
                "Canonical J.; Journal Canonical".to_string(),
            ),
            ("area".to_string(), "Systems".to_string()),
            ("utd_rank".to_string(), String::new()),
            ("utd_rating".to_string(), String::new()),
            ("abs_rank".to_string(), String::new()),
            ("abs_rating".to_string(), String::new()),
            ("fms_rank".to_string(), String::new()),
            ("fms_rating".to_string(), String::new()),
            ("fmscn_rank".to_string(), String::new()),
            ("fmscn_rating".to_string(), String::new()),
        ])
    }

    fn parse_catalog_fixture(text: &str) -> Vec<CsvRow> {
        let mut lines = text.lines();
        let headers = parse_csv_test_line(lines.next().expect("catalog fixture header"));
        lines
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let values = parse_csv_test_line(line);
                headers
                    .iter()
                    .enumerate()
                    .map(|(index, header)| {
                        (
                            header.clone(),
                            values.get(index).cloned().unwrap_or_default(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .collect()
    }

    fn parse_csv_test_line(line: &str) -> Vec<String> {
        let mut values = Vec::new();
        let mut current = String::new();
        let mut characters = line.chars().peekable();
        let mut is_quoted = false;
        while let Some(character) = characters.next() {
            match character {
                '"' if is_quoted && characters.peek() == Some(&'"') => {
                    current.push('"');
                    characters.next();
                }
                '"' => is_quoted = !is_quoted,
                ',' if !is_quoted => {
                    values.push(current.clone());
                    current.clear();
                }
                _ => current.push(character),
            }
        }
        values.push(current);
        values
    }

    #[test]
    fn canonical_catalog_parser_normalizes_and_rejects_forbidden_columns() {
        let entry = build_catalog_entry(&catalog_row()).expect("canonical row should pass");
        assert_eq!(entry.issn.as_deref(), Some("1234-5679"));
        assert_eq!(entry.title_aliases.len(), 2);

        let mut forbidden = catalog_row();
        forbidden.insert("source".to_string(), "cnki".to_string());
        assert!(build_catalog_entry(&forbidden)
            .expect_err("provider column should fail")
            .to_string()
            .contains("source"));

        let mut forbidden = catalog_row();
        forbidden.insert("detail_url".to_string(), "https://example.test".to_string());
        assert!(build_catalog_entry(&forbidden)
            .expect_err("URL column should fail")
            .to_string()
            .contains("detail_url"));
    }

    #[test]
    fn canonical_catalog_parser_rejects_invalid_and_duplicate_ids() {
        let mut invalid_issn = catalog_row();
        invalid_issn.insert("issn".to_string(), "1234-5678".to_string());
        assert!(build_catalog_entry(&invalid_issn).is_err());

        let mut blank_id = catalog_row();
        blank_id.insert("catalog_id".to_string(), String::new());
        assert!(build_catalog_entry(&blank_id).is_err());

        let rows = vec![catalog_row(), catalog_row()];
        assert!(build_catalog_entries(&rows)
            .expect_err("duplicate immutable ID should fail")
            .to_string()
            .contains("duplicates"));
    }

    #[test]
    fn maintained_catalogs_use_v2_and_contain_all_959_entries() {
        let fixtures = [
            (
                include_str!("../../../data/meta/ccf_computer_journals.csv"),
                291,
            ),
            (include_str!("../../../data/meta/chinese_journals.csv"), 94),
            (include_str!("../../../data/meta/english_journals.csv"), 574),
        ];
        let mut total = 0;
        for (fixture, expected) in fixtures {
            let header = parse_csv_test_line(fixture.lines().next().expect("catalog header"));
            assert_eq!(header, CATALOG_CSV_V2_COLUMNS);
            let rows = parse_catalog_fixture(fixture);
            assert_eq!(rows.len(), expected);
            assert_eq!(
                build_catalog_entries(&rows)
                    .expect("catalog should pass")
                    .len(),
                expected
            );
            total += expected;
        }
        assert_eq!(total, 959);
    }

    #[test]
    fn scholarly_issue_ids_use_python_compatible_prefix() {
        let work = json!({
            "DOI": "10.1/a",
            "published": {"date-parts": [[2025, 2, 3]]},
            "volume": "12",
            "issue": "1"
        });

        let issue = build_scholarly_issue_record(42, &work).expect("issue should build");

        assert_eq!(issue.publication_year, Some(2025));
        assert_eq!(issue.date.as_deref(), Some("2025-02-03"));
    }

    #[test]
    fn article_enrichment_prefers_crossref_then_openalex_then_s2() {
        let work = json!({
            "DOI": "https://doi.org/10.1/A",
            "URL": "https://doi.org/10.1/A",
            "title": ["Crossref Title"],
            "published": {"date-parts": [[2025, 2, 3]]},
            "page": "1-9",
            "author": [{"given": "Ada", "family": "Lovelace"}]
        });
        let openalex = json!({
            "abstract_inverted_index": {"OpenAlex": [0], "abstract.": [1]},
            "best_oa_location": {"landing_page_url": "https://openalex.test/a"}
        });
        let s2 = json!({
            "isOpenAccess": true,
            "openAccessPdf": {"url": "https://s2.test/a.pdf"}
        });

        let article = build_scholarly_article_record(&work, Some(&openalex), Some(&s2), 7, Some(8))
            .expect("article should build");

        assert_eq!(article.doi.as_deref(), Some("10.1/a"));
        assert_eq!(article.authors.as_deref(), Some("Ada Lovelace"));
        assert_eq!(article.start_page.as_deref(), Some("1"));
        assert_eq!(article.end_page.as_deref(), Some("9"));
        assert_eq!(
            article.full_text_file.as_deref(),
            Some("https://s2.test/a.pdf")
        );
        assert_eq!(article.open_access, Some(1));
    }

    #[test]
    fn crossref_publication_date_uses_python_key_order() {
        let work = json!({
            "issued": {"date-parts": [[2024]]},
            "published-online": {"date-parts": [[2025, 6]]}
        });

        assert_eq!(
            crossref_publication_date(&work).as_deref(),
            Some("2025-06-01")
        );
    }

    #[test]
    fn cnki_records_match_python_compatibility_rules() {
        let row = CsvRow::from([
            ("source".to_string(), "cnki".to_string()),
            ("title".to_string(), "CNKI Test Journal".to_string()),
            ("issn".to_string(), "1234-5678".to_string()),
            ("id".to_string(), "CNKI Test Journal".to_string()),
        ]);
        let details = json!({
            "pykm": "TEST",
            "title": "CNKI Test Journal",
            "issn": "1234-5678",
            "impact_factor": "1.5",
            "cover_url": "https://oversea.cnki.net/cover.jpg"
        });
        let issue = json!({
            "year": 2026,
            "number": "01",
            "title": "2026 No.01",
            "year_issue": "202601"
        });
        let summary = json!({
            "article_url": "https://oversea.cnki.net/kcms2/article/abstract?filename=CNKI202601001",
            "platform_id": "CNKI202601001",
            "title": "CNKI Article",
            "authors": "Test Author",
            "pages": "1-2",
            "is_free": 1,
            "date": "2026-01-01"
        });
        let detail = json!({
            "platform_id": "CNKI202601001",
            "title": "CNKI Article",
            "authors": "Test Author",
            "abstract": "Test abstract.",
            "doi": "https://doi.org/10.1/CNKI",
            "online_release_date": "2026-01-02",
            "pages": "1-2",
            "html_read_url": "https://oversea.cnki.net/barnew/download/order?id=abc",
            "permalink": "https://oversea.cnki.net/openlink/detail?filename=CNKI202601001",
            "content_location": "https://oversea.cnki.net/openlink/detail?filename=CNKI202601001"
        });

        let journal = build_cnki_journal_record(1, &row, Some(&details));
        let issue = build_cnki_issue_record(1, "TEST", &issue).expect("issue should build");
        let article = build_cnki_article_record(Some(&detail), &summary, 1, Some(issue.issue_id))
            .expect("article should build");

        assert_eq!(journal.library_id, "cnki");
        assert_eq!(journal.platform_journal_id.as_deref(), Some("TEST"));
        assert_eq!(journal.scimago_rank, Some(1.5));
        assert_eq!(issue.publication_year, Some(2026));
        assert_eq!(issue.number.as_deref(), Some("01"));
        assert_eq!(article.doi.as_deref(), Some("10.1/cnki"));
        assert_eq!(article.open_access, None);
        assert_eq!(article.full_text_file, None);
        assert_eq!(
            article.content_location.as_deref(),
            Some("https://oversea.cnki.net/openlink/detail?filename=CNKI202601001")
        );
    }
}
