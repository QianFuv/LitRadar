//! Live CSV index orchestration for the legacy `index` command.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ps_sources::{
    CnkiClient, CnkiSourceError, CnkiTransport, LiveCnkiConfig, LiveCnkiTransport,
    LiveScholarlyConfig, LiveScholarlyTransport, ScholarlyClient, ScholarlyTransport,
    SourceAttempt, SourceError,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::cnki::{process_cnki_row, CnkiIndexConfig, CnkiIndexError};
use crate::manifest::{
    build_change_manifest_from_snapshots, collect_article_snapshot, write_change_manifest,
};
use crate::schema::{
    init_index_db, mark_article_listing_ready, mark_journal_done, mark_year_done,
    persist_index_run_stats,
};
use crate::scholarly::{process_scholarly_row, ScholarlyIndexError};
use crate::stats::{ApiCallStats, IndexRunStats, PathCountIncrements, PathStats};
use crate::transforms::{
    build_journal_id, journal_title_from_row, source_from_row, ArticleRecord, CsvRow,
};

const SCHOLARLY_SOURCE: &str = "scholarly";
const CNKI_SOURCE: &str = "cnki";
const LIVE_INDEX_WORKER_REQUEST_ENV: &str = "PAPER_SCANNER_LIVE_INDEX_WORKER_REQUEST";
const SQLITE_BUSY_TIMEOUT_SECONDS: u64 = 30;

/// Live index run configuration.
#[derive(Debug, Clone)]
pub struct LiveIndexConfig {
    /// Project root containing the `data` directory.
    pub project_root: PathBuf,
    /// Optional CSV filename under `data/meta`.
    pub file: Option<String>,
    /// Number of CNKI article-detail request workers per journal worker.
    pub worker_count: usize,
    /// Number of journal worker processes.
    pub process_count: usize,
    /// Number of issues processed together for CNKI.
    pub issue_batch_size: usize,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// Whether completed journals and years may be skipped.
    pub resume: bool,
    /// Whether to perform an update run and emit a change manifest.
    pub update: bool,
    /// Whether to run `notify` after an update manifest is written.
    pub notify: bool,
    /// Whether notify handoff should use dry-run mode.
    pub notify_dry_run: bool,
}

/// Live index command outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveIndexOutcome {
    /// Final run status.
    pub status: String,
    /// Human-readable message for skipped work.
    pub message: Option<String>,
    /// Per-CSV outcomes.
    pub csvs: Vec<LiveCsvIndexOutcome>,
}

/// Live index outcome for one CSV file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveCsvIndexOutcome {
    /// Source CSV path.
    pub csv_path: String,
    /// Output database path.
    pub db_path: String,
    /// Run identifier.
    pub run_id: String,
    /// Final run status.
    pub status: String,
    /// Indexed journal count.
    pub journal_count: usize,
    /// Written article ids.
    pub written_article_ids: Vec<i64>,
    /// Source attempt count.
    pub source_attempt_count: usize,
    /// Optional update manifest path.
    pub manifest_path: Option<String>,
    /// Optional notify process exit code.
    pub notify_exit_code: Option<i32>,
}

#[derive(Debug)]
struct LiveJournalOutcome {
    source: String,
    status: String,
    counts: PathCountIncrements,
    attempts: Vec<SourceAttempt>,
    written_articles: Vec<ArticleRecord>,
}

#[derive(Debug)]
struct LiveJournalFailure {
    source: String,
    attempts: Vec<SourceAttempt>,
    error: LiveIndexError,
}

struct LiveJournalContext<'a> {
    connection: &'a Connection,
    row: &'a CsvRow,
    csv_file: &'a str,
    journal_id: i64,
    timestamp: &'a str,
    cnki_config: &'a CnkiIndexConfig,
}

struct LiveJournalRowsContext<'a> {
    rows: &'a [CsvRow],
    csv_path: &'a Path,
    db_path: &'a Path,
    csv_file: &'a str,
    run_id: &'a str,
    timestamp: &'a str,
    config: &'a LiveIndexConfig,
}

#[derive(Debug)]
struct LiveJournalRowsOutcome {
    stats: IndexRunStats,
    written_article_ids: Vec<i64>,
    source_attempt_count: usize,
}

#[derive(Debug)]
struct LiveJournalRowsFailure {
    partial: LiveJournalRowsOutcome,
    error: LiveIndexError,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LiveIndexWorkerRequest {
    project_root: PathBuf,
    csv_path: PathBuf,
    db_path: PathBuf,
    csv_file: String,
    run_id: String,
    timestamp: String,
    worker_id: usize,
    process_count: usize,
    worker_count: usize,
    issue_batch_size: usize,
    timeout_seconds: u64,
    resume: bool,
    update: bool,
    rows: Vec<CsvRow>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LiveIndexWorkerResponse {
    worker_id: usize,
    status: String,
    stats: LiveIndexWorkerStats,
    written_article_ids: Vec<i64>,
    source_attempt_count: usize,
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LiveIndexWorkerStats {
    run_id: String,
    csv_file: String,
    started_at: String,
    finished_at: Option<String>,
    status: String,
    total_journals: i64,
    succeeded_journals: i64,
    failed_journals: i64,
    resumed_journals: i64,
    error_summary: Option<String>,
    path_stats: Vec<PathStats>,
    api_stats: Vec<ApiCallStats>,
}

trait LiveWorkerLauncher {
    fn run_workers(
        &self,
        requests: Vec<LiveIndexWorkerRequest>,
    ) -> Result<Vec<LiveIndexWorkerResponse>, LiveIndexError>;
}

struct ProcessLiveWorkerLauncher {
    command_path: PathBuf,
}

struct SpawnedLiveWorker {
    worker_id: usize,
    request_path: PathBuf,
    child: Child,
}

/// Live index workflow errors.
#[derive(Debug)]
pub enum LiveIndexError {
    /// IO operation failed.
    Io(std::io::Error),
    /// SQLite operation failed.
    Sqlite(rusqlite::Error),
    /// JSON encoding or decoding failed.
    Json(serde_json::Error),
    /// Scholarly source operation failed.
    Source(SourceError),
    /// CNKI source operation failed.
    CnkiSource(CnkiSourceError),
    /// Scholarly index row failed.
    Scholarly(ScholarlyIndexError),
    /// CNKI index row failed.
    Cnki(CnkiIndexError),
    /// A CSV row has an unsupported source.
    UnsupportedSource(String),
    /// Required runtime configuration is missing.
    MissingConfig(String),
    /// Runtime configuration is invalid.
    InvalidConfig(String),
    /// Internal journal worker failed.
    Worker(String),
    /// Notify handoff failed.
    Notify(String),
}

impl fmt::Display for LiveIndexError {
    /// Format the live index error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Source(error) => write!(formatter, "{error}"),
            Self::CnkiSource(error) => write!(formatter, "{error}"),
            Self::Scholarly(error) => write!(formatter, "{error}"),
            Self::Cnki(error) => write!(formatter, "{error}"),
            Self::UnsupportedSource(message) => formatter.write_str(message),
            Self::MissingConfig(message) => formatter.write_str(message),
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::Worker(message) => formatter.write_str(message),
            Self::Notify(message) => formatter.write_str(message),
        }
    }
}

impl Error for LiveIndexError {
    /// Return the underlying error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Source(error) => Some(error),
            Self::CnkiSource(error) => Some(error),
            Self::Scholarly(error) => Some(error),
            Self::Cnki(error) => Some(error),
            Self::UnsupportedSource(_)
            | Self::MissingConfig(_)
            | Self::InvalidConfig(_)
            | Self::Worker(_)
            | Self::Notify(_) => None,
        }
    }
}

