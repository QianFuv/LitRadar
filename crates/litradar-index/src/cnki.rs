//! CNKI index orchestration backed by fixture source clients.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use litradar_sources::{
    CnkiClient, CnkiFixtureData, CnkiSourceError, CnkiTransport, FixtureCnkiTransport,
    SourceAttempt,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;

use crate::changes::{write_change_manifest_from_events, ChangeWriteError};
use crate::schema::{
    apply_article_changes, get_completed_years, get_journal_issue_ids_with_articles,
    is_journal_complete, mark_article_listing_ready, mark_journal_done, mark_year_done,
    open_index_db, optimize_index_db, persist_index_run_stats, upsert_issues, upsert_journal,
    upsert_meta, with_immediate_index_transaction, ChangeEventContext, IndexRunLeaseContext,
};
use crate::stats::{IndexRunStats, PathCountIncrements};
use crate::transforms::{
    build_cnki_article_record, build_cnki_issue_record, build_cnki_journal_record,
    build_journal_id, build_meta_record, journal_title_from_row, split_article_records_by_authors,
    CsvRow, IssueRecord,
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
    /// Number of article detail workers per issue batch.
    pub worker_count: usize,
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
    /// Written article count.
    pub written_article_count: i64,
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
    /// Article detail worker failed.
    Worker(String),
    /// Journal row is invalid.
    InvalidJournal(String),
    /// Live run lease ownership was lost.
    Lease(String),
}

