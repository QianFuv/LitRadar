//! CNKI index orchestration backed by fixture source clients.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ps_sources::{
    CnkiClient, CnkiFixtureData, CnkiSourceError, CnkiTransport, FixtureCnkiTransport,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;

use crate::manifest::{build_change_manifest, write_change_manifest};
use crate::schema::{
    delete_articles, get_completed_years, get_journal_issue_ids_with_articles, init_index_db,
    is_journal_complete, mark_article_listing_ready, mark_journal_done, mark_year_done,
    persist_index_run_stats, refresh_article_listing_for_articles, upsert_article_search,
    upsert_articles, upsert_issues, upsert_journal, upsert_meta,
};
use crate::stats::{IndexRunStats, PathCountIncrements};
use crate::transforms::{
    build_cnki_article_record, build_cnki_issue_record, build_cnki_journal_record,
    build_journal_id, build_meta_record, journal_title_from_row, split_article_records_by_authors,
    ArticleRecord, CsvRow, IssueRecord,
};

/// CNKI fixture index run configuration.
#[derive(Debug, Clone)]
pub struct CnkiIndexConfig {
    /// Source CSV path.
    pub csv_path: PathBuf,
    /// CNKI source fixture JSON path.
    pub fixture_path: PathBuf,
    /// Output SQLite database path.
    pub output_db_path: PathBuf,
    /// Optional change manifest output path.
    pub manifest_path: Option<PathBuf>,
    /// Deterministic run id.
    pub run_id: String,
    /// Deterministic timestamp.
    pub timestamp: String,
    /// Whether completed journals and years may be skipped.
    pub resume: bool,
    /// Whether to refresh only recent existing issue ranges.
    pub update: bool,
    /// Number of issues processed together.
    pub issue_batch_size: usize,
}

/// CNKI fixture index outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CnkiIndexOutcome {
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

/// CNKI index workflow errors.
#[derive(Debug)]
pub enum CnkiIndexError {
    /// IO operation failed.
    Io(std::io::Error),
    /// Fixture JSON parsing failed.
    Json(serde_json::Error),
    /// SQLite operation failed.
    Sqlite(rusqlite::Error),
    /// Source operation failed.
    Source(CnkiSourceError),
    /// Journal row is invalid.
    InvalidJournal(String),
}