impl From<std::io::Error> for LiveIndexError {
    /// Convert IO errors into live index errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for LiveIndexError {
    /// Convert SQLite errors into live index errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for LiveIndexError {
    /// Convert JSON errors into live index errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<SourceError> for LiveIndexError {
    /// Convert source errors into live index errors.
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

impl From<CnkiSourceError> for LiveIndexError {
    /// Convert CNKI source errors into live index errors.
    fn from(error: CnkiSourceError) -> Self {
        Self::CnkiSource(error)
    }
}

impl From<ScholarlyIndexError> for LiveIndexError {
    /// Convert Scholarly row errors into live index errors.
    fn from(error: ScholarlyIndexError) -> Self {
        Self::Scholarly(error)
    }
}

impl From<CnkiIndexError> for LiveIndexError {
    /// Convert CNKI row errors into live index errors.
    fn from(error: CnkiIndexError) -> Self {
        Self::Cnki(error)
    }
}

impl From<IndexRunStats> for LiveIndexWorkerStats {
    fn from(stats: IndexRunStats) -> Self {
        Self {
            run_id: stats.run_id,
            csv_file: stats.csv_file,
            started_at: stats.started_at,
            finished_at: stats.finished_at,
            status: stats.status,
            total_journals: stats.total_journals,
            succeeded_journals: stats.succeeded_journals,
            failed_journals: stats.failed_journals,
            resumed_journals: stats.resumed_journals,
            error_summary: stats.error_summary,
            path_stats: stats.path_stats.into_values().collect(),
            api_stats: stats.api_stats.into_values().collect(),
        }
    }
}

impl LiveIndexWorkerStats {
    fn into_index_run_stats(self) -> IndexRunStats {
        let mut stats = IndexRunStats::new(self.run_id, self.csv_file, self.started_at);
        stats.finished_at = self.finished_at;
        stats.status = self.status;
        stats.total_journals = self.total_journals;
        stats.succeeded_journals = self.succeeded_journals;
        stats.failed_journals = self.failed_journals;
        stats.resumed_journals = self.resumed_journals;
        stats.error_summary = self.error_summary;
        for path_stats in self.path_stats {
            stats.path_stats.insert(path_stats.key.clone(), path_stats);
        }
        for api_stats in self.api_stats {
            stats.api_stats.insert(api_stats.key.clone(), api_stats);
        }
        stats
    }
}

impl ProcessLiveWorkerLauncher {
    fn new(command_path: PathBuf) -> Self {
        Self { command_path }
    }
}

impl LiveWorkerLauncher for ProcessLiveWorkerLauncher {
    fn run_workers(
        &self,
        requests: Vec<LiveIndexWorkerRequest>,
    ) -> Result<Vec<LiveIndexWorkerResponse>, LiveIndexError> {
        let mut spawned_workers = Vec::new();
        for request in &requests {
            let request_path = write_live_worker_request_file(request)?;
            let child = Command::new(&self.command_path)
                .env(LIVE_INDEX_WORKER_REQUEST_ENV, &request_path)
                .env("PAPER_SCANNER_PROJECT_ROOT", &request.project_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| {
                    LiveIndexError::Worker(format!(
                        "failed to spawn live index worker {}: {error}",
                        request.worker_id
                    ))
                })?;
            spawned_workers.push(SpawnedLiveWorker {
                worker_id: request.worker_id,
                request_path,
                child,
            });
        }

        let mut responses = Vec::new();
        for spawned_worker in spawned_workers {
            let output = spawned_worker.child.wait_with_output().map_err(|error| {
                LiveIndexError::Worker(format!(
                    "failed to wait for live index worker {}: {error}",
                    spawned_worker.worker_id
                ))
            })?;
            let _ = fs::remove_file(&spawned_worker.request_path);
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(LiveIndexError::Worker(format!(
                    "live index worker {} exited with {}: {}",
                    spawned_worker.worker_id,
                    output.status,
                    stderr.trim()
                )));
            }
            let mut response: LiveIndexWorkerResponse = serde_json::from_slice(&output.stdout)?;
            if response.worker_id != spawned_worker.worker_id {
                return Err(LiveIndexError::Worker(format!(
                    "live index worker {} returned response for worker {}",
                    spawned_worker.worker_id, response.worker_id
                )));
            }
            response.written_article_ids.sort_unstable();
            responses.push(response);
        }
        responses.sort_by_key(|response| response.worker_id);
        Ok(responses)
    }
}

/// Run live indexing for the legacy `index` command.
///
/// # Arguments
///
/// * `config` - Live index configuration.
///
/// # Returns
///
/// Live index outcome.
pub fn run_live_index(config: &LiveIndexConfig) -> Result<LiveIndexOutcome, LiveIndexError> {
    validate_live_concurrency_config(config)?;

    let meta_dir = config.project_root.join("data").join("meta");
    let index_dir = config.project_root.join("data").join("index");
    if !meta_dir.exists() {
        return Ok(LiveIndexOutcome {
            status: "skipped".to_string(),
            message: Some(format!("Directory not found: {}", meta_dir.display())),
            csvs: Vec::new(),
        });
    }
    fs::create_dir_all(&index_dir)?;
    let csv_paths = csv_paths(&meta_dir, config.file.as_deref())?;
    if csv_paths.is_empty() {
        return Ok(LiveIndexOutcome {
            status: "skipped".to_string(),
            message: Some(format!("No CSV files found in {}", meta_dir.display())),
            csvs: Vec::new(),
        });
    }

    let mut outcomes = Vec::new();
    for csv_path in csv_paths {
        let stem = csv_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("index");
        let db_path = index_dir.join(format!("{stem}.sqlite"));
        outcomes.push(run_live_csv_index(config, &csv_path, &db_path)?);
    }
    Ok(LiveIndexOutcome {
        status: "succeeded".to_string(),
        message: None,
        csvs: outcomes,
    })
}

