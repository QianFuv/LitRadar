//! Scholarly index orchestration backed by fixture source clients.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use litradar_sources::{
    normalize_doi, FixtureScholarlyTransport, ScholarlyClient, ScholarlyFixtureData,
    ScholarlyTransport, SourceError,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;

use crate::manifest::{build_change_manifest, write_change_manifest};
use crate::schema::{
    apply_article_changes, mark_article_listing_ready, mark_journal_done, mark_year_done,
    open_index_db, optimize_index_db, persist_index_run_stats, upsert_issues, upsert_journal,
    upsert_meta, with_immediate_index_transaction, ChangeEventContext,
};
use crate::stats::{IndexRunStats, PathCountIncrements};
use crate::transforms::{
    apply_crossref_resolved_meta, apply_openalex_resolved_meta, build_journal_id,
    build_meta_record, build_openalex_crossref_work, build_openalex_journal_row,
    build_scholarly_article_record, build_scholarly_issue_record, build_scholarly_journal_record,
    candidate_issns_from_row, doi_values_from_works, embedded_openalex_work, issue_year,
    journal_title_from_row, split_article_records_by_authors, ArticleRecord, CsvRow, IssueRecord,
};

/// Scholarly fixture index run configuration.
#[derive(Debug, Clone)]
pub struct ScholarlyIndexConfig {
    /// Source CSV path.
    pub csv_path: PathBuf,
    /// Scholarly source fixture JSON path.
    pub fixture_path: PathBuf,
    /// Output SQLite database path.
    pub output_db_path: PathBuf,
    /// Optional change manifest output path.
    pub manifest_path: Option<PathBuf>,
    /// Deterministic run id.
    pub run_id: String,
    /// Deterministic timestamp.
    pub timestamp: String,
    /// Whether Semantic Scholar enrichment is configured.
    pub has_semantic_scholar_key: bool,
}

/// Scholarly fixture index outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScholarlyIndexOutcome {
    /// Final run status.
    pub status: String,
    /// Run identifier.
    pub run_id: String,
    /// Output database path.
    pub db_path: String,
    /// Optional manifest path.
    pub manifest_path: Option<String>,
    /// Written article ids.
    pub written_article_ids: Vec<i64>,
    /// Source attempt count.
    pub source_attempt_count: usize,
}

/// Scholarly index workflow errors.
#[derive(Debug)]
pub enum ScholarlyIndexError {
    /// IO operation failed.
    Io(std::io::Error),
    /// Fixture JSON parsing failed.
    Json(serde_json::Error),
    /// SQLite operation failed.
    Sqlite(rusqlite::Error),
    /// Source operation failed.
    Source(SourceError),
    /// Journal row is invalid.
    InvalidJournal(String),
}

impl fmt::Display for ScholarlyIndexError {
    /// Format the scholarly index error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Source(error) => write!(formatter, "{error}"),
            Self::InvalidJournal(message) => formatter.write_str(message),
        }
    }
}

impl Error for ScholarlyIndexError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Source(error) => Some(error),
            Self::InvalidJournal(_) => None,
        }
    }
}

