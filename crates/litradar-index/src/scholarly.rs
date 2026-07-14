//! Scholarly index orchestration backed by fixture source clients.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use litradar_sources::{
    normalize_doi, FixtureScholarlyTransport, ScholarlyClient, ScholarlyFixtureData,
    ScholarlyTransport, ScholarlyWorksPage, SourceAttempt, SourceError,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;

use crate::changes::{write_change_manifest_from_events, ChangeWriteError};
use crate::schema::{
    apply_article_changes, get_journal_synchronization_date, is_journal_complete,
    mark_article_listing_ready, mark_journal_done, mark_year_done, open_index_db,
    optimize_index_db, persist_index_run_stats, upsert_issues, upsert_journal, upsert_meta,
    with_immediate_index_transaction, ChangeEventContext, IndexRunLeaseContext,
};
use crate::stats::{IndexRunStats, PathCountIncrements};
use crate::transforms::{
    apply_crossref_resolved_meta, apply_openalex_resolved_meta, build_journal_id,
    build_meta_record, build_openalex_crossref_work, build_openalex_journal_row,
    build_scholarly_article_record, build_scholarly_issue_record, build_scholarly_journal_record,
    candidate_issns_from_row, doi_values_from_works, embedded_openalex_work, issue_year,
    journal_title_from_row, split_article_records_by_authors, ArticleRecord, CsvRow, IssueRecord,
    JournalRecord, MetaRecord,
};

const SCHOLARLY_WRITE_BATCH_SIZE: usize = 100;

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
    /// Written article count.
    pub written_article_count: i64,
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
    /// Live run lease ownership was lost.
    Lease(String),
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
            Self::Lease(message) => formatter.write_str(message),
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
            Self::Lease(_) => None,
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