/// Run an internal live index worker when the worker environment is present.
///
/// # Returns
///
/// Serialized worker response when this process was launched as a worker.
pub fn run_live_index_worker_from_environment() -> Result<Option<String>, LiveIndexError> {
    let Ok(request_path) = env::var(LIVE_INDEX_WORKER_REQUEST_ENV) else {
        return Ok(None);
    };
    let response = run_live_index_worker_from_file(Path::new(&request_path))?;
    Ok(Some(serde_json::to_string(&response)?))
}

fn run_live_index_worker_from_file(
    request_path: &Path,
) -> Result<LiveIndexWorkerResponse, LiveIndexError> {
    let request: LiveIndexWorkerRequest = serde_json::from_str(&fs::read_to_string(request_path)?)?;
    run_live_index_worker(request)
}

fn run_live_index_worker(
    request: LiveIndexWorkerRequest,
) -> Result<LiveIndexWorkerResponse, LiveIndexError> {
    let connection = open_live_index_connection(&request.db_path)?;
    let config = LiveIndexConfig {
        project_root: request.project_root.clone(),
        file: None,
        worker_count: request.worker_count,
        process_count: 1,
        issue_batch_size: request.issue_batch_size,
        timeout_seconds: request.timeout_seconds,
        resume: request.resume,
        update: request.update,
        notify: false,
        notify_dry_run: true,
    };
    let context = LiveJournalRowsContext {
        rows: &request.rows,
        csv_path: &request.csv_path,
        db_path: &request.db_path,
        csv_file: &request.csv_file,
        run_id: &request.run_id,
        timestamp: &request.timestamp,
        config: &config,
    };

    let response = match run_live_journal_rows_locally(&connection, &context) {
        Ok(outcome) => LiveIndexWorkerResponse {
            worker_id: request.worker_id,
            status: "succeeded".to_string(),
            stats: outcome.stats.into(),
            written_article_ids: outcome.written_article_ids,
            source_attempt_count: outcome.source_attempt_count,
            error: None,
        },
        Err(failure) => {
            let failure = *failure;
            LiveIndexWorkerResponse {
                worker_id: request.worker_id,
                status: "failed".to_string(),
                stats: failure.partial.stats.into(),
                written_article_ids: failure.partial.written_article_ids,
                source_attempt_count: failure.partial.source_attempt_count,
                error: Some(failure.error.to_string()),
            }
        }
    };
    Ok(response)
}

fn open_live_index_connection(db_path: &Path) -> Result<Connection, LiveIndexError> {
    let connection = Connection::open(db_path)?;
    connection.busy_timeout(Duration::from_secs(SQLITE_BUSY_TIMEOUT_SECONDS))?;
    init_index_db(&connection)?;
    Ok(connection)
}

fn write_live_worker_request_file(
    request: &LiveIndexWorkerRequest,
) -> Result<PathBuf, LiveIndexError> {
    let request_path = live_worker_request_path(request.worker_id);
    fs::write(&request_path, serde_json::to_vec(request)?)?;
    Ok(request_path)
}

fn live_worker_request_path(worker_id: usize) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    env::temp_dir().join(format!(
        "paper-scanner-live-worker-{}-{nanos}-{worker_id}.json",
        std::process::id()
    ))
}

fn validate_live_concurrency_config(config: &LiveIndexConfig) -> Result<(), LiveIndexError> {
    if config.worker_count == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "worker_count must be at least 1".to_string(),
        ));
    }
    if config.process_count == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "process_count must be at least 1".to_string(),
        ));
    }
    if config.issue_batch_size == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "issue_batch_size must be at least 1".to_string(),
        ));
    }
    Ok(())
}

fn record_live_journal_attempts(
    stats: &mut IndexRunStats,
    source: &str,
    attempts: &[SourceAttempt],
    journal_id: i64,
    journal_title: &str,
) {
    stats.record_source_attempts_for_source(source, attempts, Some(journal_id), journal_title);
}

fn journal_failure(
    source: impl Into<String>,
    attempts: Vec<SourceAttempt>,
    error: impl Into<LiveIndexError>,
) -> Box<LiveJournalFailure> {
    Box::new(LiveJournalFailure {
        source: source.into(),
        attempts,
        error: error.into(),
    })
}

