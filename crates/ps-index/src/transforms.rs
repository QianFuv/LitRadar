//! Python-compatible transformations for scholarly index records.

use std::collections::{BTreeMap, BTreeSet};

use ps_domain::stable_sqlite_id;
use ps_sources::normalize_doi;
use serde_json::{json, Map, Value};

/// Normalized CSV journal row.
pub type CsvRow = BTreeMap<String, String>;

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
    use serde_json::json;

    use super::{
        build_scholarly_article_record, build_scholarly_issue_record, crossref_publication_date,
    };

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
}