impl fmt::Display for CnkiIndexError {
    /// Format the CNKI index error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Source(error) => write!(formatter, "{error}"),
            Self::Worker(message) => formatter.write_str(message),
            Self::InvalidJournal(message) => formatter.write_str(message),
            Self::Lease(message) => formatter.write_str(message),
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
            Self::Worker(_) => None,
            Self::InvalidJournal(_) => None,
            Self::Lease(_) => None,
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

impl From<ChangeWriteError> for CnkiIndexError {
    /// Convert streamed manifest errors into index errors.
    fn from(error: ChangeWriteError) -> Self {
        match error {
            ChangeWriteError::Sqlite(error) => Self::Sqlite(error),
            ChangeWriteError::Io(error) => Self::Io(error),
            ChangeWriteError::Json(error) => Self::Json(error),
        }
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
    let transport = FixtureCnkiTransport::new(fixture_data);
    let mut client = CnkiClient::new(transport);
    let mut stats = IndexRunStats::new(
        config.run_id.clone(),
        csv_file.clone(),
        config.timestamp.clone(),
    );
    let mut written_article_count = 0;
    let mut source_attempt_count = 0;
    let change_event_context = config
        .manifest_path
        .as_ref()
        .map(|_| ChangeEventContext::new(&config.run_id, "fixture-0", &config.timestamp, false));

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
        let result = {
            let mut attempt_sink = |attempts: &[SourceAttempt]| {
                source_attempt_count += attempts.len();
                stats.record_source_attempts_for_source(
                    "cnki",
                    attempts,
                    Some(journal_id),
                    &journal_title,
                );
            };
            process_cnki_row(
                &connection,
                &mut client,
                &row,
                CnkiProcessContext {
                    csv_file: &csv_file,
                    journal_id,
                    config,
                    change_event_context: change_event_context.as_ref(),
                    lease_context: None,
                    attempt_sink: &mut attempt_sink,
                },
            )
        };
        match result {
            Ok(ProcessOutcome {
                status,
                written_article_count: journal_written_article_count,
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
                        articles_written_count: journal_written_article_count,
                        articles_deleted_no_authors_count: deleted_article_count,
                        ..PathCountIncrements::default()
                    },
                );
                stats.finish_path(&path_key, &status, config.timestamp.clone(), None);
                written_article_count += journal_written_article_count;
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
    let manifest_path = if let Some(path) = &config.manifest_path {
        write_change_manifest_from_events(
            &connection,
            &db_name,
            &config.run_id,
            &config.timestamp,
            path,
        )?;
        Some(path.display().to_string())
    } else {
        None
    };
    mark_article_listing_ready(&connection, &config.timestamp)?;
    optimize_index_db(&connection)?;

    Ok(CnkiIndexOutcome {
        status: "succeeded".to_string(),
        run_id: config.run_id.clone(),
        db_path: config.output_db_path.display().to_string(),
        manifest_path,
        written_article_count,
        source_attempt_count,
    })
}

#[derive(Debug)]
pub(crate) struct ProcessOutcome {
    pub(crate) status: String,
    pub(crate) written_article_count: i64,
    pub(crate) issues_count: i64,
    pub(crate) article_summaries_count: i64,
    pub(crate) article_details_count: i64,
    pub(crate) deleted_article_count: i64,
}

/// Per-journal CNKI processing context.
pub(crate) struct CnkiProcessContext<'a> {
    pub(crate) csv_file: &'a str,
    pub(crate) journal_id: i64,
    pub(crate) config: &'a CnkiIndexConfig,
    pub(crate) change_event_context: Option<&'a ChangeEventContext>,
    pub(crate) lease_context: Option<&'a IndexRunLeaseContext>,
    pub(crate) attempt_sink: &'a mut dyn FnMut(&[SourceAttempt]),
}

#[derive(Debug, Clone)]
struct CnkiArticleDetailTask {
    order: usize,
    issue_id: i64,
    summary: Value,
    article_url: String,
    platform_id: Option<String>,
}

#[derive(Debug)]
struct CnkiArticleDetailResult {
    order: usize,
    issue_id: i64,
    summary: Value,
    detail: Value,
}

#[derive(Debug)]
struct CnkiArticleDetailWorkerOutput {
    task: CnkiArticleDetailTask,
    result: Result<Value, CnkiSourceError>,
    attempts: Vec<SourceAttempt>,
}

/// Process one CNKI CSV row into an index database.
pub(crate) fn process_cnki_row<T>(
    connection: &Connection,
    client: &mut CnkiClient<T>,
    row: &CsvRow,
    context: CnkiProcessContext<'_>,
) -> Result<ProcessOutcome, CnkiIndexError>
where
    T: CnkiTransport + Clone + Send + 'static,
{
    let CnkiProcessContext {
        csv_file,
        journal_id,
        config,
        change_event_context,
        lease_context,
        attempt_sink,
    } = context;
    if config.resume && !config.update && is_journal_complete(connection, journal_id)? {
        return Ok(ProcessOutcome::resumed());
    }

    let details =
        capture_cnki_attempts(client, attempt_sink, |client| client.resolve_journal(row))?
            .ok_or_else(|| {
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

    let issues =
        capture_cnki_attempts(client, attempt_sink, |client| client.year_issues(&details))?;
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

    let mut written_article_count = 0;
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
        let batch_count = issue_pairs.len().div_ceil(batch_size);
        for (batch_index, batch) in issue_pairs.chunks(batch_size).enumerate() {
            let mut batch_records = Vec::new();
            let mut detail_tasks = Vec::new();
            for (issue_id, issue) in batch {
                let summaries = capture_cnki_attempts(client, attempt_sink, |client| {
                    client.issue_articles(&details, issue)
                })?;
                article_summaries_count += summaries.len() as i64;
                for summary in summaries {
                    let Some(article_url) = json_text(summary.get("article_url")) else {
                        continue;
                    };
                    let platform_id = json_text(summary.get("platform_id"));
                    detail_tasks.push(CnkiArticleDetailTask {
                        order: detail_tasks.len(),
                        issue_id: *issue_id,
                        summary,
                        article_url,
                        platform_id,
                    });
                }
            }
            let detail_results = capture_cnki_index_attempts(client, attempt_sink, |client| {
                fetch_cnki_article_details(client, detail_tasks, config.worker_count.max(1))
            })?;
            article_details_count += detail_results.len() as i64;
            for result in detail_results {
                if let Some(record) = build_cnki_article_record(
                    Some(&result.detail),
                    &result.summary,
                    journal_id,
                    Some(result.issue_id),
                ) {
                    batch_records.push(record);
                }
            }
            let (batch_records, deleted_article_ids) =
                split_article_records_by_authors(batch_records);
            let batch_issue_ids = batch
                .iter()
                .map(|(issue_id, _)| *issue_id)
                .collect::<BTreeSet<_>>();
            let batch_issue_records = issue_records
                .iter()
                .filter(|record| batch_issue_ids.contains(&record.issue_id))
                .cloned()
                .collect::<Vec<_>>();
            let is_last_batch = batch_index + 1 == batch_count;
            with_immediate_index_transaction(connection, |transaction| {
                if let Some(lease_context) = lease_context {
                    lease_context
                        .assert_owner(transaction)
                        .map_err(|error| CnkiIndexError::Lease(error.to_string()))?;
                }
                upsert_journal(transaction, &journal_record)?;
                upsert_meta(transaction, &meta_record)?;
                if !batch_issue_records.is_empty() {
                    upsert_issues(transaction, &batch_issue_records)?;
                }
                apply_article_changes(
                    transaction,
                    &batch_records,
                    &deleted_article_ids,
                    &journal_title,
                    change_event_context,
                )?;
                if is_last_batch {
                    mark_year_done(transaction, journal_id, year, &config.timestamp)?;
                }
                Ok::<(), CnkiIndexError>(())
            })?;
            deleted_article_count += deleted_article_ids.len() as i64;
            written_article_count += batch_records.len() as i64;
        }
    }

    with_immediate_index_transaction(connection, |transaction| {
        if let Some(lease_context) = lease_context {
            lease_context
                .assert_owner(transaction)
                .map_err(|error| CnkiIndexError::Lease(error.to_string()))?;
        }
        upsert_journal(transaction, &journal_record)?;
        upsert_meta(transaction, &meta_record)?;
        mark_journal_done(transaction, journal_id, &config.timestamp)?;
        Ok::<(), CnkiIndexError>(())
    })?;

    Ok(ProcessOutcome {
        status: "succeeded".to_string(),
        written_article_count,
        issues_count: issues.len() as i64,
        article_summaries_count,
        article_details_count,
        deleted_article_count,
    })
}

fn fetch_cnki_article_details<T>(
    client: &mut CnkiClient<T>,
    tasks: Vec<CnkiArticleDetailTask>,
    worker_count: usize,
) -> Result<Vec<CnkiArticleDetailResult>, CnkiIndexError>
where
    T: CnkiTransport + Clone + Send + 'static,
{
    let mut results = Vec::with_capacity(tasks.len());
    for chunk in tasks.chunks(worker_count.max(1)) {
        let mut handles = Vec::new();
        for task in chunk.iter().cloned() {
            let mut worker_client = client.clone();
            let _ = worker_client.drain_attempts();
            handles.push(thread::spawn(move || {
                let result =
                    worker_client.article_detail(&task.article_url, task.platform_id.as_deref());
                let attempts = worker_client.drain_attempts();
                CnkiArticleDetailWorkerOutput {
                    task,
                    result,
                    attempts,
                }
            }));
        }

        let mut worker_outputs = Vec::new();
        for handle in handles {
            let output = handle.join().map_err(|_| {
                CnkiIndexError::Worker("CNKI article detail worker panicked".to_string())
            })?;
            let should_retry_serially = matches!(&output.result, Err(CnkiSourceError::Request(_)))
                && output
                    .attempts
                    .last()
                    .is_some_and(|attempt| !attempt.did_succeed && attempt.status_code.is_none());
            client.append_attempts(output.attempts);
            worker_outputs.push((output.task, output.result, should_retry_serially));
        }
        for (task, result, should_retry_serially) in worker_outputs {
            let detail = match result {
                Ok(detail) => detail,
                Err(_) if should_retry_serially => {
                    client.article_detail(&task.article_url, task.platform_id.as_deref())?
                }
                Err(error) => return Err(error.into()),
            };
            results.push(CnkiArticleDetailResult {
                order: task.order,
                issue_id: task.issue_id,
                summary: task.summary,
                detail,
            });
        }
    }
    results.sort_by_key(|result| result.order);
    Ok(results)
}

impl ProcessOutcome {
    fn resumed() -> Self {
        Self {
            status: "resumed".to_string(),
            written_article_count: 0,
            issues_count: 0,
            article_summaries_count: 0,
            article_details_count: 0,
            deleted_article_count: 0,
        }
    }
}

fn capture_cnki_attempts<T, R>(
    client: &mut CnkiClient<T>,
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
    operation: impl FnOnce(&mut CnkiClient<T>) -> Result<R, CnkiSourceError>,
) -> Result<R, CnkiSourceError>
where
    T: CnkiTransport,
{
    let result = operation(client);
    let attempts = client.drain_attempts();
    attempt_sink(&attempts);
    result
}

fn capture_cnki_index_attempts<T, R>(
    client: &mut CnkiClient<T>,
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
    operation: impl FnOnce(&mut CnkiClient<T>) -> Result<R, CnkiIndexError>,
) -> Result<R, CnkiIndexError>
where
    T: CnkiTransport,
{
    let result = operation(client);
    let attempts = client.drain_attempts();
    attempt_sink(&attempts);
    result
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use litradar_sources::{
        CnkiClient, CnkiFixtureData, CnkiSourceError, CnkiTransport, FixtureCnkiTransport,
        SourceAttempt,
    };
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use tempfile::tempdir;

    use crate::cnki::{
        fetch_cnki_article_details, process_cnki_row, run_cnki_fixture_index,
        select_recent_update_issue_ids, CnkiArticleDetailTask, CnkiIndexConfig, CnkiIndexError,
        CnkiProcessContext,
    };
    use crate::schema::{
        begin_index_run, init_index_db, mark_journal_done, IndexRunLeaseContext,
        IndexRunStartRequest,
    };
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
            worker_count: 1,
        };

        let outcome = process_cnki_row(
            &connection,
            &mut client,
            &row,
            CnkiProcessContext {
                csv_file: "journals.csv",
                journal_id,
                config: &config,
                change_event_context: None,
                lease_context: None,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("completed journal should resume");

        assert_eq!(outcome.status, "resumed");
        assert_eq!(outcome.written_article_count, 0);
        assert_eq!(client.attempts().len(), 0);
    }

    #[test]
    fn article_details_use_worker_concurrency() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let state = Arc::new(InstrumentedCnkiState::default());
        let mut client = CnkiClient::new(InstrumentedCnkiTransport::new(state.clone()));
        let row = CsvRow::from([
            ("source".to_string(), "cnki".to_string()),
            ("title".to_string(), "Concurrent CNKI".to_string()),
            ("issn".to_string(), "1234-5678".to_string()),
            ("id".to_string(), "Concurrent CNKI".to_string()),
        ]);
        let config = CnkiIndexConfig {
            csv_path: PathBuf::new(),
            fixture_path: PathBuf::new(),
            output_db_path: PathBuf::new(),
            manifest_path: None,
            run_id: "run-cnki-concurrent".to_string(),
            timestamp: "2026-07-05T00:00:00Z".to_string(),
            resume: false,
            update: false,
            issue_batch_size: 1,
            worker_count: 2,
        };

        let mut captured_attempts = Vec::new();
        let outcome = process_cnki_row(
            &connection,
            &mut client,
            &row,
            CnkiProcessContext {
                csv_file: "journals.csv",
                journal_id: 99,
                config: &config,
                change_event_context: None,
                lease_context: None,
                attempt_sink: &mut |attempts| captured_attempts.extend_from_slice(attempts),
            },
        )
        .expect("concurrent detail processing should succeed");

        assert_eq!(outcome.article_details_count, 2);
        assert_eq!(outcome.written_article_count, 2);
        assert!(state.max_active.load(Ordering::SeqCst) >= 2);
        assert_eq!(
            captured_attempts
                .iter()
                .filter(|attempt| attempt.endpoint == "article_detail")
                .count(),
            2
        );
    }

    #[test]
    fn article_details_retry_transport_failures_serially() {
        let state = Arc::new(InstrumentedCnkiState {
            fail_first_detail_calls: 1,
            ..InstrumentedCnkiState::default()
        });
        let mut client = CnkiClient::new(InstrumentedCnkiTransport::new(state.clone()));

        let results = fetch_cnki_article_details(&mut client, article_detail_tasks(), 2)
            .expect("transport failure should recover serially");

        assert_eq!(
            results
                .iter()
                .map(|result| result.order)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(state.detail_calls.load(Ordering::SeqCst), 3);
        assert!(state.max_active.load(Ordering::SeqCst) >= 2);
        assert_eq!(
            client
                .attempts()
                .iter()
                .filter(|attempt| !attempt.did_succeed)
                .count(),
            1
        );
        assert_eq!(
            client
                .attempts()
                .iter()
                .filter(|attempt| attempt.did_succeed)
                .count(),
            2
        );
    }

    #[test]
    fn article_details_do_not_retry_failures_with_http_status() {
        let state = Arc::new(InstrumentedCnkiState {
            fail_first_detail_calls: 1,
            failed_detail_status_code: Some(503),
            ..InstrumentedCnkiState::default()
        });
        let mut client = CnkiClient::new(InstrumentedCnkiTransport::new(state.clone()));

        let error = fetch_cnki_article_details(&mut client, article_detail_tasks(), 2)
            .expect_err("HTTP failure should remain fatal");

        assert!(matches!(error, CnkiIndexError::Source(_)));
        assert_eq!(state.detail_calls.load(Ordering::SeqCst), 2);
        assert_eq!(client.attempts().len(), 2);
    }

    #[test]
    fn article_details_fail_when_serial_transport_retry_fails() {
        let state = Arc::new(InstrumentedCnkiState {
            fail_first_detail_calls: usize::MAX,
            ..InstrumentedCnkiState::default()
        });
        let mut client = CnkiClient::new(InstrumentedCnkiTransport::new(state.clone()));

        let error = fetch_cnki_article_details(&mut client, article_detail_tasks(), 2)
            .expect_err("persistent transport failure should remain fatal");

        assert!(matches!(error, CnkiIndexError::Source(_)));
        assert_eq!(state.detail_calls.load(Ordering::SeqCst), 3);
        assert_eq!(client.attempts().len(), 3);
        assert!(client
            .attempts()
            .iter()
            .all(|attempt| !attempt.did_succeed && attempt.status_code.is_none()));
    }

    #[test]
    fn stale_live_worker_cannot_commit_cnki_batches() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        begin_index_run(
            &connection,
            &IndexRunStartRequest {
                run_id: "run-owner",
                csv_file: "journals.csv",
                started_at: "4000000000",
                total_journals: 1,
                now_epoch_seconds: 4_000_000_000,
                should_adopt_events: false,
            },
        )
        .expect("owner run should start");
        let stale_lease = IndexRunLeaseContext::new("run-stale");
        let state = Arc::new(InstrumentedCnkiState::default());
        let mut client = CnkiClient::new(InstrumentedCnkiTransport::new(state));
        let row = CsvRow::from([
            ("source".to_string(), "cnki".to_string()),
            ("title".to_string(), "Fenced CNKI".to_string()),
            ("issn".to_string(), "1234-5678".to_string()),
            ("id".to_string(), "Fenced CNKI".to_string()),
        ]);
        let config = CnkiIndexConfig {
            csv_path: PathBuf::new(),
            fixture_path: PathBuf::new(),
            output_db_path: PathBuf::new(),
            manifest_path: None,
            run_id: "run-stale".to_string(),
            timestamp: "4000000000".to_string(),
            resume: false,
            update: false,
            issue_batch_size: 1,
            worker_count: 1,
        };

        let error = process_cnki_row(
            &connection,
            &mut client,
            &row,
            CnkiProcessContext {
                csv_file: "journals.csv",
                journal_id: 100,
                config: &config,
                change_event_context: None,
                lease_context: Some(&stale_lease),
                attempt_sink: &mut |_| {},
            },
        )
        .expect_err("stale worker should fail before its first commit");

        let article_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))
            .expect("article count should load");
        assert!(matches!(error, CnkiIndexError::Lease(_)));
        assert_eq!(article_count, 0);
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
            worker_count: 10,
        })
        .expect("fixture index should succeed");

        assert_eq!(outcome.status, "succeeded");
        assert!(manifest_path.exists());
        assert_eq!(outcome.written_article_count, 1);
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

    #[derive(Debug, Default)]
    struct InstrumentedCnkiState {
        active: AtomicUsize,
        max_active: AtomicUsize,
        detail_calls: AtomicUsize,
        fail_first_detail_calls: usize,
        failed_detail_status_code: Option<u16>,
    }

    #[derive(Debug, Clone)]
    struct InstrumentedCnkiTransport {
        state: Arc<InstrumentedCnkiState>,
        attempts: Vec<SourceAttempt>,
    }

    impl InstrumentedCnkiTransport {
        fn new(state: Arc<InstrumentedCnkiState>) -> Self {
            Self {
                state,
                attempts: Vec::new(),
            }
        }

        fn record_attempt(&mut self, endpoint: &str, key: Option<&str>) {
            self.record_attempt_outcome(endpoint, key, Some(200), true, None);
        }

        fn record_attempt_outcome(
            &mut self,
            endpoint: &str,
            key: Option<&str>,
            status_code: Option<u16>,
            did_succeed: bool,
            error: Option<String>,
        ) {
            self.attempts.push(SourceAttempt {
                service: "cnki".to_string(),
                endpoint: endpoint.to_string(),
                method: if endpoint == "article_detail" {
                    "GET".to_string()
                } else {
                    "POST".to_string()
                },
                url: format!("https://example.test/{}", key.unwrap_or(endpoint)),
                status_code,
                did_succeed,
                did_retry: false,
                error,
            });
        }
    }

    impl CnkiTransport for InstrumentedCnkiTransport {
        fn text(&mut self, endpoint: &str, _key: Option<&str>) -> Result<String, CnkiSourceError> {
            Err(CnkiSourceError::Request(format!(
                "instrumented transport does not use text endpoint {endpoint}"
            )))
        }

        fn resolve_journal(&mut self, _row: &CsvRow) -> Result<Option<Value>, CnkiSourceError> {
            self.record_attempt("journal_detail", None);
            Ok(Some(json!({
                "pykm": "TEST",
                "pcode": "CJFD",
                "title": "Concurrent CNKI",
                "issn": "1234-5678"
            })))
        }

        fn year_issues(&mut self, _journal: &Value) -> Result<Vec<Value>, CnkiSourceError> {
            self.record_attempt("year_issues", None);
            Ok(vec![json!({
                "year": 2026,
                "number": "1",
                "title": "2026 No.1",
                "year_issue": "202601"
            })])
        }

        fn issue_articles(
            &mut self,
            _journal: &Value,
            _issue: &Value,
        ) -> Result<Vec<Value>, CnkiSourceError> {
            self.record_attempt("issue_articles", Some("202601"));
            Ok(vec![
                json!({
                    "title": "Article A",
                    "article_url": "https://example.test/article-a",
                    "platform_id": "ARTICLE-A",
                    "authors": "Summary Author"
                }),
                json!({
                    "title": "Article B",
                    "article_url": "https://example.test/article-b",
                    "platform_id": "ARTICLE-B",
                    "authors": "Summary Author"
                }),
            ])
        }

        fn article_detail(
            &mut self,
            article_url: &str,
            platform_id: Option<&str>,
        ) -> Result<Value, CnkiSourceError> {
            let detail_call = self.state.detail_calls.fetch_add(1, Ordering::SeqCst);
            let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.state.max_active.fetch_max(active, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(50));
            self.state.active.fetch_sub(1, Ordering::SeqCst);
            if detail_call < self.state.fail_first_detail_calls {
                let message = "simulated article detail failure".to_string();
                self.record_attempt_outcome(
                    "article_detail",
                    platform_id,
                    self.state.failed_detail_status_code,
                    false,
                    Some(message.clone()),
                );
                return Err(CnkiSourceError::Request(message));
            }
            self.record_attempt("article_detail", platform_id);
            Ok(json!({
                "platform_id": platform_id.unwrap_or(article_url),
                "title": platform_id.unwrap_or(article_url),
                "authors": "Detail Author",
                "permalink": article_url
            }))
        }

        fn attempts(&self) -> &[SourceAttempt] {
            &self.attempts
        }

        fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
            std::mem::take(&mut self.attempts)
        }

        fn append_attempts(&mut self, attempts: Vec<SourceAttempt>) {
            self.attempts.extend(attempts);
        }
    }

    fn article_detail_tasks() -> Vec<CnkiArticleDetailTask> {
        ["ARTICLE-A", "ARTICLE-B"]
            .into_iter()
            .enumerate()
            .map(|(order, platform_id)| CnkiArticleDetailTask {
                order,
                issue_id: 1,
                summary: json!({"platform_id": platform_id}),
                article_url: format!("https://example.test/{platform_id}"),
                platform_id: Some(platform_id.to_string()),
            })
            .collect()
    }
}