impl From<std::io::Error> for ScholarlyIndexError {
    /// Convert IO errors into index errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ScholarlyIndexError {
    /// Convert JSON errors into index errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<rusqlite::Error> for ScholarlyIndexError {
    /// Convert SQLite errors into index errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<SourceError> for ScholarlyIndexError {
    /// Convert source errors into index errors.
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

/// Run the Scholarly fixture index pipeline.
///
/// # Arguments
///
/// * `config` - Index run configuration.
///
/// # Returns
///
/// Index run outcome.
pub fn run_scholarly_fixture_index(
    config: &ScholarlyIndexConfig,
) -> Result<ScholarlyIndexOutcome, ScholarlyIndexError> {
    let rows = read_csv_rows(&config.csv_path)?;
    let fixture_data: ScholarlyFixtureData =
        serde_json::from_str(&fs::read_to_string(&config.fixture_path)?)?;
    if let Some(parent) = config.output_db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = open_index_db(&config.output_db_path)?;
    let csv_file = config
        .csv_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("journals.csv")
        .to_string();
    let db_name = config
        .output_db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("index.sqlite")
        .to_string();
    let transport = FixtureScholarlyTransport::new(fixture_data);
    let mut client = ScholarlyClient::new(transport, config.has_semantic_scholar_key);
    let mut stats = IndexRunStats::new(
        config.run_id.clone(),
        csv_file.clone(),
        config.timestamp.clone(),
    );
    let mut all_written_articles = Vec::new();

    for row in rows {
        let journal_id = build_journal_id(&row).ok_or_else(|| {
            ScholarlyIndexError::InvalidJournal(format!(
                "Scholarly journal missing id: {}",
                journal_title_from_row(&row)
            ))
        })?;
        let journal_title = journal_title_from_row(&row);
        let path_key = stats.start_path(
            "scholarly",
            "journal",
            Some(journal_id),
            journal_title.clone(),
            config.timestamp.clone(),
        );
        let attempt_start = client.attempts().len();
        let result = process_scholarly_row(
            &connection,
            &mut client,
            &row,
            &csv_file,
            journal_id,
            &config.timestamp,
            None,
        );
        let attempts = client.attempts()[attempt_start..].to_vec();
        stats.record_source_attempts(&attempts, Some(journal_id), &journal_title);
        match result {
            Ok(ProcessOutcome {
                written_articles,
                works_count,
                issues_count,
                deleted_article_count,
            }) => {
                stats.record_path_counts(
                    &path_key,
                    PathCountIncrements {
                        works_count,
                        issues_count,
                        articles_written_count: written_articles.len() as i64,
                        articles_deleted_no_authors_count: deleted_article_count,
                        ..PathCountIncrements::default()
                    },
                );
                stats.finish_path(&path_key, "succeeded", config.timestamp.clone(), None);
                all_written_articles.extend(written_articles);
            }
            Err(error) => {
                stats.finish_path(
                    &path_key,
                    "failed",
                    config.timestamp.clone(),
                    Some(&error.to_string()),
                );
                stats.finish("failed", config.timestamp.clone(), Some(error.to_string()));
                persist_index_run_stats(&connection, &stats)?;
                return Err(error);
            }
        }
    }

    stats.finish("succeeded", config.timestamp.clone(), None);
    persist_index_run_stats(&connection, &stats)?;
    all_written_articles.sort_by_key(|article| article.article_id);
    let manifest_path = if let Some(path) = &config.manifest_path {
        let manifest = build_change_manifest(
            &db_name,
            &config.run_id,
            &config.timestamp,
            &all_written_articles,
        );
        write_change_manifest(&manifest, path)?;
        Some(path.display().to_string())
    } else {
        None
    };
    mark_article_listing_ready(&connection, &config.timestamp)?;
    optimize_index_db(&connection)?;

    Ok(ScholarlyIndexOutcome {
        status: "succeeded".to_string(),
        run_id: config.run_id.clone(),
        db_path: config.output_db_path.display().to_string(),
        manifest_path,
        written_article_ids: all_written_articles
            .iter()
            .map(|article| article.article_id)
            .collect(),
        source_attempt_count: client.attempts().len(),
    })
}

#[derive(Debug)]
pub(crate) struct ProcessOutcome {
    pub(crate) written_articles: Vec<ArticleRecord>,
    pub(crate) works_count: i64,
    pub(crate) issues_count: i64,
    pub(crate) deleted_article_count: i64,
}

/// Process one Scholarly CSV row into an index database.
pub(crate) fn process_scholarly_row<T>(
    connection: &Connection,
    client: &mut ScholarlyClient<T>,
    row: &CsvRow,
    csv_file: &str,
    journal_id: i64,
    timestamp: &str,
    change_event_context: Option<&ChangeEventContext>,
) -> Result<ProcessOutcome, ScholarlyIndexError>
where
    T: ScholarlyTransport,
{
    let issn_candidates = candidate_issns_from_row(row);
    if issn_candidates.is_empty() {
        return Err(ScholarlyIndexError::InvalidJournal(format!(
            "Scholarly journal missing ISSN: {}",
            journal_title_from_row(row)
        )));
    }

    let mut issn = issn_candidates[0].clone();
    let mut works = Vec::new();
    let mut openalex_source = None;
    let mut fallback_openalex_by_doi = BTreeMap::new();
    let mut last_404 = None;
    let mut did_crossref_success = false;
    for candidate in &issn_candidates {
        match client.fetch_journal_works(candidate, None) {
            Ok(candidate_works) => {
                issn = candidate.clone();
                works = candidate_works;
                did_crossref_success = true;
                break;
            }
            Err(SourceError::HttpStatus {
                status_code: 404, ..
            }) => {
                last_404 = Some(candidate.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }

    if !did_crossref_success && last_404.is_some() {
        openalex_source = client.fetch_openalex_source_by_issns(&issn_candidates)?;
        if openalex_source.is_none() {
            openalex_source =
                client.fetch_openalex_source_by_title(&journal_title_from_row(row))?;
        }
        let source = openalex_source.clone().ok_or_else(|| {
            ScholarlyIndexError::InvalidJournal(format!(
                "Scholarly journal has no available ISSN candidate: {}",
                journal_title_from_row(row)
            ))
        })?;
        let source_id = source
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let openalex_source_works = client.fetch_openalex_works_by_source(&source_id, None)?;
        let journal_row = build_openalex_journal_row(row, &source);
        issn = journal_row.get("issn").cloned().unwrap_or(issn);
        let source_issns = candidate_issns_from_row(&journal_row);
        works = openalex_source_works
            .iter()
            .filter_map(|work| build_openalex_crossref_work(work, &source_issns))
            .collect();
        fallback_openalex_by_doi = openalex_source_works
            .iter()
            .filter_map(|work| normalize_doi(work.get("doi")).map(|doi| (doi, work.clone())))
            .collect();
        if works.is_empty() {
            return Err(ScholarlyIndexError::InvalidJournal(format!(
                "OpenAlex fallback returned no usable works: {}",
                journal_title_from_row(row)
            )));
        }
    }

    let mut journal_row = row.clone();
    if let Some(source) = &openalex_source {
        journal_row = build_openalex_journal_row(row, source);
    } else {
        journal_row.insert("issn".to_string(), issn.clone());
    }
    let mut journal_record = build_scholarly_journal_record(journal_id, &journal_row, &works);
    journal_record.has_articles = Some(if works.is_empty() { 0 } else { 1 });
    let mut meta_record = build_meta_record(journal_id, csv_file, row);
    if let Some(source) = &openalex_source {
        apply_openalex_resolved_meta(&mut meta_record, source);
    } else {
        apply_crossref_resolved_meta(&mut meta_record, &issn, &journal_record);
    }
    let journal_title = journal_record
        .title
        .clone()
        .or_else(|| row.get("title").cloned())
        .unwrap_or_default();

    let dois = doi_values_from_works(&works);
    let openalex_by_doi = if fallback_openalex_by_doi.is_empty() && !dois.is_empty() {
        client.fetch_openalex_by_dois(&dois, 100)?
    } else {
        fallback_openalex_by_doi
    };
    let semantic_scholar_by_doi = if dois.is_empty() {
        BTreeMap::new()
    } else {
        client.fetch_semantic_scholar_by_dois(&dois, 500)?
    };

    let mut issue_records_by_id = BTreeMap::<i64, IssueRecord>::new();
    let mut article_records = Vec::new();
    let mut years = BTreeSet::new();
    for work in &works {
        let issue_record = build_scholarly_issue_record(journal_id, work);
        let issue_id = issue_record.as_ref().map(|record| record.issue_id);
        if let Some(issue_record) = issue_record {
            if let Some(year) = issue_year(&issue_record) {
                years.insert(year);
            }
            issue_records_by_id.insert(issue_record.issue_id, issue_record);
        }
        let doi = normalize_doi(work.get("DOI"));
        let openalex_work = doi
            .as_ref()
            .and_then(|doi| openalex_by_doi.get(doi))
            .or_else(|| embedded_openalex_work(work));
        let semantic_scholar_work = doi
            .as_ref()
            .and_then(|doi| semantic_scholar_by_doi.get(doi));
        if let Some(mut article_record) = build_scholarly_article_record(
            work,
            openalex_work,
            semantic_scholar_work,
            journal_id,
            issue_id,
        ) {
            backfill_semantic_scholar_abstract(&mut article_record, semantic_scholar_work);
            article_records.push(article_record);
        }
    }

    let issue_records = issue_records_by_id.into_values().collect::<Vec<_>>();
    let (article_records, deleted_article_ids) = split_article_records_by_authors(article_records);
    with_immediate_index_transaction(connection, |transaction| {
        upsert_journal(transaction, &journal_record)?;
        upsert_meta(transaction, &meta_record)?;
        if !issue_records.is_empty() {
            upsert_issues(transaction, &issue_records)?;
        }
        apply_article_changes(
            transaction,
            &article_records,
            &deleted_article_ids,
            &journal_title,
            change_event_context,
        )?;
        for year in &years {
            mark_year_done(transaction, journal_id, *year, timestamp)?;
        }
        mark_journal_done(transaction, journal_id, timestamp)?;
        Ok::<(), ScholarlyIndexError>(())
    })?;

    Ok(ProcessOutcome {
        works_count: works.len() as i64,
        issues_count: issue_records.len() as i64,
        deleted_article_count: deleted_article_ids.len() as i64,
        written_articles: article_records,
    })
}

fn backfill_semantic_scholar_abstract(
    article_record: &mut ArticleRecord,
    semantic_scholar_work: Option<&Value>,
) {
    if article_record
        .abstract_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return;
    }
    article_record.abstract_text = semantic_scholar_work
        .and_then(|work| work.get("abstract"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
}

fn read_csv_rows(path: &Path) -> Result<Vec<CsvRow>, ScholarlyIndexError> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return Ok(Vec::new());
    };
    let headers = parse_csv_line(header_line);
    let mut rows = Vec::new();
    for line in lines {
        let values = parse_csv_line(line);
        let mut row = CsvRow::new();
        for (index, header) in headers.iter().enumerate() {
            row.insert(
                header.clone(),
                values.get(index).cloned().unwrap_or_default(),
            );
        }
        rows.push(row);
    }
    Ok(rows)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut characters = line.chars().peekable();
    let mut inside_quotes = false;
    while let Some(character) = characters.next() {
        match character {
            '"' if inside_quotes && characters.peek() == Some(&'"') => {
                current.push('"');
                characters.next();
            }
            '"' => inside_quotes = !inside_quotes,
            ',' if !inside_quotes => {
                values.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    values.push(current.trim().to_string());
    values
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{run_scholarly_fixture_index, ScholarlyIndexConfig};

    #[test]
    fn fixture_index_writes_database_and_manifest() {
        let temp = tempdir().expect("temp dir should create");
        let csv_path = temp.path().join("journals.csv");
        let fixture_path = temp.path().join("fixture.json");
        let db_path = temp.path().join("index.sqlite");
        let manifest_path = temp.path().join("index.changes.json");
        fs::write(
            &csv_path,
            "source,title,issn,id,area,all_issns\nscholarly,Cognition,1873-7838,1873-7838,testing,1873-7838\n",
        )
        .expect("csv should write");
        fs::write(
            &fixture_path,
            r#"{
              "crossref_status": 404,
              "openalex_source_by_issns": {
                "id": "https://openalex.org/S88198767",
                "display_name": "Cognition",
                "issn_l": "0010-0277",
                "issn": ["0010-0277", "1873-7838"]
              },
              "openalex_source_works": [{
                "id": "https://openalex.org/W1",
                "doi": "https://doi.org/10.1/fallback",
                "title": "Fallback Article",
                "publication_date": "2026-07-03",
                "biblio": {"volume": "12", "issue": "1", "first_page": "1", "last_page": "9"},
                "authorships": [{"author": {"display_name": "Fallback Author"}}],
                "open_access": {"is_oa": true},
                "best_oa_location": {
                  "pdf_url": "https://openalex.test/fallback.pdf",
                  "landing_page_url": "https://openalex.test/fallback"
                }
              }],
              "semantic_scholar_by_doi": {
                "10.1/fallback": {
                  "externalIds": {"DOI": "10.1/fallback"},
                  "isOpenAccess": true,
                  "openAccessPdf": {"url": "https://s2.test/fallback.pdf"},
                  "abstract": "S2 abstract."
                }
              }
            }"#,
        )
        .expect("fixture should write");

        let outcome = run_scholarly_fixture_index(&ScholarlyIndexConfig {
            csv_path,
            fixture_path,
            output_db_path: db_path.clone(),
            manifest_path: Some(manifest_path.clone()),
            run_id: "run-1".into(),
            timestamp: "2026-07-03T00:00:00Z".into(),
            has_semantic_scholar_key: true,
        })
        .expect("fixture index should succeed");

        assert_eq!(outcome.status, "succeeded");
        assert!(db_path.exists());
        assert!(manifest_path.exists());
        assert_eq!(outcome.source_attempt_count, 4);
        let connection = Connection::open(db_path).expect("db should open");
        let (status, updated_at): (String, String) = connection
            .query_row(
                "SELECT status, updated_at FROM listing_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("listing ready state should exist");
        assert_eq!(status, "ready");
        assert_eq!(updated_at, "2026-07-03T00:00:00Z");
    }
}