impl fmt::Display for CnkiIndexError {
    /// Format the CNKI index error.
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

impl Error for CnkiIndexError {
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

impl From<std::io::Error> for CnkiIndexError {
    /// Convert IO errors into index errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for CnkiIndexError {
    /// Convert JSON errors into index errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<rusqlite::Error> for CnkiIndexError {
    /// Convert SQLite errors into index errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<CnkiSourceError> for CnkiIndexError {
    /// Convert source errors into index errors.
    fn from(error: CnkiSourceError) -> Self {
        Self::Source(error)
    }
}

/// Run the CNKI fixture index pipeline.
///
/// # Arguments
///
/// * `config` - Index run configuration.
///
/// # Returns
///
/// Index run outcome.
pub fn run_cnki_fixture_index(
    config: &CnkiIndexConfig,
) -> Result<CnkiIndexOutcome, CnkiIndexError> {
    let rows = read_csv_rows(&config.csv_path)?;
    let fixture_data: CnkiFixtureData =
        serde_json::from_str(&fs::read_to_string(&config.fixture_path)?)?;
    if let Some(parent) = config.output_db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(&config.output_db_path)?;
    init_index_db(&connection)?;
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
    let transport = FixtureCnkiTransport::new(fixture_data);
    let mut client = CnkiClient::new(transport);
    let mut stats = IndexRunStats::new(
        config.run_id.clone(),
        csv_file.clone(),
        config.timestamp.clone(),
    );
    let mut all_written_articles = Vec::new();

    for row in rows {
        let journal_id = build_journal_id(&row).ok_or_else(|| {
            CnkiIndexError::InvalidJournal(format!(
                "CNKI journal missing id: {}",
                journal_title_from_row(&row)
            ))
        })?;
        let journal_title = journal_title_from_row(&row);
        let path_key = stats.start_path(
            "cnki",
            "journal",
            Some(journal_id),
            journal_title.clone(),
            config.timestamp.clone(),
        );
        let attempt_start = client.attempts().len();
        let result = process_cnki_row(
            &connection,
            &mut client,
            &row,
            &csv_file,
            journal_id,
            config,
        );
        let attempts = client.attempts()[attempt_start..].to_vec();
        stats.record_source_attempts_for_source(
            "cnki",
            &attempts,
            Some(journal_id),
            &journal_title,
        );
        match result {
            Ok(ProcessOutcome {
                status,
                written_articles,
                issues_count,
                article_summaries_count,
                article_details_count,
                deleted_article_count,
            }) => {
                stats.record_path_counts(
                    &path_key,
                    PathCountIncrements {
                        issues_count,
                        article_summaries_count,
                        article_details_count,
                        articles_written_count: written_articles.len() as i64,
                        articles_deleted_no_authors_count: deleted_article_count,
                        ..PathCountIncrements::default()
                    },
                );
                stats.finish_path(&path_key, &status, config.timestamp.clone(), None);
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
            &config.output_db_path,
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

    Ok(CnkiIndexOutcome {
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
    pub(crate) status: String,
    pub(crate) written_articles: Vec<ArticleRecord>,
    pub(crate) issues_count: i64,
    pub(crate) article_summaries_count: i64,
    pub(crate) article_details_count: i64,
    pub(crate) deleted_article_count: i64,
}

/// Process one CNKI CSV row into an index database.
pub(crate) fn process_cnki_row<T>(
    connection: &Connection,
    client: &mut CnkiClient<T>,
    row: &CsvRow,
    csv_file: &str,
    journal_id: i64,
    config: &CnkiIndexConfig,
) -> Result<ProcessOutcome, CnkiIndexError>
where
    T: CnkiTransport,
{
    if config.resume && !config.update && is_journal_complete(connection, journal_id)? {
        return Ok(ProcessOutcome::resumed());
    }

    let details = client.resolve_journal(row)?.ok_or_else(|| {
        CnkiIndexError::InvalidJournal(format!(
            "No CNKI details for journal: {}",
            journal_title_from_row(row)
        ))
    })?;
    let journal_record = build_cnki_journal_record(journal_id, row, Some(&details));
    let meta_record = build_meta_record(journal_id, csv_file, row);
    let journal_title = journal_record
        .title
        .clone()
        .or_else(|| row.get("title").cloned())
        .unwrap_or_default();
    let journal_code = json_text(details.get("pykm")).ok_or_else(|| {
        CnkiIndexError::InvalidJournal(format!(
            "CNKI journal missing code: {}",
            journal_title_from_row(row)
        ))
    })?;

    upsert_journal(connection, &journal_record)?;
    upsert_meta(connection, &meta_record)?;

    let issues = client.year_issues(&details)?;
    if issues.is_empty() {
        return Err(CnkiIndexError::InvalidJournal(format!(
            "No CNKI publication years for journal {journal_code}"
        )));
    }

    let mut issue_records_by_year = BTreeMap::<i64, Vec<IssueRecord>>::new();
    let mut issue_pairs_by_year = BTreeMap::<i64, Vec<(i64, Value)>>::new();
    for issue in &issues {
        let Some(record) = build_cnki_issue_record(journal_id, &journal_code, issue) else {
            continue;
        };
        let Some(year) = record.publication_year else {
            continue;
        };
        issue_records_by_year
            .entry(year)
            .or_default()
            .push(record.clone());
        issue_pairs_by_year
            .entry(year)
            .or_default()
            .push((record.issue_id, issue.clone()));
    }

    let selected_update_issue_ids = if config.update {
        let ordered_issue_ids = issue_pairs_by_year
            .iter()
            .rev()
            .flat_map(|(_, pairs)| pairs.iter().map(|(issue_id, _)| *issue_id))
            .collect::<Vec<_>>();
        let existing_issue_ids = get_journal_issue_ids_with_articles(connection, journal_id)?;
        Some(
            select_recent_update_issue_ids(&ordered_issue_ids, &existing_issue_ids)
                .into_iter()
                .collect::<BTreeSet<_>>(),
        )
    } else {
        None
    };
    let completed_years = if config.resume && !config.update {
        get_completed_years(connection, journal_id)?
    } else {
        BTreeSet::new()
    };
    let years_to_process = issue_pairs_by_year
        .keys()
        .rev()
        .filter(|year| {
            if let Some(selected) = &selected_update_issue_ids {
                issue_pairs_by_year
                    .get(year)
                    .map(|pairs| {
                        pairs
                            .iter()
                            .any(|(issue_id, _)| selected.contains(issue_id))
                    })
                    .unwrap_or(false)
            } else {
                !completed_years.contains(year)
            }
        })
        .copied()
        .collect::<Vec<_>>();

    let mut written_articles = Vec::new();
    let mut article_summaries_count = 0;
    let mut article_details_count = 0;
    let mut deleted_article_count = 0;
    let batch_size = config.issue_batch_size.max(1);
    for year in years_to_process {
        let mut issue_records = issue_records_by_year
            .get(&year)
            .cloned()
            .unwrap_or_default();
        let mut issue_pairs = issue_pairs_by_year.get(&year).cloned().unwrap_or_default();
        if let Some(selected) = &selected_update_issue_ids {
            issue_records.retain(|record| selected.contains(&record.issue_id));
            issue_pairs.retain(|(issue_id, _)| selected.contains(issue_id));
        }
        if !issue_records.is_empty() {
            upsert_issues(connection, &issue_records)?;
        }
        for batch in issue_pairs.chunks(batch_size) {
            let mut batch_records = Vec::new();
            for (issue_id, issue) in batch {
                let summaries = client.issue_articles(&details, issue)?;
                article_summaries_count += summaries.len() as i64;
                for summary in summaries {
                    let Some(article_url) = json_text(summary.get("article_url")) else {
                        continue;
                    };
                    let platform_id = json_text(summary.get("platform_id"));
                    let detail = client.article_detail(&article_url, platform_id.as_deref())?;
                    article_details_count += 1;
                    if let Some(record) = build_cnki_article_record(
                        Some(&detail),
                        &summary,
                        journal_id,
                        Some(*issue_id),
                    ) {
                        batch_records.push(record);
                    }
                }
            }
            let (batch_records, deleted_article_ids) =
                split_article_records_by_authors(batch_records);
            deleted_article_count += deleted_article_ids.len() as i64;
            if !deleted_article_ids.is_empty() {
                delete_articles(connection, &deleted_article_ids)?;
            }
            if !batch_records.is_empty() {
                upsert_articles(connection, &batch_records)?;
                upsert_article_search(connection, &batch_records, &journal_title)?;
                refresh_article_listing_for_articles(
                    connection,
                    &batch_records
                        .iter()
                        .map(|record| record.article_id)
                        .collect::<Vec<_>>(),
                )?;
                written_articles.extend(batch_records);
            }
        }
        mark_year_done(connection, journal_id, year, &config.timestamp)?;
    }

    mark_journal_done(connection, journal_id, &config.timestamp)?;
    connection.execute_batch("PRAGMA optimize;")?;

    Ok(ProcessOutcome {
        status: "succeeded".to_string(),
        written_articles,
        issues_count: issues.len() as i64,
        article_summaries_count,
        article_details_count,
        deleted_article_count,
    })
}

impl ProcessOutcome {
    fn resumed() -> Self {
        Self {
            status: "resumed".to_string(),
            written_articles: Vec::new(),
            issues_count: 0,
            article_summaries_count: 0,
            article_details_count: 0,
            deleted_article_count: 0,
        }
    }
}

/// Select issues from the newest issue through the latest indexed issue.
///
/// # Arguments
///
/// * `issue_ids` - Issue ids in newest-to-oldest upstream order.
/// * `existing_issue_ids` - Issue ids that already have indexed articles.
///
/// # Returns
///
/// Issue ids to refresh during an update run.
pub fn select_recent_update_issue_ids(
    issue_ids: &[i64],
    existing_issue_ids: &BTreeSet<i64>,
) -> Vec<i64> {
    if existing_issue_ids.is_empty() {
        return issue_ids.to_vec();
    }
    let mut selected = Vec::new();
    for issue_id in issue_ids {
        selected.push(*issue_id);
        if existing_issue_ids.contains(issue_id) {
            break;
        }
    }
    selected
}

fn read_csv_rows(path: &Path) -> Result<Vec<CsvRow>, CnkiIndexError> {
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

fn json_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(value) => non_empty(value),
        other => non_empty(&other.to_string()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    use ps_sources::{CnkiClient, CnkiFixtureData, FixtureCnkiTransport};
    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::cnki::{
        process_cnki_row, run_cnki_fixture_index, select_recent_update_issue_ids, CnkiIndexConfig,
    };
    use crate::schema::{init_index_db, mark_journal_done};
    use crate::transforms::CsvRow;

    #[test]
    fn selects_update_issues_through_first_existing_issue() {
        let existing = BTreeSet::from([20, 10]);

        assert_eq!(
            select_recent_update_issue_ids(&[30, 20, 10], &existing),
            vec![30, 20]
        );
        assert_eq!(
            select_recent_update_issue_ids(&[30, 20, 10], &BTreeSet::new()),
            vec![30, 20, 10]
        );
    }

    #[test]
    fn resume_skips_completed_journal_before_source_calls() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let journal_id = 42;
        mark_journal_done(&connection, journal_id, "2026-07-05T00:00:00Z")
            .expect("journal state should be marked complete");
        let row = CsvRow::from([
            ("source".to_string(), "cnki".to_string()),
            ("title".to_string(), "Completed CNKI".to_string()),
            ("issn".to_string(), "1234-5678".to_string()),
            ("id".to_string(), "Completed CNKI".to_string()),
        ]);
        let mut client = CnkiClient::new(FixtureCnkiTransport::new(CnkiFixtureData::default()));
        let config = CnkiIndexConfig {
            csv_path: PathBuf::new(),
            fixture_path: PathBuf::new(),
            output_db_path: PathBuf::new(),
            manifest_path: None,
            run_id: "run-cnki-resume".to_string(),
            timestamp: "2026-07-05T00:00:00Z".to_string(),
            resume: true,
            update: false,
            issue_batch_size: 10,
        };

        let outcome = process_cnki_row(
            &connection,
            &mut client,
            &row,
            "journals.csv",
            journal_id,
            &config,
        )
        .expect("completed journal should resume");

        assert_eq!(outcome.status, "resumed");
        assert!(outcome.written_articles.is_empty());
        assert_eq!(client.attempts().len(), 0);
    }

    #[test]
    fn fixture_index_writes_cnki_database_and_manifest() {
        let temp = tempdir().expect("temp dir should create");
        let csv_path = temp.path().join("journals.csv");
        let fixture_path = temp.path().join("fixture.json");
        let db_path = temp.path().join("index.sqlite");
        let manifest_path = temp.path().join("index.changes.json");
        fs::write(
            &csv_path,
            "source,title,issn,id,area\ncnki,CNKI Test Journal,1234-5678,CNKI Test Journal,testing\n",
        )
        .expect("csv should write");
        fs::write(&fixture_path, fixture_json()).expect("fixture should write");

        let outcome = run_cnki_fixture_index(&CnkiIndexConfig {
            csv_path,
            fixture_path,
            output_db_path: db_path.clone(),
            manifest_path: Some(manifest_path.clone()),
            run_id: "run-cnki-1".into(),
            timestamp: "2026-07-03T00:00:00Z".into(),
            resume: false,
            update: false,
            issue_batch_size: 10,
        })
        .expect("fixture index should succeed");

        assert_eq!(outcome.status, "succeeded");
        assert!(manifest_path.exists());
        let connection = Connection::open(db_path).expect("db should open");
        init_index_db(&connection).expect("schema should initialize");
        let (listing_status, listing_updated_at): (String, String) = connection
            .query_row(
                "SELECT status, updated_at FROM listing_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("listing ready state should exist");
        let open_access: Option<i64> = connection
            .query_row("SELECT open_access FROM articles LIMIT 1", [], |row| {
                row.get(0)
            })
            .expect("article should exist");
        let full_text_file: Option<String> = connection
            .query_row("SELECT full_text_file FROM articles LIMIT 1", [], |row| {
                row.get(0)
            })
            .expect("article should exist");

        assert_eq!(outcome.source_attempt_count, 4);
        assert_eq!(listing_status, "ready");
        assert_eq!(listing_updated_at, "2026-07-03T00:00:00Z");
        assert_eq!(open_access, None);
        assert_eq!(full_text_file, None);
    }

    fn fixture_json() -> &'static str {
        r#"{
          "journal_detail_html": "<html><head><title>CNKI Test Journal - 中国知网</title></head><body><input id=\"pykm\" value=\"TEST\" /><input id=\"pCode\" value=\"CJFD\" /><input id=\"time\" value=\"token\" /><input id=\"shareChName\" value=\"CNKI Test Journal\" /><p>ISSN: 1234-5678</p><p>Combined IF: 1.5</p><img src=\"/images/journal-cover.jpg\" /></body></html>",
          "year_issues_html": "<div id=\"YearIssueTree\"><a id=\"yq202601\" value=\"202601\">2026 No.01</a></div>",
          "issue_articles_html": {
            "202601": "<dt class=\"tit\">Articles</dt><dd class=\"row\"><a href=\"/kcms2/article/abstract?v=1&filename=CNKI202601001\">CNKI article CNKI202601001</a><b name=\"encrypt\" id=\"CNKI202601001\"></b><span class=\"author\" title=\"Test Author\"></span><span class=\"company\" title=\"1-2\"></span>Free</dd>"
          },
          "article_detail_html": {
            "CNKI202601001": "<html><head><title>CNKI article CNKI202601001</title></head><body><input id=\"paramfilename\" value=\"CNKI202601001\" /><input id=\"paramdbcode\" value=\"CJFD\" /><input id=\"paramdbname\" value=\"CJFDLAST2026\" /><input id=\"abstract_text\" value=\"Test abstract.\" /><p class=\"title-one\">CNKI article CNKI202601001</p><h3 class=\"author\" id=\"authorpart\"><span>Test Author</span></h3><span class=\"rowtit\">Online Release Time:</span><p>2026-01-02</p><span class=\"rowtit\">DOI:</span><p>10.1/cnki</p><span class=\"rowtit\">Pages:</span><p>1-2</p><a href=\"/barnew/download/order?id=abc\">HTML阅读</a></body></html>"
          }
        }"#
    }
}