fn run_live_journal_rows_locally(
    connection: &Connection,
    context: &LiveJournalRowsContext<'_>,
) -> Result<LiveJournalRowsOutcome, Box<LiveJournalRowsFailure>> {
    let mut stats = IndexRunStats::new(
        context.run_id.to_string(),
        context.csv_file.to_string(),
        context.timestamp.to_string(),
    );
    if context.rows.is_empty() {
        stats.finish("succeeded", context.timestamp.to_string(), None);
        return Ok(LiveJournalRowsOutcome {
            stats,
            written_article_ids: Vec::new(),
            source_attempt_count: 0,
        });
    }

    let scholarly_config = LiveScholarlyConfig::from_environment(context.config.timeout_seconds);
    let mut scholarly_client = match LiveScholarlyTransport::new(scholarly_config.clone()) {
        Ok(transport) => {
            ScholarlyClient::new(transport, scholarly_config.has_semantic_scholar_key())
        }
        Err(error) => return Err(finish_live_rows_failure(stats, Vec::new(), 0, error.into())),
    };
    let mut cnki_client = match LiveCnkiTransport::new(LiveCnkiConfig {
        timeout_seconds: context.config.timeout_seconds,
    }) {
        Ok(transport) => CnkiClient::new(transport),
        Err(error) => return Err(finish_live_rows_failure(stats, Vec::new(), 0, error.into())),
    };
    let cnki_config = CnkiIndexConfig {
        csv_path: context.csv_path.to_path_buf(),
        fixture_path: PathBuf::new(),
        output_db_path: context.db_path.to_path_buf(),
        manifest_path: None,
        run_id: context.run_id.to_string(),
        timestamp: context.timestamp.to_string(),
        resume: context.config.resume,
        update: context.config.update,
        issue_batch_size: context.config.issue_batch_size.max(1),
        worker_count: context.config.worker_count.max(1),
    };
    let mut all_written_articles = Vec::new();

    for row in context.rows {
        let source = source_from_row(row);
        let journal_id = match build_journal_id(row) {
            Some(journal_id) => journal_id,
            None => {
                let error = LiveIndexError::UnsupportedSource(format!(
                    "Journal row missing id: {}",
                    journal_title_from_row(row)
                ));
                return Err(finish_live_rows_failure(
                    stats,
                    all_written_articles,
                    scholarly_client.attempts().len() + cnki_client.attempts().len(),
                    error,
                ));
            }
        };
        let journal_title = journal_title_from_row(row);
        let path_key = stats.start_path(
            &source,
            "journal",
            Some(journal_id),
            journal_title.clone(),
            context.timestamp.to_string(),
        );
        match process_live_journal_row(
            &mut scholarly_client,
            &mut cnki_client,
            LiveJournalContext {
                connection,
                row,
                csv_file: context.csv_file,
                journal_id,
                timestamp: context.timestamp,
                cnki_config: &cnki_config,
            },
        ) {
            Ok(outcome) => {
                record_live_journal_attempts(
                    &mut stats,
                    &outcome.source,
                    &outcome.attempts,
                    journal_id,
                    &journal_title,
                );
                stats.record_path_counts(&path_key, outcome.counts);
                stats.finish_path(
                    &path_key,
                    &outcome.status,
                    context.timestamp.to_string(),
                    None,
                );
                all_written_articles.extend(outcome.written_articles);
            }
            Err(failure) => {
                let LiveJournalFailure {
                    source,
                    attempts,
                    error,
                } = *failure;
                record_live_journal_attempts(
                    &mut stats,
                    &source,
                    &attempts,
                    journal_id,
                    &journal_title,
                );
                stats.finish_path(
                    &path_key,
                    "failed",
                    context.timestamp.to_string(),
                    Some(&error.to_string()),
                );
                return Err(finish_live_rows_failure(
                    stats,
                    all_written_articles,
                    scholarly_client.attempts().len() + cnki_client.attempts().len(),
                    error,
                ));
            }
        }
    }

    stats.finish("succeeded", context.timestamp.to_string(), None);
    Ok(live_rows_outcome(
        stats,
        all_written_articles,
        scholarly_client.attempts().len() + cnki_client.attempts().len(),
    ))
}

fn finish_live_rows_failure(
    mut stats: IndexRunStats,
    written_articles: Vec<ArticleRecord>,
    source_attempt_count: usize,
    error: LiveIndexError,
) -> Box<LiveJournalRowsFailure> {
    let finished_at = stats.started_at.clone();
    stats.finish("failed", finished_at, Some(error.to_string()));
    Box::new(LiveJournalRowsFailure {
        partial: live_rows_outcome(stats, written_articles, source_attempt_count),
        error,
    })
}

fn live_rows_outcome(
    stats: IndexRunStats,
    mut written_articles: Vec<ArticleRecord>,
    source_attempt_count: usize,
) -> LiveJournalRowsOutcome {
    written_articles.sort_by_key(|article| article.article_id);
    LiveJournalRowsOutcome {
        stats,
        written_article_ids: written_articles
            .into_iter()
            .map(|article| article.article_id)
            .collect(),
        source_attempt_count,
    }
}

fn run_live_journal_rows_in_worker_processes(
    context: &LiveJournalRowsContext<'_>,
    launcher: &dyn LiveWorkerLauncher,
) -> Result<LiveJournalRowsOutcome, Box<LiveJournalRowsFailure>> {
    let requests = build_live_worker_requests(context);
    let responses = match launcher.run_workers(requests) {
        Ok(responses) => responses,
        Err(error) => {
            let stats = IndexRunStats::new(
                context.run_id.to_string(),
                context.csv_file.to_string(),
                context.timestamp.to_string(),
            );
            return Err(finish_live_rows_failure(stats, Vec::new(), 0, error));
        }
    };

    let mut stats = IndexRunStats::new(
        context.run_id.to_string(),
        context.csv_file.to_string(),
        context.timestamp.to_string(),
    );
    let mut written_article_ids = Vec::new();
    let mut source_attempt_count = 0;
    let mut errors = Vec::new();
    for response in responses {
        stats.merge_worker_stats(response.stats.into_index_run_stats());
        written_article_ids.extend(response.written_article_ids);
        source_attempt_count += response.source_attempt_count;
        if response.status != "succeeded" {
            errors.push(
                response
                    .error
                    .unwrap_or_else(|| format!("worker {} failed", response.worker_id)),
            );
        }
    }
    written_article_ids.sort_unstable();
    written_article_ids.dedup();
    if errors.is_empty() {
        stats.finish("succeeded", context.timestamp.to_string(), None);
        Ok(LiveJournalRowsOutcome {
            stats,
            written_article_ids,
            source_attempt_count,
        })
    } else {
        let error = LiveIndexError::Worker(errors.join("; "));
        stats.finish(
            "failed",
            context.timestamp.to_string(),
            Some(error.to_string()),
        );
        Err(Box::new(LiveJournalRowsFailure {
            partial: LiveJournalRowsOutcome {
                stats,
                written_article_ids,
                source_attempt_count,
            },
            error,
        }))
    }
}

fn build_live_worker_requests(context: &LiveJournalRowsContext<'_>) -> Vec<LiveIndexWorkerRequest> {
    partition_live_worker_rows(context.rows, context.config.process_count)
        .into_iter()
        .enumerate()
        .filter_map(|(worker_id, rows)| {
            if rows.is_empty() {
                return None;
            }
            Some(LiveIndexWorkerRequest {
                project_root: context.config.project_root.clone(),
                csv_path: context.csv_path.to_path_buf(),
                db_path: context.db_path.to_path_buf(),
                csv_file: context.csv_file.to_string(),
                run_id: context.run_id.to_string(),
                timestamp: context.timestamp.to_string(),
                worker_id,
                process_count: context.config.process_count,
                worker_count: context.config.worker_count,
                issue_batch_size: context.config.issue_batch_size,
                timeout_seconds: context.config.timeout_seconds,
                resume: context.config.resume,
                update: context.config.update,
                rows,
            })
        })
        .collect()
}