impl From<ChangeWriteError> for ScholarlyIndexError {
    /// Convert streamed manifest errors into index errors.
    fn from(error: ChangeWriteError) -> Self {
        match error {
            ChangeWriteError::Sqlite(error) => Self::Sqlite(error),
            ChangeWriteError::Io(error) => Self::Io(error),
            ChangeWriteError::Json(error) => Self::Json(error),
        }
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
    let mut written_article_count = 0;
    let mut source_attempt_count = 0;
    let change_event_context = config
        .manifest_path
        .as_ref()
        .map(|_| ChangeEventContext::new(&config.run_id, "fixture-0", &config.timestamp, false));

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
        let result = {
            let mut attempt_sink = |attempts: &[SourceAttempt]| {
                source_attempt_count += attempts.len();
                stats.record_source_attempts(attempts, Some(journal_id), &journal_title);
            };
            process_scholarly_row(
                &connection,
                &mut client,
                &row,
                ScholarlyProcessContext {
                    csv_file: &csv_file,
                    journal_id,
                    timestamp: &config.timestamp,
                    change_event_context: change_event_context.as_ref(),
                    lease_context: None,
                    should_resume: false,
                    attempt_sink: &mut attempt_sink,
                },
            )
        };
        match result {
            Ok(ProcessOutcome {
                status,
                written_article_count: journal_written_article_count,
                works_count,
                issues_count,
                deleted_article_count,
            }) => {
                stats.record_path_counts(
                    &path_key,
                    PathCountIncrements {
                        works_count,
                        issues_count,
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

    Ok(ScholarlyIndexOutcome {
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
    pub(crate) works_count: i64,
    pub(crate) issues_count: i64,
    pub(crate) deleted_article_count: i64,
}

/// Per-journal Scholarly processing context.
pub(crate) struct ScholarlyProcessContext<'a> {
    pub(crate) csv_file: &'a str,
    pub(crate) journal_id: i64,
    pub(crate) timestamp: &'a str,
    pub(crate) change_event_context: Option<&'a ChangeEventContext>,
    pub(crate) lease_context: Option<&'a IndexRunLeaseContext>,
    pub(crate) should_resume: bool,
    pub(crate) attempt_sink: &'a mut dyn FnMut(&[SourceAttempt]),
}

/// Process one Scholarly CSV row into an index database.
pub(crate) fn process_scholarly_row<T>(
    connection: &Connection,
    client: &mut ScholarlyClient<T>,
    row: &CsvRow,
    context: ScholarlyProcessContext<'_>,
) -> Result<ProcessOutcome, ScholarlyIndexError>
where
    T: ScholarlyTransport,
{
    let ScholarlyProcessContext {
        csv_file,
        journal_id,
        timestamp,
        change_event_context,
        lease_context,
        should_resume,
        attempt_sink,
    } = context;
    if should_resume && is_journal_complete(connection, journal_id)? {
        return Ok(ProcessOutcome::resumed());
    }
    let is_live_synchronization = change_event_context.is_some() && lease_context.is_some();
    let synchronization_date = if is_live_synchronization {
        get_journal_synchronization_date(connection, journal_id, timestamp)?
    } else {
        None
    };
    let issn_candidates = candidate_issns_from_row(row);
    if issn_candidates.is_empty() {
        return Err(ScholarlyIndexError::InvalidJournal(format!(
            "Scholarly journal missing ISSN: {}",
            journal_title_from_row(row)
        )));
    }

    let mut issn = issn_candidates[0].clone();
    let mut openalex_source = None;
    let mut last_404 = None;
    let mut crossref_page = None;
    for candidate in &issn_candidates {
        match capture_scholarly_attempts(client, attempt_sink, |client| {
            client.fetch_journal_works_page(candidate, synchronization_date.as_deref(), None)
        }) {
            Ok(candidate_page) => {
                issn = candidate.clone();
                crossref_page = Some(candidate_page);
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

    if crossref_page.is_none() && last_404.is_some() {
        openalex_source = capture_scholarly_attempts(client, attempt_sink, |client| {
            client.fetch_openalex_source_by_issns(&issn_candidates)
        })?;
        if openalex_source.is_none() {
            openalex_source = capture_scholarly_attempts(client, attempt_sink, |client| {
                client.fetch_openalex_source_by_title(&journal_title_from_row(row))
            })?;
        }
    }
    let mut journal_row = row.clone();
    if let Some(source) = &openalex_source {
        journal_row = build_openalex_journal_row(row, source);
        issn = journal_row.get("issn").cloned().unwrap_or(issn);
    } else {
        journal_row.insert("issn".to_string(), issn.clone());
    }
    let mut journal_record = build_scholarly_journal_record(journal_id, &journal_row, &[]);
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
    let mut totals = ScholarlyWriteTotals::default();
    let mut years = BTreeSet::new();

    if let Some(first_page) = crossref_page {
        process_crossref_pages(
            connection,
            client,
            first_page,
            &issn,
            synchronization_date.as_deref(),
            &mut journal_record,
            &meta_record,
            &journal_title,
            change_event_context,
            lease_context,
            attempt_sink,
            &mut totals,
            &mut years,
        )?;
    } else {
        let source = openalex_source.as_ref().ok_or_else(|| {
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
        let source_issns = candidate_issns_from_row(&journal_row);
        process_openalex_pages(
            connection,
            client,
            &source_id,
            &source_issns,
            synchronization_date.as_deref(),
            &mut journal_record,
            &meta_record,
            &journal_title,
            change_event_context,
            lease_context,
            attempt_sink,
            &mut totals,
            &mut years,
        )?;
        if totals.works_count == 0 && synchronization_date.is_none() {
            return Err(ScholarlyIndexError::InvalidJournal(format!(
                "OpenAlex fallback returned no usable works: {}",
                journal_title_from_row(row)
            )));
        }
    }

    if synchronization_date.is_none() {
        journal_record.has_articles = Some(i64::from(totals.works_count > 0));
    }
    with_immediate_index_transaction(connection, |transaction| {
        if let Some(lease_context) = lease_context {
            lease_context
                .assert_owner(transaction)
                .map_err(|error| ScholarlyIndexError::Lease(error.to_string()))?;
        }
        if synchronization_date.is_none() {
            upsert_journal(transaction, &journal_record)?;
            upsert_meta(transaction, &meta_record)?;
        }
        for year in &years {
            mark_year_done(transaction, journal_id, *year, timestamp)?;
        }
        mark_journal_done(transaction, journal_id, timestamp)?;
        Ok::<(), ScholarlyIndexError>(())
    })?;
    Ok(ProcessOutcome {
        status: "succeeded".to_string(),
        works_count: totals.works_count,
        issues_count: totals.issue_ids.len() as i64,
        deleted_article_count: totals.deleted_article_count,
        written_article_count: totals.written_article_count,
    })
}

#[derive(Debug, Default)]
struct ScholarlyWriteTotals {
    works_count: i64,
    written_article_count: i64,
    deleted_article_count: i64,
    issue_ids: BTreeSet<i64>,
}

#[allow(clippy::too_many_arguments)]
fn process_crossref_pages<T>(
    connection: &Connection,
    client: &mut ScholarlyClient<T>,
    mut page: ScholarlyWorksPage,
    issn: &str,
    synchronization_date: Option<&str>,
    journal_record: &mut JournalRecord,
    meta_record: &MetaRecord,
    journal_title: &str,
    change_event_context: Option<&ChangeEventContext>,
    lease_context: Option<&IndexRunLeaseContext>,
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
    totals: &mut ScholarlyWriteTotals,
    years: &mut BTreeSet<i64>,
) -> Result<(), ScholarlyIndexError>
where
    T: ScholarlyTransport,
{
    loop {
        update_scholarly_journal_from_page(journal_record, &page.items);
        process_scholarly_work_page(
            connection,
            client,
            &page.items,
            journal_record,
            meta_record,
            journal_title,
            change_event_context,
            lease_context,
            synchronization_date.is_none(),
            attempt_sink,
            totals,
            years,
        )?;
        let Some(cursor) = page.next_cursor.take() else {
            break;
        };
        let next_page = capture_scholarly_attempts(client, attempt_sink, |client| {
            client.fetch_journal_works_page(issn, synchronization_date, Some(&cursor))
        })?;
        page = next_page;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_openalex_pages<T>(
    connection: &Connection,
    client: &mut ScholarlyClient<T>,
    source_id: &str,
    source_issns: &[String],
    synchronization_date: Option<&str>,
    journal_record: &mut JournalRecord,
    meta_record: &MetaRecord,
    journal_title: &str,
    change_event_context: Option<&ChangeEventContext>,
    lease_context: Option<&IndexRunLeaseContext>,
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
    totals: &mut ScholarlyWriteTotals,
    years: &mut BTreeSet<i64>,
) -> Result<(), ScholarlyIndexError>
where
    T: ScholarlyTransport,
{
    let mut cursor = None;
    loop {
        let page = capture_scholarly_attempts(client, attempt_sink, |client| {
            client.fetch_openalex_works_by_source_page(
                source_id,
                synchronization_date,
                cursor.as_deref(),
            )
        })?;
        for raw_batch in page.items.chunks(SCHOLARLY_WRITE_BATCH_SIZE) {
            let works = raw_batch
                .iter()
                .filter_map(|work| build_openalex_crossref_work(work, source_issns))
                .collect::<Vec<_>>();
            update_scholarly_journal_from_page(journal_record, &works);
            process_scholarly_work_page(
                connection,
                client,
                &works,
                journal_record,
                meta_record,
                journal_title,
                change_event_context,
                lease_context,
                synchronization_date.is_none(),
                attempt_sink,
                totals,
                years,
            )?;
        }
        let Some(next_cursor) = page.next_cursor else {
            break;
        };
        if cursor.as_deref() == Some(next_cursor.as_str()) {
            return Err(ScholarlyIndexError::InvalidJournal(
                "OpenAlex returned a repeated cursor".to_string(),
            ));
        }
        cursor = Some(next_cursor);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_scholarly_work_page<T>(
    connection: &Connection,
    client: &mut ScholarlyClient<T>,
    works: &[Value],
    journal_record: &JournalRecord,
    meta_record: &MetaRecord,
    journal_title: &str,
    change_event_context: Option<&ChangeEventContext>,
    lease_context: Option<&IndexRunLeaseContext>,
    should_write_journal_metadata: bool,
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
    totals: &mut ScholarlyWriteTotals,
    years: &mut BTreeSet<i64>,
) -> Result<(), ScholarlyIndexError>
where
    T: ScholarlyTransport,
{
    for batch in works.chunks(SCHOLARLY_WRITE_BATCH_SIZE) {
        let dois = doi_values_from_works(batch);
        let openalex_by_doi = if dois.is_empty()
            || batch
                .iter()
                .any(|work| embedded_openalex_work(work).is_some())
        {
            BTreeMap::new()
        } else {
            capture_scholarly_attempts(client, attempt_sink, |client| {
                client.fetch_openalex_by_dois(&dois, SCHOLARLY_WRITE_BATCH_SIZE)
            })?
        };
        let semantic_scholar_by_doi = if dois.is_empty() {
            BTreeMap::new()
        } else {
            capture_scholarly_attempts(client, attempt_sink, |client| {
                client.fetch_semantic_scholar_by_dois(&dois, SCHOLARLY_WRITE_BATCH_SIZE)
            })?
        };
        let mut issue_records_by_id = BTreeMap::<i64, IssueRecord>::new();
        let mut article_records = Vec::new();
        for work in batch {
            let issue_record = build_scholarly_issue_record(journal_record.journal_id, work);
            let issue_id = issue_record.as_ref().map(|record| record.issue_id);
            if let Some(issue_record) = issue_record {
                if let Some(year) = issue_year(&issue_record) {
                    years.insert(year);
                }
                totals.issue_ids.insert(issue_record.issue_id);
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
                journal_record.journal_id,
                issue_id,
            ) {
                backfill_semantic_scholar_abstract(&mut article_record, semantic_scholar_work);
                article_records.push(article_record);
            }
        }
        let issue_records = issue_records_by_id.into_values().collect::<Vec<_>>();
        let (article_records, deleted_article_ids) =
            split_article_records_by_authors(article_records);
        with_immediate_index_transaction(connection, |transaction| {
            if let Some(lease_context) = lease_context {
                lease_context
                    .assert_owner(transaction)
                    .map_err(|error| ScholarlyIndexError::Lease(error.to_string()))?;
            }
            if should_write_journal_metadata {
                upsert_journal(transaction, journal_record)?;
                upsert_meta(transaction, meta_record)?;
            }
            if !issue_records.is_empty() {
                upsert_issues(transaction, &issue_records)?;
            }
            apply_article_changes(
                transaction,
                &article_records,
                &deleted_article_ids,
                journal_title,
                change_event_context,
            )?;
            Ok::<(), ScholarlyIndexError>(())
        })?;
        totals.works_count += batch.len() as i64;
        totals.written_article_count += article_records.len() as i64;
        totals.deleted_article_count += deleted_article_ids.len() as i64;
    }
    Ok(())
}

fn update_scholarly_journal_from_page(journal_record: &mut JournalRecord, works: &[Value]) {
    if journal_record.eissn.is_none() {
        let row = journal_record
            .issn
            .as_ref()
            .map(|issn| CsvRow::from([("issn".to_string(), issn.clone())]))
            .unwrap_or_default();
        journal_record.eissn =
            build_scholarly_journal_record(journal_record.journal_id, &row, works).eissn;
    }
}

fn capture_scholarly_attempts<T, R>(
    client: &mut ScholarlyClient<T>,
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
    operation: impl FnOnce(&mut ScholarlyClient<T>) -> Result<R, SourceError>,
) -> Result<R, SourceError>
where
    T: ScholarlyTransport,
{
    let result = operation(client);
    let attempts = client.drain_attempts();
    attempt_sink(&attempts);
    result
}

impl ProcessOutcome {
    fn resumed() -> Self {
        Self {
            status: "resumed".to_string(),
            written_article_count: 0,
            works_count: 0,
            issues_count: 0,
            deleted_article_count: 0,
        }
    }
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

    use litradar_sources::{
        FixtureScholarlyTransport, ScholarlyClient, ScholarlyFixtureData, SourceAttempt,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::schema::{
        begin_index_run, init_index_db, mark_journal_done, ChangeEventContext,
        IndexRunLeaseContext, IndexRunStartRequest,
    };
    use crate::transforms::CsvRow;

    use super::{
        process_scholarly_row, run_scholarly_fixture_index, ScholarlyIndexConfig,
        ScholarlyIndexError, ScholarlyProcessContext,
    };

    #[test]
    fn resume_skips_completed_journal_before_source_calls() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        mark_journal_done(&connection, 42, "2026-07-13T00:00:00Z")
            .expect("journal should be complete");
        let row = scholarly_row();
        let mut client = ScholarlyClient::new(
            FixtureScholarlyTransport::new(ScholarlyFixtureData::default()),
            true,
        );

        let outcome = process_scholarly_row(
            &connection,
            &mut client,
            &row,
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 42,
                timestamp: "2026-07-13T00:00:00Z",
                change_event_context: None,
                lease_context: None,
                should_resume: true,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("completed journal should resume");

        assert_eq!(outcome.status, "resumed");
        assert_eq!(outcome.written_article_count, 0);
        assert!(client.attempts().is_empty());
    }

    #[test]
    fn stateful_crossref_cursor_pages_are_enriched_and_committed_in_order() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let fixture = ScholarlyFixtureData {
            crossref_work_pages: vec![
                vec![crossref_work("10.1/first", "First Article", "1")],
                vec![crossref_work("10.1/second", "Second Article", "2")],
                vec![crossref_work("10.1/third", "Third Article", "3")],
            ],
            ..ScholarlyFixtureData::default()
        };
        let mut client = ScholarlyClient::new(FixtureScholarlyTransport::new(fixture), true);
        let mut attempts = Vec::<SourceAttempt>::new();

        let outcome = process_scholarly_row(
            &connection,
            &mut client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 43,
                timestamp: "2026-07-13T00:00:00Z",
                change_event_context: None,
                lease_context: None,
                should_resume: false,
                attempt_sink: &mut |batch| attempts.extend_from_slice(batch),
            },
        )
        .expect("multi-page journal should succeed");

        assert_eq!(outcome.works_count, 3);
        assert_eq!(outcome.written_article_count, 3);
        assert_eq!(
            attempts
                .iter()
                .map(|attempt| attempt.endpoint.as_str())
                .collect::<Vec<_>>(),
            vec![
                "journal_works",
                "works",
                "paper_batch",
                "journal_works",
                "works",
                "paper_batch",
                "journal_works",
                "works",
                "paper_batch"
            ]
        );
        assert!(client.attempts().is_empty());
        let transport = client.into_transport();
        assert_eq!(
            transport.journal_work_requests(),
            &[
                ("1234-5678".to_string(), None),
                ("1234-5678".to_string(), None),
                ("1234-5678".to_string(), None),
            ]
        );
        let article_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))
            .expect("article count should query");
        assert_eq!(article_count, 3);
    }

    #[test]
    fn stale_live_worker_cannot_commit_scholarly_batches() {
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
        let fixture = ScholarlyFixtureData {
            crossref_work_pages: vec![vec![crossref_work("10.1/fenced", "Fenced Article", "1")]],
            ..ScholarlyFixtureData::default()
        };
        let mut client = ScholarlyClient::new(FixtureScholarlyTransport::new(fixture), true);

        let error = process_scholarly_row(
            &connection,
            &mut client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 44,
                timestamp: "4000000000",
                change_event_context: None,
                lease_context: Some(&stale_lease),
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect_err("stale worker should fail before its first commit");

        let article_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))
            .expect("article count should load");
        assert!(matches!(error, ScholarlyIndexError::Lease(_)));
        assert_eq!(article_count, 0);
    }

    #[test]
    fn incremental_crossref_uses_one_checkpoint_and_preserves_metadata() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        seed_scholarly_journal(&connection, 45, "Existing Crossref Title");
        mark_journal_done(&connection, 45, "2026-07-13T00:00:00Z")
            .expect("prior checkpoint should be written");
        let (lease_context, change_event_context) =
            live_synchronization_context(&connection, "run-crossref-sync", "2026-07-14T00:00:00Z");
        let fixture = ScholarlyFixtureData {
            crossref_work_pages: vec![
                vec![crossref_work("10.1/new-first", "New First", "10")],
                vec![crossref_work("10.1/new-second", "New Second", "20")],
            ],
            ..ScholarlyFixtureData::default()
        };
        let mut client = ScholarlyClient::new(FixtureScholarlyTransport::new(fixture), true);

        let outcome = process_scholarly_row(
            &connection,
            &mut client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 45,
                timestamp: "2026-07-14T00:00:00Z",
                change_event_context: Some(&change_event_context),
                lease_context: Some(&lease_context),
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("incremental Crossref pages should succeed");

        let transport = client.into_transport();
        let (title, source_csv, checkpoint): (String, String, String) = connection
            .query_row(
                "
                SELECT j.title, m.source_csv, s.updated_at
                FROM journals j
                JOIN journal_meta m ON m.journal_id = j.journal_id
                JOIN journal_state s ON s.journal_id = j.journal_id
                WHERE j.journal_id = 45
                ",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("preserved metadata should load");
        assert_eq!(outcome.works_count, 2);
        assert_eq!(title, "Existing Crossref Title");
        assert_eq!(source_csv, "existing.csv");
        assert_eq!(checkpoint, "2026-07-14T00:00:00Z");
        assert_eq!(
            transport.journal_work_requests(),
            &[
                ("1234-5678".to_string(), Some("2026-06-13".to_string())),
                ("1234-5678".to_string(), Some("2026-06-13".to_string())),
            ]
        );
    }

    #[test]
    fn empty_incremental_openalex_pages_preserve_existing_journal() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        seed_scholarly_journal(&connection, 46, "Existing OpenAlex Title");
        mark_journal_done(&connection, 46, "1783900800")
            .expect("prior checkpoint should be written");
        let (lease_context, change_event_context) =
            live_synchronization_context(&connection, "run-openalex-sync", "1783987200");
        let fixture = ScholarlyFixtureData {
            crossref_status: Some(404),
            openalex_source_by_issns: Some(json!({
                "id": "https://openalex.org/S46",
                "display_name": "Existing OpenAlex Title",
                "issn_l": "1234-5678",
                "issn": ["1234-5678"]
            })),
            openalex_source_work_pages: vec![Vec::new(), Vec::new()],
            ..ScholarlyFixtureData::default()
        };
        let mut client = ScholarlyClient::new(FixtureScholarlyTransport::new(fixture), true);

        let outcome = process_scholarly_row(
            &connection,
            &mut client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 46,
                timestamp: "1783987200",
                change_event_context: Some(&change_event_context),
                lease_context: Some(&lease_context),
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("empty incremental OpenAlex pages should succeed");

        let transport = client.into_transport();
        let (title, checkpoint): (String, String) = connection
            .query_row(
                "
                SELECT j.title, s.updated_at
                FROM journals j
                JOIN journal_state s ON s.journal_id = j.journal_id
                WHERE j.journal_id = 46
                ",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("preserved journal should load");
        assert_eq!(outcome.works_count, 0);
        assert_eq!(title, "Existing OpenAlex Title");
        assert_eq!(checkpoint, "1783987200");
        assert_eq!(
            transport.source_work_requests(),
            &[
                (
                    "https://openalex.org/S46".to_string(),
                    Some("2026-06-13".to_string())
                ),
                (
                    "https://openalex.org/S46".to_string(),
                    Some("2026-06-13".to_string())
                ),
            ]
        );
    }

    #[test]
    fn empty_incremental_crossref_page_preserves_existing_journal() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        seed_scholarly_journal(&connection, 49, "Existing Empty Title");
        mark_journal_done(&connection, 49, "2026-07-13T00:00:00Z")
            .expect("prior checkpoint should be written");
        let (lease_context, change_event_context) =
            live_synchronization_context(&connection, "run-empty-crossref", "2026-07-14T00:00:00Z");
        let mut client = ScholarlyClient::new(
            FixtureScholarlyTransport::new(ScholarlyFixtureData::default()),
            true,
        );

        let outcome = process_scholarly_row(
            &connection,
            &mut client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 49,
                timestamp: "2026-07-14T00:00:00Z",
                change_event_context: Some(&change_event_context),
                lease_context: Some(&lease_context),
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("empty incremental Crossref page should succeed");

        let transport = client.into_transport();
        let title: String = connection
            .query_row(
                "SELECT title FROM journals WHERE journal_id = 49",
                [],
                |row| row.get(0),
            )
            .expect("preserved title should load");
        assert_eq!(outcome.works_count, 0);
        assert_eq!(title, "Existing Empty Title");
        assert_eq!(
            transport.journal_work_requests(),
            &[("1234-5678".to_string(), Some("2026-06-13".to_string()))]
        );
    }

    #[test]
    fn fixture_manifest_context_does_not_enable_incremental_filter() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        seed_scholarly_journal(&connection, 47, "Existing Fixture Title");
        mark_journal_done(&connection, 47, "2026-07-13T00:00:00Z")
            .expect("prior checkpoint should be written");
        let change_event_context =
            ChangeEventContext::new("fixture-run", "fixture-0", "2026-07-14T00:00:00Z", false);
        let fixture = ScholarlyFixtureData {
            crossref_work_pages: vec![vec![crossref_work("10.1/fixture", "Fixture Article", "30")]],
            ..ScholarlyFixtureData::default()
        };
        let mut client = ScholarlyClient::new(FixtureScholarlyTransport::new(fixture), true);

        process_scholarly_row(
            &connection,
            &mut client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 47,
                timestamp: "2026-07-14T00:00:00Z",
                change_event_context: Some(&change_event_context),
                lease_context: None,
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("fixture manifest run should remain a full scan");

        let transport = client.into_transport();
        assert_eq!(
            transport.journal_work_requests(),
            &[("1234-5678".to_string(), None)]
        );
    }

    #[test]
    fn failed_incremental_journal_reuses_prior_checkpoint_on_retry() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        seed_scholarly_journal(&connection, 48, "Retry Journal");
        mark_journal_done(&connection, 48, "2026-07-13T00:00:00Z")
            .expect("prior checkpoint should be written");
        let (lease_context, change_event_context) =
            live_synchronization_context(&connection, "run-retry-sync", "2026-07-14T00:00:00Z");
        let failed_fixture = ScholarlyFixtureData {
            crossref_work_pages: vec![vec![crossref_work("10.1/retry", "Retry Article", "40")]],
            semantic_scholar_status: Some(503),
            ..ScholarlyFixtureData::default()
        };
        let mut failed_client =
            ScholarlyClient::new(FixtureScholarlyTransport::new(failed_fixture), true);

        process_scholarly_row(
            &connection,
            &mut failed_client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 48,
                timestamp: "2026-07-14T00:00:00Z",
                change_event_context: Some(&change_event_context),
                lease_context: Some(&lease_context),
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect_err("failed incremental enrichment should not advance the checkpoint");
        let failed_checkpoint: String = connection
            .query_row(
                "SELECT updated_at FROM journal_state WHERE journal_id = 48",
                [],
                |row| row.get(0),
            )
            .expect("failed checkpoint should load");
        assert_eq!(failed_checkpoint, "2026-07-13T00:00:00Z");

        let retry_fixture = ScholarlyFixtureData {
            crossref_work_pages: vec![vec![crossref_work("10.1/retry", "Retry Article", "40")]],
            ..ScholarlyFixtureData::default()
        };
        let mut retry_client =
            ScholarlyClient::new(FixtureScholarlyTransport::new(retry_fixture), true);
        process_scholarly_row(
            &connection,
            &mut retry_client,
            &scholarly_row(),
            ScholarlyProcessContext {
                csv_file: "journals.csv",
                journal_id: 48,
                timestamp: "2026-07-14T00:00:00Z",
                change_event_context: Some(&change_event_context),
                lease_context: Some(&lease_context),
                should_resume: false,
                attempt_sink: &mut |_| {},
            },
        )
        .expect("retry should replay the prior overlapped window");

        let transport = retry_client.into_transport();
        assert_eq!(
            transport.journal_work_requests(),
            &[(("1234-5678").to_string(), Some("2026-06-13".to_string()))]
        );
    }

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
        assert_eq!(outcome.written_article_count, 1);
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

    fn scholarly_row() -> CsvRow {
        CsvRow::from([
            ("source".to_string(), "scholarly".to_string()),
            ("title".to_string(), "Paged Journal".to_string()),
            ("issn".to_string(), "1234-5678".to_string()),
            ("id".to_string(), "paged-journal".to_string()),
        ])
    }

    fn seed_scholarly_journal(connection: &Connection, journal_id: i64, title: &str) {
        connection
            .execute(
                "
                INSERT INTO journals (journal_id, library_id, title, issn, has_articles)
                VALUES (?1, 'scholarly', ?2, '1234-5678', 1)
                ",
                rusqlite::params![journal_id, title],
            )
            .expect("existing journal should insert");
        connection
            .execute(
                "
                INSERT INTO journal_meta (journal_id, source_csv, csv_title)
                VALUES (?1, 'existing.csv', ?2)
                ",
                rusqlite::params![journal_id, title],
            )
            .expect("existing journal metadata should insert");
    }

    fn live_synchronization_context(
        connection: &Connection,
        run_id: &str,
        timestamp: &str,
    ) -> (IndexRunLeaseContext, ChangeEventContext) {
        begin_index_run(
            connection,
            &IndexRunStartRequest {
                run_id,
                csv_file: "journals.csv",
                started_at: timestamp,
                total_journals: 1,
                now_epoch_seconds: 4_000_000_000,
                should_adopt_events: false,
            },
        )
        .expect("live synchronization run should start");
        (
            IndexRunLeaseContext::new(run_id),
            ChangeEventContext::new(run_id, "worker-0", timestamp, false),
        )
    }

    fn crossref_work(doi: &str, title: &str, page: &str) -> serde_json::Value {
        json!({
            "DOI": doi,
            "title": [title],
            "author": [{"given": "Test", "family": "Author"}],
            "published": {"date-parts": [[2026, 7, 13]]},
            "volume": "1",
            "issue": "1",
            "page": page,
            "ISSN": ["1234-5678"]
        })
    }
}