fn partition_live_worker_rows(rows: &[CsvRow], process_count: usize) -> Vec<Vec<CsvRow>> {
    let worker_count = process_count.min(rows.len()).max(1);
    let mut partitions = vec![Vec::new(); worker_count];
    for (row_index, row) in rows.iter().enumerate() {
        partitions[row_index % worker_count].push(row.clone());
    }
    partitions
}

fn process_live_journal_row<S, C>(
    scholarly_client: &mut ScholarlyClient<S>,
    cnki_client: &mut CnkiClient<C>,
    context: LiveJournalContext<'_>,
) -> Result<LiveJournalOutcome, Box<LiveJournalFailure>>
where
    S: ScholarlyTransport,
    C: CnkiTransport + Clone + Send + 'static,
{
    let LiveJournalContext {
        connection,
        row,
        csv_file,
        journal_id,
        timestamp,
        cnki_config,
    } = context;

    match source_from_row(row).as_str() {
        SCHOLARLY_SOURCE => {
            let attempt_start = scholarly_client.attempts().len();
            let result = process_scholarly_row(
                connection,
                scholarly_client,
                row,
                csv_file,
                journal_id,
                timestamp,
            );
            let attempts = scholarly_client.attempts()[attempt_start..].to_vec();
            match result {
                Ok(outcome) => {
                    for year in outcome.years {
                        mark_year_done(connection, journal_id, year, timestamp).map_err(
                            |error| journal_failure(SCHOLARLY_SOURCE, attempts.clone(), error),
                        )?;
                    }
                    mark_journal_done(connection, journal_id, timestamp).map_err(|error| {
                        journal_failure(SCHOLARLY_SOURCE, attempts.clone(), error)
                    })?;
                    Ok(LiveJournalOutcome {
                        source: SCHOLARLY_SOURCE.to_string(),
                        status: "succeeded".to_string(),
                        counts: PathCountIncrements {
                            works_count: outcome.works_count,
                            issues_count: outcome.issues_count,
                            articles_written_count: outcome.written_articles.len() as i64,
                            articles_deleted_no_authors_count: outcome.deleted_article_count,
                            ..PathCountIncrements::default()
                        },
                        attempts,
                        written_articles: outcome.written_articles,
                    })
                }
                Err(error) => Err(journal_failure(SCHOLARLY_SOURCE, attempts, error)),
            }
        }
        CNKI_SOURCE => {
            let attempt_start = cnki_client.attempts().len();
            let result = process_cnki_row(
                connection,
                cnki_client,
                row,
                csv_file,
                journal_id,
                cnki_config,
            );
            let attempts = cnki_client.attempts()[attempt_start..].to_vec();
            match result {
                Ok(outcome) => Ok(LiveJournalOutcome {
                    source: CNKI_SOURCE.to_string(),
                    status: outcome.status,
                    counts: PathCountIncrements {
                        issues_count: outcome.issues_count,
                        article_summaries_count: outcome.article_summaries_count,
                        article_details_count: outcome.article_details_count,
                        articles_written_count: outcome.written_articles.len() as i64,
                        articles_deleted_no_authors_count: outcome.deleted_article_count,
                        ..PathCountIncrements::default()
                    },
                    attempts,
                    written_articles: outcome.written_articles,
                }),
                Err(error) => Err(journal_failure(CNKI_SOURCE, attempts, error)),
            }
        }
        other => Err(journal_failure(
            other,
            Vec::new(),
            LiveIndexError::UnsupportedSource(format!(
                "Unsupported source for {}: {other}",
                journal_title_from_row(row)
            )),
        )),
    }
}

fn run_live_csv_index(
    config: &LiveIndexConfig,
    csv_path: &Path,
    db_path: &Path,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let rows = read_csv_rows(csv_path)?;
    if rows.is_empty() {
        return Ok(LiveCsvIndexOutcome {
            csv_path: csv_path.display().to_string(),
            db_path: db_path.display().to_string(),
            run_id: String::new(),
            status: "skipped".to_string(),
            journal_count: 0,
            written_article_ids: Vec::new(),
            source_attempt_count: 0,
            manifest_path: None,
            notify_exit_code: None,
        });
    }
    validate_sources(&rows)?;
    let scholarly_config = LiveScholarlyConfig::from_environment(config.timeout_seconds);
    validate_required_source_config(&rows, &scholarly_config)?;
    let timestamp = default_timestamp();
    let csv_file = csv_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("journals.csv")
        .to_string();
    let db_name = db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("index.sqlite")
        .to_string();
    let run_id = format!(
        "{}-{timestamp}",
        csv_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("index")
    );

    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = open_live_index_connection(db_path)?;
    let before_snapshot = if config.update {
        Some(collect_article_snapshot(&connection)?)
    } else {
        None
    };
    let journal_context = LiveJournalRowsContext {
        rows: &rows,
        csv_path,
        db_path,
        csv_file: &csv_file,
        run_id: &run_id,
        timestamp: &timestamp,
        config,
    };
    let journal_rows_outcome = if config.process_count > 1 && rows.len() > 1 {
        let command_path = env::current_exe().map_err(|error| {
            LiveIndexError::Worker(format!("failed to resolve current executable: {error}"))
        })?;
        let launcher = ProcessLiveWorkerLauncher::new(command_path);
        run_live_journal_rows_in_worker_processes(&journal_context, &launcher)
    } else {
        run_live_journal_rows_locally(&connection, &journal_context)
    };
    let journal_rows_outcome = match journal_rows_outcome {
        Ok(outcome) => outcome,
        Err(failure) => {
            let failure = *failure;
            persist_index_run_stats(&connection, &failure.partial.stats)?;
            return Err(failure.error);
        }
    };
    persist_index_run_stats(&connection, &journal_rows_outcome.stats)?;
    let mut manifest_path = None;
    if let Some(before_snapshot) = before_snapshot {
        let after_snapshot = collect_article_snapshot(&connection)?;
        let path = config
            .project_root
            .join("data")
            .join("push_state")
            .join(format!(
                "{}.changes.json",
                db_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("index")
            ));
        let manifest = build_change_manifest_from_snapshots(
            &db_name,
            db_path,
            &run_id,
            &timestamp,
            &before_snapshot,
            &after_snapshot,
        );
        write_change_manifest(&manifest, &path)?;
        manifest_path = Some(path);
    }
    let notify_exit_code = if config.notify {
        let Some(path) = manifest_path.as_ref() else {
            return Err(LiveIndexError::Notify(
                "--notify requires an update manifest".to_string(),
            ));
        };
        Some(run_notify_for_manifest(config, &db_name, path)?)
    } else {
        None
    };
    mark_article_listing_ready(&connection, &timestamp)?;

    Ok(LiveCsvIndexOutcome {
        csv_path: csv_path.display().to_string(),
        db_path: db_path.display().to_string(),
        run_id,
        status: "succeeded".to_string(),
        journal_count: rows.len(),
        written_article_ids: journal_rows_outcome.written_article_ids,
        source_attempt_count: journal_rows_outcome.source_attempt_count,
        manifest_path: manifest_path.map(|path| path.display().to_string()),
        notify_exit_code,
    })
}

fn csv_paths(meta_dir: &Path, file: Option<&str>) -> Result<Vec<PathBuf>, LiveIndexError> {
    if let Some(file) = file {
        let path = meta_dir.join(file);
        return if path.exists() {
            Ok(vec![path])
        } else {
            Ok(Vec::new())
        };
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(meta_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("csv") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_csv_rows(path: &Path) -> Result<Vec<CsvRow>, LiveIndexError> {
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
        let source = row
            .get("source")
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| SCHOLARLY_SOURCE.to_string());
        row.insert("source".to_string(), source);
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

fn validate_sources(rows: &[CsvRow]) -> Result<(), LiveIndexError> {
    for row in rows {
        let source = source_from_row(row);
        if source != SCHOLARLY_SOURCE && source != CNKI_SOURCE {
            return Err(LiveIndexError::UnsupportedSource(format!(
                "Unsupported source for {}: {source}",
                journal_title_from_row(row)
            )));
        }
    }
    Ok(())
}

fn validate_required_source_config(
    rows: &[CsvRow],
    config: &LiveScholarlyConfig,
) -> Result<(), LiveIndexError> {
    let has_scholarly_rows = rows
        .iter()
        .any(|row| source_from_row(row) == SCHOLARLY_SOURCE);
    if !has_scholarly_rows {
        return Ok(());
    }
    if config.openalex_api_keys.is_empty() {
        return Err(LiveIndexError::MissingConfig(
            "OpenAlex API key is required for scholarly indexing.".to_string(),
        ));
    }
    if !config.has_semantic_scholar_key() {
        return Err(LiveIndexError::MissingConfig(
            "Semantic Scholar API key is required for scholarly indexing.".to_string(),
        ));
    }
    Ok(())
}

fn run_notify_for_manifest(
    config: &LiveIndexConfig,
    db_name: &str,
    manifest_path: &Path,
) -> Result<i32, LiveIndexError> {
    run_notify_command_for_manifest(Path::new("notify"), config, db_name, manifest_path)
}

fn run_notify_command_for_manifest(
    command_path: &Path,
    config: &LiveIndexConfig,
    db_name: &str,
    manifest_path: &Path,
) -> Result<i32, LiveIndexError> {
    let state_dir = config.project_root.join("data").join("push_state");
    let mut command = Command::new(command_path);
    command
        .arg("--db")
        .arg(db_name)
        .arg("--changes-file")
        .arg(manifest_path)
        .arg("--state-dir")
        .arg(&state_dir)
        .env("PAPER_SCANNER_PROJECT_ROOT", &config.project_root);
    if config.notify_dry_run {
        command.arg("--dry-run");
    }
    let status = command
        .status()
        .map_err(|error| LiveIndexError::Notify(error.to_string()))?;
    Ok(status.code().unwrap_or(1))
}

fn default_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    use ps_sources::LiveScholarlyConfig;
    use tempfile::tempdir;

    use super::{
        csv_paths, parse_csv_line, read_csv_rows, run_live_index,
        run_live_journal_rows_in_worker_processes, run_notify_command_for_manifest,
        validate_required_source_config, validate_sources, LiveIndexConfig, LiveIndexError,
        LiveIndexWorkerRequest, LiveIndexWorkerResponse, LiveIndexWorkerStats,
        LiveJournalRowsContext, LiveWorkerLauncher,
    };
    use crate::stats::IndexRunStats;
    use crate::transforms::{build_journal_id, journal_title_from_row, source_from_row, CsvRow};

    #[test]
    fn csv_parser_handles_quotes() {
        assert_eq!(
            parse_csv_line(r#"source,title,issn"#),
            vec!["source", "title", "issn"]
        );
        assert_eq!(
            parse_csv_line(r#"scholarly,"A, B",1234-5678"#),
            vec!["scholarly", "A, B", "1234-5678"]
        );
    }

    #[test]
    fn source_validation_rejects_unknown_values() {
        let row = CsvRow::from([
            ("source".to_string(), "unknown".to_string()),
            ("title".to_string(), "Bad Source".to_string()),
        ]);

        assert!(validate_sources(&[row]).is_err());
    }

    #[test]
    fn csv_path_discovery_sorts_csvs_and_respects_explicit_file() {
        let root = tempdir().expect("temp root should be created");
        fs::write(root.path().join("b.csv"), "source,title\n").expect("b csv should be written");
        fs::write(root.path().join("notes.txt"), "ignored").expect("text file should be written");
        fs::write(root.path().join("a.csv"), "source,title\n").expect("a csv should be written");

        assert_eq!(
            csv_file_names(&csv_paths(root.path(), None).expect("csvs should be listed")),
            vec!["a.csv", "b.csv"]
        );
        assert_eq!(
            csv_file_names(&csv_paths(root.path(), Some("b.csv")).expect("csv should be selected")),
            vec!["b.csv"]
        );
        assert!(csv_paths(root.path(), Some("missing.csv"))
            .expect("missing explicit csv should not fail")
            .is_empty());
    }

    #[test]
    fn live_index_skips_missing_or_empty_meta_inputs() {
        let root = tempdir().expect("temp root should be created");
        let missing_meta = run_live_index(&live_config(root.path()))
            .expect("missing meta dir should return a skipped outcome");

        assert_eq!(missing_meta.status, "skipped");
        assert!(missing_meta
            .message
            .as_deref()
            .expect("missing meta should explain skip")
            .contains("Directory not found"));

        fs::create_dir_all(root.path().join("data").join("meta"))
            .expect("meta dir should be created");
        let empty_meta =
            run_live_index(&live_config(root.path())).expect("empty meta dir should skip");

        assert_eq!(empty_meta.status, "skipped");
        assert!(empty_meta
            .message
            .as_deref()
            .expect("empty meta should explain skip")
            .contains("No CSV files"));
    }

    #[test]
    fn live_index_rejects_zero_concurrency_config() {
        let root = tempdir().expect("temp root should be created");

        let mut zero_workers = live_config(root.path());
        zero_workers.worker_count = 0;
        let worker_error =
            run_live_index(&zero_workers).expect_err("zero workers should fail fast");
        assert!(matches!(
            worker_error,
            LiveIndexError::InvalidConfig(message) if message.contains("worker_count")
        ));

        let mut zero_processes = live_config(root.path());
        zero_processes.process_count = 0;
        let process_error =
            run_live_index(&zero_processes).expect_err("zero processes should fail fast");
        assert!(matches!(
            process_error,
            LiveIndexError::InvalidConfig(message) if message.contains("process_count")
        ));

        let mut zero_issue_batch = live_config(root.path());
        zero_issue_batch.issue_batch_size = 0;
        let issue_batch_error =
            run_live_index(&zero_issue_batch).expect_err("zero issue batch should fail fast");
        assert!(matches!(
            issue_batch_error,
            LiveIndexError::InvalidConfig(message) if message.contains("issue_batch_size")
        ));
    }

    #[test]
    fn live_index_reports_empty_csv_without_network_transports() {
        let root = tempdir().expect("temp root should be created");
        let meta_dir = root.path().join("data").join("meta");
        fs::create_dir_all(&meta_dir).expect("meta dir should be created");
        fs::write(meta_dir.join("journals.csv"), "source,title,issn\n")
            .expect("empty csv should be written");

        let outcome = run_live_index(&live_config(root.path()))
            .expect("empty csv should not construct live transports");

        assert_eq!(outcome.status, "succeeded");
        assert_eq!(outcome.csvs.len(), 1);
        assert_eq!(outcome.csvs[0].status, "skipped");
        assert!(outcome.csvs[0].written_article_ids.is_empty());
    }

    #[test]
    fn live_index_rejects_unsupported_source_before_live_transports() {
        let root = tempdir().expect("temp root should be created");
        let meta_dir = root.path().join("data").join("meta");
        fs::create_dir_all(&meta_dir).expect("meta dir should be created");
        fs::write(
            meta_dir.join("selected.csv"),
            "source,title,issn\nunknown,Bad Source,1234-5678\n",
        )
        .expect("csv should be written");

        let error = run_live_index(&LiveIndexConfig {
            file: Some("selected.csv".to_string()),
            ..live_config(root.path())
        })
        .expect_err("unsupported source should fail before transports");

        assert!(matches!(
            error,
            LiveIndexError::UnsupportedSource(message) if message.contains("Bad Source")
        ));
    }

    #[test]
    fn parallel_worker_requests_partition_rows_and_merge_parent_summary() {
        let root = tempdir().expect("temp root should be created");
        let rows = vec![
            worker_row("a", "Journal A"),
            worker_row("b", "Journal B"),
            worker_row("c", "Journal C"),
        ];
        let mut config = live_config(root.path());
        config.process_count = 2;
        let csv_path = root.path().join("data").join("meta").join("journals.csv");
        let db_path = root
            .path()
            .join("data")
            .join("index")
            .join("journals.sqlite");
        let context = worker_rows_context(&csv_path, &db_path, &config, &rows);
        let launcher = RecordingWorkerLauncher::default();

        let outcome = run_live_journal_rows_in_worker_processes(&context, &launcher)
            .expect("parallel worker rows should merge");

        assert_eq!(outcome.stats.status, "succeeded");
        assert_eq!(outcome.stats.total_journals, 3);
        assert_eq!(outcome.stats.succeeded_journals, 3);
        assert_eq!(outcome.source_attempt_count, 3);
        assert_eq!(outcome.written_article_ids, vec![1, 2, 1001]);

        let requests = launcher.requests.borrow();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            row_titles(&requests[0].rows),
            vec!["Journal A", "Journal C"]
        );
        assert_eq!(row_titles(&requests[1].rows), vec!["Journal B"]);
    }

    #[test]
    fn parallel_worker_failure_marks_parent_run_failed() {
        let root = tempdir().expect("temp root should be created");
        let rows = vec![worker_row("a", "Journal A"), worker_row("b", "Journal B")];
        let mut config = live_config(root.path());
        config.process_count = 2;
        let csv_path = root.path().join("data").join("meta").join("journals.csv");
        let db_path = root
            .path()
            .join("data")
            .join("index")
            .join("journals.sqlite");
        let context = worker_rows_context(&csv_path, &db_path, &config, &rows);
        let launcher = RecordingWorkerLauncher {
            failed_worker_id: Some(1),
            ..RecordingWorkerLauncher::default()
        };

        let failure = run_live_journal_rows_in_worker_processes(&context, &launcher)
            .expect_err("failed worker should fail parent run");
        let failure = *failure;

        assert!(matches!(
            failure.error,
            LiveIndexError::Worker(message) if message.contains("worker 1 failed")
        ));
        assert_eq!(failure.partial.stats.status, "failed");
        assert_eq!(failure.partial.stats.total_journals, 2);
        assert_eq!(failure.partial.stats.failed_journals, 1);
    }

    #[test]
    fn csv_reader_defaults_source_and_validates_required_scholarly_config() {
        let root = tempdir().expect("temp root should be created");
        let csv_path = root.path().join("journals.csv");
        fs::write(&csv_path, "title,issn\nJournal,1234-5678\n").expect("csv should be written");

        let rows = read_csv_rows(&csv_path).expect("csv should parse");
        let missing_config = validate_required_source_config(
            &rows,
            &LiveScholarlyConfig {
                timeout_seconds: 1,
                openalex_api_keys: Vec::new(),
                semantic_scholar_api_keys: Vec::new(),
                crossref_mailtos: Vec::new(),
            },
        )
        .expect_err("scholarly rows should require API configuration");

        assert_eq!(rows[0].get("source").map(String::as_str), Some("scholarly"));
        assert!(missing_config.to_string().contains("OpenAlex API key"));

        let semantic_missing = validate_required_source_config(
            &rows,
            &LiveScholarlyConfig {
                timeout_seconds: 1,
                openalex_api_keys: vec!["openalex".to_string()],
                semantic_scholar_api_keys: Vec::new(),
                crossref_mailtos: Vec::new(),
            },
        )
        .expect_err("scholarly rows should require Semantic Scholar configuration");
        assert!(semantic_missing
            .to_string()
            .contains("Semantic Scholar API key"));

        let cnki_only = CsvRow::from([
            ("source".to_string(), "cnki".to_string()),
            ("title".to_string(), "CNKI".to_string()),
        ]);
        validate_required_source_config(
            &[cnki_only],
            &LiveScholarlyConfig {
                timeout_seconds: 1,
                openalex_api_keys: Vec::new(),
                semantic_scholar_api_keys: Vec::new(),
                crossref_mailtos: Vec::new(),
            },
        )
        .expect("CNKI-only rows should not require scholarly configuration");
    }

    #[test]
    fn notify_command_helper_reports_exit_code_and_arguments() {
        let root = tempdir().expect("temp root should be created");
        let manifest_path = root
            .path()
            .join("data")
            .join("push_state")
            .join("fixture.changes.json");
        fs::create_dir_all(manifest_path.parent().expect("manifest should have parent"))
            .expect("manifest dir should be created");
        fs::write(&manifest_path, "{}").expect("manifest should be written");
        let command_path = write_notify_command(root.path());

        let exit_code = run_notify_command_for_manifest(
            &command_path,
            &live_config(root.path()),
            "fixture.sqlite",
            &manifest_path,
        )
        .expect("notify command should run");

        let args =
            fs::read_to_string(root.path().join("args.txt")).expect("args should be captured");
        assert_eq!(exit_code, 7);
        assert!(args.contains("--db"));
        assert!(args.contains("fixture.sqlite"));
        assert!(args.contains("--changes-file"));
        assert!(args.contains("fixture.changes.json"));
        assert!(args.contains("--state-dir"));
        assert!(args.contains("push_state"));
        assert!(args.contains("--dry-run"));
    }

    #[test]
    fn notify_command_helper_maps_spawn_failures() {
        let root = tempdir().expect("temp root should be created");
        let manifest_path = root.path().join("missing.changes.json");

        let error = run_notify_command_for_manifest(
            &root.path().join("missing-notify"),
            &live_config(root.path()),
            "fixture.sqlite",
            &manifest_path,
        )
        .expect_err("missing notify command should fail");

        assert!(matches!(error, LiveIndexError::Notify(message) if !message.is_empty()));
    }

    fn csv_file_names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .expect("csv path should have a UTF-8 filename")
                    .to_string()
            })
            .collect()
    }

    #[derive(Default)]
    struct RecordingWorkerLauncher {
        requests: RefCell<Vec<LiveIndexWorkerRequest>>,
        failed_worker_id: Option<usize>,
    }

    impl LiveWorkerLauncher for RecordingWorkerLauncher {
        fn run_workers(
            &self,
            requests: Vec<LiveIndexWorkerRequest>,
        ) -> Result<Vec<LiveIndexWorkerResponse>, LiveIndexError> {
            self.requests.replace(requests.clone());
            Ok(requests
                .iter()
                .map(|request| {
                    if self.failed_worker_id == Some(request.worker_id) {
                        worker_response(request, "failed", Some("worker 1 failed".to_string()))
                    } else {
                        worker_response(request, "succeeded", None)
                    }
                })
                .collect())
        }
    }

    fn worker_response(
        request: &LiveIndexWorkerRequest,
        status: &str,
        error: Option<String>,
    ) -> LiveIndexWorkerResponse {
        let mut stats = IndexRunStats::new(
            request.run_id.clone(),
            request.csv_file.clone(),
            request.timestamp.clone(),
        );
        let mut article_ids = Vec::new();
        for (row_index, row) in request.rows.iter().enumerate() {
            let path_key = stats.start_path(
                &source_from_row(row),
                "journal",
                build_journal_id(row),
                journal_title_from_row(row),
                request.timestamp.clone(),
            );
            stats.finish_path(
                &path_key,
                status,
                request.timestamp.clone(),
                error.as_deref(),
            );
            article_ids.push((request.worker_id as i64 * 1000) + row_index as i64 + 1);
        }
        stats.finish(status, request.timestamp.clone(), error.clone());
        LiveIndexWorkerResponse {
            worker_id: request.worker_id,
            status: status.to_string(),
            stats: LiveIndexWorkerStats::from(stats),
            written_article_ids: article_ids,
            source_attempt_count: request.rows.len(),
            error,
        }
    }

    fn worker_rows_context<'a>(
        csv_path: &'a Path,
        db_path: &'a Path,
        config: &'a LiveIndexConfig,
        rows: &'a [CsvRow],
    ) -> LiveJournalRowsContext<'a> {
        LiveJournalRowsContext {
            rows,
            csv_path,
            db_path,
            csv_file: "journals.csv",
            run_id: "run-test",
            timestamp: "2026-07-05T00:00:00Z",
            config,
        }
    }

    fn worker_row(id: &str, title: &str) -> CsvRow {
        CsvRow::from([
            ("source".to_string(), "scholarly".to_string()),
            ("id".to_string(), id.to_string()),
            ("title".to_string(), title.to_string()),
        ])
    }

    fn row_titles(rows: &[CsvRow]) -> Vec<&str> {
        rows.iter()
            .map(|row| row.get("title").map(String::as_str).unwrap_or(""))
            .collect()
    }

    #[cfg(windows)]
    fn write_notify_command(root: &Path) -> PathBuf {
        let path = root.join("notify.cmd");
        fs::write(
            &path,
            "@echo off\r\necho %* > \"%~dp0args.txt\"\r\nexit /b 7\r\n",
        )
        .expect("notify command should be written");
        path
    }

    #[cfg(not(windows))]
    fn write_notify_command(root: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = root.join("notify");
        fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$(dirname \"$0\")/args.txt\"\nexit 7\n",
        )
        .expect("notify command should be written");
        let mut permissions = fs::metadata(&path)
            .expect("notify command metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("notify command should be executable");
        path
    }

    fn live_config(root: &Path) -> LiveIndexConfig {
        LiveIndexConfig {
            project_root: root.to_path_buf(),
            file: None,
            worker_count: 32,
            process_count: 2,
            issue_batch_size: 10,
            timeout_seconds: 1,
            resume: false,
            update: false,
            notify: false,
            notify_dry_run: true,
        }
    }
}
