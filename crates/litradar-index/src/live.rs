//! Live CSV index orchestration for the unified application.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use litradar_sources::{
    CnkiClient, CnkiSourceError, CnkiTransport, LiveCnkiConfig, LiveCnkiTransport,
    LiveScholarlyConfig, LiveScholarlyTransport, ScholarlyClient, ScholarlyTransport,
    SourceAttempt, SourceError,
};
use rusqlite::{Connection, ErrorCode};
use serde::{Deserialize, Serialize};

use crate::changes::{write_change_manifest_from_events, ChangeWriteError};
use crate::cnki::{process_cnki_row, CnkiIndexConfig, CnkiIndexError, CnkiProcessContext};
use crate::schema::{
    begin_index_run, journal_catalog_entry_is_current, mark_article_listing_ready, open_index_db,
    optimize_index_db, persist_index_run_stats, sync_journal_catalog_entry,
    with_immediate_index_transaction, ChangeEventContext, IndexRunLeaseContext, IndexRunLeaseError,
    IndexRunStartRequest,
};
use crate::scholarly::{process_scholarly_row, ScholarlyIndexError, ScholarlyProcessContext};
use crate::stats::{ApiCallStats, IndexRunStats, PathCountIncrements, PathStats};
use crate::transforms::{
    build_journal_id, build_meta_record, journal_title_from_row, source_from_row, CsvRow,
    JournalRecord,
};

const SCHOLARLY_SOURCE: &str = "scholarly";
const CNKI_SOURCE: &str = "cnki";
const LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS: u64 = 30;

/// Live index run configuration.
#[derive(Debug, Clone)]
pub struct LiveIndexConfig {
    /// Canonical application executable used for worker and notification subprocesses.
    pub application_executable: PathBuf,
    /// Project root containing the `data` directory.
    pub project_root: PathBuf,
    /// Deployment secret key file forwarded to notification handoff.
    pub secret_key_file: PathBuf,
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
    /// Scholarly source runtime configuration.
    pub scholarly_config: LiveScholarlyConfig,
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
    /// Written article count.
    pub written_article_count: i64,
    /// Source attempt count.
    pub source_attempt_count: usize,
    /// Optional update manifest path.
    pub manifest_path: Option<String>,
    /// Optional notify process exit code.
    pub notify_exit_code: Option<i32>,
}

#[derive(Debug)]
struct LiveJournalOutcome {
    status: String,
    counts: PathCountIncrements,
}

#[derive(Debug)]
struct LiveJournalFailure {
    error: LiveIndexError,
}

struct LiveJournalContext<'a> {
    connection: &'a Connection,
    row: &'a CsvRow,
    csv_file: &'a str,
    journal_id: i64,
    timestamp: &'a str,
    cnki_config: &'a CnkiIndexConfig,
    change_event_context: Option<&'a ChangeEventContext>,
    lease_context: Option<&'a IndexRunLeaseContext>,
}

struct LiveJournalRowsContext<'a> {
    rows: &'a [CsvRow],
    csv_path: &'a Path,
    db_path: &'a Path,
    csv_file: &'a str,
    run_id: &'a str,
    timestamp: &'a str,
    lease_context: Option<IndexRunLeaseContext>,
    worker_id: usize,
    process_count: usize,
    config: &'a LiveIndexConfig,
}

#[derive(Debug)]
struct LiveJournalRowsOutcome {
    stats: IndexRunStats,
    written_article_count: i64,
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
    lease_run_id: Option<String>,
    worker_id: usize,
    process_count: usize,
    worker_count: usize,
    issue_batch_size: usize,
    timeout_seconds: u64,
    resume: bool,
    update: bool,
    scholarly_config: LiveScholarlyConfig,
    rows: Vec<CsvRow>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LiveIndexWorkerResponse {
    worker_id: usize,
    status: String,
    stats: LiveIndexWorkerStats,
    written_article_count: i64,
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
    child: Option<Child>,
}

#[derive(Debug, Clone, Copy)]
struct LiveRunTime {
    epoch_seconds: i64,
    epoch_nanoseconds: u128,
}

impl LiveRunTime {
    fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            epoch_seconds: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
            epoch_nanoseconds: duration.as_nanos(),
        }
    }

    fn timestamp(self) -> String {
        self.epoch_seconds.to_string()
    }

    fn run_id(self, stem: &str) -> String {
        format!("{stem}-{}", self.epoch_nanoseconds)
    }
}

#[derive(Debug, Default)]
struct LiveIndexHeartbeatState {
    should_stop: bool,
    error: Option<String>,
}

struct LiveIndexLeaseHeartbeat {
    state: Arc<(Mutex<LiveIndexHeartbeatState>, Condvar)>,
    handle: Option<JoinHandle<()>>,
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
    /// Durable run lease acquisition, heartbeat, or ownership failed.
    Lease(String),
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
            Self::Lease(message) => formatter.write_str(message),
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
            | Self::Notify(_)
            | Self::Lease(_) => None,
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

impl From<IndexRunLeaseError> for LiveIndexError {
    /// Convert durable lease failures into live index errors.
    fn from(error: IndexRunLeaseError) -> Self {
        Self::Lease(error.to_string())
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

impl From<ChangeWriteError> for LiveIndexError {
    /// Convert streamed manifest errors into live index errors.
    fn from(error: ChangeWriteError) -> Self {
        match error {
            ChangeWriteError::Sqlite(error) => Self::Sqlite(error),
            ChangeWriteError::Io(error) => Self::Io(error),
            ChangeWriteError::Json(error) => Self::Json(error),
        }
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

impl LiveIndexLeaseHeartbeat {
    fn start(
        db_path: &Path,
        lease_context: IndexRunLeaseContext,
        interval: Duration,
    ) -> Result<Self, LiveIndexError> {
        let connection = open_live_index_connection(db_path)?;
        let state = Arc::new((
            Mutex::new(LiveIndexHeartbeatState::default()),
            Condvar::new(),
        ));
        let thread_state = Arc::clone(&state);
        let handle = thread::Builder::new()
            .name("litradar-index-heartbeat".to_string())
            .spawn(move || {
                run_live_index_heartbeat(connection, lease_context, interval, thread_state)
            })?;
        Ok(Self {
            state,
            handle: Some(handle),
        })
    }

    fn check(&self) -> Result<(), LiveIndexError> {
        let state =
            self.state.0.lock().map_err(|_| {
                LiveIndexError::Lease("heartbeat state lock was poisoned".to_string())
            })?;
        if let Some(error) = &state.error {
            Err(LiveIndexError::Lease(error.clone()))
        } else {
            Ok(())
        }
    }

    fn stop_and_check(&mut self) -> Result<(), LiveIndexError> {
        {
            let mut state = self.state.0.lock().map_err(|_| {
                LiveIndexError::Lease("heartbeat state lock was poisoned".to_string())
            })?;
            state.should_stop = true;
            self.state.1.notify_all();
        }
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| LiveIndexError::Lease("heartbeat thread panicked".to_string()))?;
        }
        self.check()
    }
}

impl Drop for LiveIndexLeaseHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop_and_check();
    }
}

impl Drop for SpawnedLiveWorker {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.request_path);
    }
}

fn run_live_index_heartbeat(
    connection: Connection,
    lease_context: IndexRunLeaseContext,
    interval: Duration,
    state: Arc<(Mutex<LiveIndexHeartbeatState>, Condvar)>,
) {
    let (state_lock, wakeup) = &*state;
    let mut current_state = match state_lock.lock() {
        Ok(current_state) => current_state,
        Err(_) => return,
    };
    loop {
        if current_state.should_stop {
            return;
        }
        let wait_result = wakeup.wait_timeout(current_state, interval);
        let Ok((next_state, timeout)) = wait_result else {
            return;
        };
        current_state = next_state;
        if current_state.should_stop {
            return;
        }
        if !timeout.timed_out() {
            continue;
        }
        drop(current_state);
        let heartbeat_result = lease_context.heartbeat(&connection);
        current_state = match state_lock.lock() {
            Ok(current_state) => current_state,
            Err(_) => return,
        };
        if let Err(error) = heartbeat_result {
            if is_retryable_heartbeat_contention(&error) {
                continue;
            }
            current_state.error = Some(error.to_string());
            wakeup.notify_all();
            return;
        }
    }
}

fn is_retryable_heartbeat_contention(error: &IndexRunLeaseError) -> bool {
    match error {
        IndexRunLeaseError::Sqlite(error) => matches!(
            error.sqlite_error_code(),
            Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked)
        ),
        IndexRunLeaseError::ActiveLease { .. }
        | IndexRunLeaseError::OwnershipLost { .. }
        | IndexRunLeaseError::Clock(_) => false,
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
            let child = match live_worker_command(&self.command_path, &request_path).spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = fs::remove_file(&request_path);
                    return Err(LiveIndexError::Worker(format!(
                        "failed to spawn live index worker {}: {error}",
                        request.worker_id
                    )));
                }
            };
            spawned_workers.push(SpawnedLiveWorker {
                worker_id: request.worker_id,
                request_path,
                child: Some(child),
            });
        }

        let mut responses = Vec::new();
        for mut spawned_worker in spawned_workers {
            let child = spawned_worker
                .child
                .take()
                .expect("spawned worker should retain its child");
            let output = child.wait_with_output().map_err(|error| {
                LiveIndexError::Worker(format!(
                    "failed to wait for live index worker {}: {error}",
                    spawned_worker.worker_id
                ))
            })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(LiveIndexError::Worker(format!(
                    "live index worker {} exited with {}: {}",
                    spawned_worker.worker_id,
                    output.status,
                    stderr.trim()
                )));
            }
            let response: LiveIndexWorkerResponse = serde_json::from_slice(&output.stdout)?;
            if response.worker_id != spawned_worker.worker_id {
                return Err(LiveIndexError::Worker(format!(
                    "live index worker {} returned response for worker {}",
                    spawned_worker.worker_id, response.worker_id
                )));
            }
            responses.push(response);
        }
        responses.sort_by_key(|response| response.worker_id);
        Ok(responses)
    }
}

fn live_worker_command(application_executable: &Path, request_path: &Path) -> Command {
    let mut command = Command::new(application_executable);
    command
        .arg("index")
        .arg("--live-worker-request")
        .arg(request_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Run live indexing for the unified application's `index` command.
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

/// Run an internal live index worker from an explicit request file.
///
/// # Arguments
///
/// * `request_path` - Path to the serialized worker request.
///
/// # Returns
///
/// Serialized worker response.
pub fn run_live_index_worker_from_file_path(
    request_path: impl AsRef<Path>,
) -> Result<String, LiveIndexError> {
    let response = run_live_index_worker_from_file(request_path.as_ref())?;
    Ok(serde_json::to_string(&response)?)
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
    let lease_context = request
        .lease_run_id
        .as_deref()
        .map(IndexRunLeaseContext::new);
    let config = LiveIndexConfig {
        application_executable: PathBuf::new(),
        project_root: request.project_root.clone(),
        secret_key_file: PathBuf::new(),
        file: None,
        worker_count: request.worker_count,
        process_count: 1,
        issue_batch_size: request.issue_batch_size,
        timeout_seconds: request.timeout_seconds,
        resume: request.resume,
        update: request.update,
        notify: false,
        notify_dry_run: true,
        scholarly_config: request.scholarly_config.clone(),
    };
    let context = LiveJournalRowsContext {
        rows: &request.rows,
        csv_path: &request.csv_path,
        db_path: &request.db_path,
        csv_file: &request.csv_file,
        run_id: &request.run_id,
        timestamp: &request.timestamp,
        lease_context,
        worker_id: request.worker_id,
        process_count: request.process_count,
        config: &config,
    };

    let response = match run_live_journal_rows_locally(&connection, &context) {
        Ok(outcome) => LiveIndexWorkerResponse {
            worker_id: request.worker_id,
            status: "succeeded".to_string(),
            stats: outcome.stats.into(),
            written_article_count: outcome.written_article_count,
            source_attempt_count: outcome.source_attempt_count,
            error: None,
        },
        Err(failure) => {
            let failure = *failure;
            LiveIndexWorkerResponse {
                worker_id: request.worker_id,
                status: "failed".to_string(),
                stats: failure.partial.stats.into(),
                written_article_count: failure.partial.written_article_count,
                source_attempt_count: failure.partial.source_attempt_count,
                error: Some(failure.error.to_string()),
            }
        }
    };
    Ok(response)
}

fn open_live_index_connection(db_path: &Path) -> Result<Connection, LiveIndexError> {
    Ok(open_index_db(db_path)?)
}

fn write_live_worker_request_file(
    request: &LiveIndexWorkerRequest,
) -> Result<PathBuf, LiveIndexError> {
    let request_path = live_worker_request_path(request.worker_id);
    let payload = serde_json::to_vec(request)?;
    if let Err(error) = fs::write(&request_path, payload) {
        let _ = fs::remove_file(&request_path);
        return Err(error.into());
    }
    Ok(request_path)
}

fn live_worker_request_path(worker_id: usize) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "litradar-live-worker-{}-{nanos}-{worker_id}.json",
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

fn journal_failure(error: impl Into<LiveIndexError>) -> Box<LiveJournalFailure> {
    Box::new(LiveJournalFailure {
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
            written_article_count: 0,
            source_attempt_count: 0,
        });
    }

    let scholarly_config = context
        .config
        .scholarly_config
        .clone()
        .with_worker_context(context.worker_id, context.process_count);
    let mut scholarly_client = match LiveScholarlyTransport::new(scholarly_config.clone()) {
        Ok(transport) => {
            ScholarlyClient::new(transport, scholarly_config.has_semantic_scholar_key())
        }
        Err(error) => return Err(finish_live_rows_failure(stats, 0, 0, error.into())),
    };
    let mut cnki_client = match LiveCnkiTransport::new(LiveCnkiConfig {
        timeout_seconds: context.config.timeout_seconds,
    }) {
        Ok(transport) => CnkiClient::new(transport),
        Err(error) => return Err(finish_live_rows_failure(stats, 0, 0, error.into())),
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
    let mut written_article_count = 0;
    let mut source_attempt_count = 0;
    let change_event_context = context.config.update.then(|| {
        ChangeEventContext::new(
            context.run_id,
            format!("worker-{}", context.worker_id),
            context.timestamp,
            false,
        )
    });

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
                    written_article_count,
                    source_attempt_count,
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
        let result = {
            let mut attempt_sink = |attempts: &[SourceAttempt]| {
                source_attempt_count += attempts.len();
                stats.record_source_attempts_for_source(
                    &source,
                    attempts,
                    Some(journal_id),
                    &journal_title,
                );
            };
            process_live_journal_row(
                &mut scholarly_client,
                &mut cnki_client,
                LiveJournalContext {
                    connection,
                    row,
                    csv_file: context.csv_file,
                    journal_id,
                    timestamp: context.timestamp,
                    cnki_config: &cnki_config,
                    change_event_context: change_event_context.as_ref(),
                    lease_context: context.lease_context.as_ref(),
                },
                &mut attempt_sink,
            )
        };
        match result {
            Ok(outcome) => {
                stats.record_path_counts(&path_key, outcome.counts);
                stats.finish_path(
                    &path_key,
                    &outcome.status,
                    context.timestamp.to_string(),
                    None,
                );
                written_article_count += outcome.counts.articles_written_count;
            }
            Err(failure) => {
                let LiveJournalFailure { error } = *failure;
                stats.finish_path(
                    &path_key,
                    "failed",
                    context.timestamp.to_string(),
                    Some(&error.to_string()),
                );
                return Err(finish_live_rows_failure(
                    stats,
                    written_article_count,
                    source_attempt_count,
                    error,
                ));
            }
        }
    }

    stats.finish("succeeded", context.timestamp.to_string(), None);
    Ok(live_rows_outcome(
        stats,
        written_article_count,
        source_attempt_count,
    ))
}

fn finish_live_rows_failure(
    mut stats: IndexRunStats,
    written_article_count: i64,
    source_attempt_count: usize,
    error: LiveIndexError,
) -> Box<LiveJournalRowsFailure> {
    let finished_at = stats.started_at.clone();
    stats.finish("failed", finished_at, Some(error.to_string()));
    Box::new(LiveJournalRowsFailure {
        partial: live_rows_outcome(stats, written_article_count, source_attempt_count),
        error,
    })
}

fn live_rows_outcome(
    stats: IndexRunStats,
    written_article_count: i64,
    source_attempt_count: usize,
) -> LiveJournalRowsOutcome {
    LiveJournalRowsOutcome {
        stats,
        written_article_count,
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
            return Err(finish_live_rows_failure(stats, 0, 0, error));
        }
    };

    let mut stats = IndexRunStats::new(
        context.run_id.to_string(),
        context.csv_file.to_string(),
        context.timestamp.to_string(),
    );
    let mut written_article_count = 0;
    let mut source_attempt_count = 0;
    let mut errors = Vec::new();
    for response in responses {
        stats.merge_worker_stats(response.stats.into_index_run_stats());
        written_article_count += response.written_article_count;
        source_attempt_count += response.source_attempt_count;
        if response.status != "succeeded" {
            errors.push(
                response
                    .error
                    .unwrap_or_else(|| format!("worker {} failed", response.worker_id)),
            );
        }
    }
    if errors.is_empty() {
        stats.finish("succeeded", context.timestamp.to_string(), None);
        Ok(LiveJournalRowsOutcome {
            stats,
            written_article_count,
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
                written_article_count,
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
                lease_run_id: context
                    .lease_context
                    .as_ref()
                    .map(|lease_context| lease_context.run_id().to_string()),
                worker_id,
                process_count: context.config.process_count,
                worker_count: context.config.worker_count,
                issue_batch_size: context.config.issue_batch_size,
                timeout_seconds: context.config.timeout_seconds,
                resume: context.config.resume,
                update: context.config.update,
                scholarly_config: context.config.scholarly_config.clone(),
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
    attempt_sink: &mut dyn FnMut(&[SourceAttempt]),
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
        change_event_context,
        lease_context,
    } = context;

    match source_from_row(row).as_str() {
        SCHOLARLY_SOURCE => {
            let result = process_scholarly_row(
                connection,
                scholarly_client,
                row,
                ScholarlyProcessContext {
                    csv_file,
                    journal_id,
                    timestamp,
                    change_event_context,
                    lease_context,
                    should_resume: cnki_config.resume && !cnki_config.update,
                    attempt_sink,
                },
            );
            match result {
                Ok(outcome) => Ok(LiveJournalOutcome {
                    status: outcome.status,
                    counts: PathCountIncrements {
                        works_count: outcome.works_count,
                        issues_count: outcome.issues_count,
                        articles_written_count: outcome.written_article_count,
                        articles_deleted_no_authors_count: outcome.deleted_article_count,
                        ..PathCountIncrements::default()
                    },
                }),
                Err(error) => Err(journal_failure(error)),
            }
        }
        CNKI_SOURCE => {
            let result = process_cnki_row(
                connection,
                cnki_client,
                row,
                CnkiProcessContext {
                    csv_file,
                    journal_id,
                    config: cnki_config,
                    change_event_context,
                    lease_context,
                    attempt_sink,
                },
            );
            match result {
                Ok(outcome) => Ok(LiveJournalOutcome {
                    status: outcome.status,
                    counts: PathCountIncrements {
                        issues_count: outcome.issues_count,
                        article_summaries_count: outcome.article_summaries_count,
                        article_details_count: outcome.article_details_count,
                        articles_written_count: outcome.written_article_count,
                        articles_deleted_no_authors_count: outcome.deleted_article_count,
                        ..PathCountIncrements::default()
                    },
                }),
                Err(error) => Err(journal_failure(error)),
            }
        }
        other => Err(journal_failure(LiveIndexError::UnsupportedSource(format!(
            "Unsupported source for {}: {other}",
            journal_title_from_row(row)
        )))),
    }
}

fn run_live_csv_index(
    config: &LiveIndexConfig,
    csv_path: &Path,
    db_path: &Path,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let launcher = ProcessLiveWorkerLauncher::new(config.application_executable.clone());
    run_live_csv_index_with_runtime(
        config,
        csv_path,
        db_path,
        &launcher,
        LiveRunTime::now(),
        Duration::from_secs(LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS),
    )
}

fn run_live_csv_index_with_runtime(
    config: &LiveIndexConfig,
    csv_path: &Path,
    db_path: &Path,
    launcher: &dyn LiveWorkerLauncher,
    run_time: LiveRunTime,
    heartbeat_interval: Duration,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let rows = read_csv_rows(csv_path)?;
    if rows.is_empty() {
        return Ok(LiveCsvIndexOutcome {
            csv_path: csv_path.display().to_string(),
            db_path: db_path.display().to_string(),
            run_id: String::new(),
            status: "skipped".to_string(),
            journal_count: 0,
            written_article_count: 0,
            source_attempt_count: 0,
            manifest_path: None,
            notify_exit_code: None,
        });
    }
    validate_sources(&rows)?;
    validate_unique_journal_identities(&rows)?;
    validate_required_source_config(&rows, &config.scholarly_config)?;
    if config.notify && !config.update {
        return Err(LiveIndexError::Notify(
            "--notify requires an update manifest".to_string(),
        ));
    }
    let timestamp = run_time.timestamp();
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
    let stem = csv_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("index");
    let run_id = run_time.run_id(stem);
    let expected_journal_count = i64::try_from(rows.len()).unwrap_or(i64::MAX);

    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = open_live_index_connection(db_path)?;
    begin_index_run(
        &connection,
        &IndexRunStartRequest {
            run_id: &run_id,
            csv_file: &csv_file,
            started_at: &timestamp,
            total_journals: expected_journal_count,
            now_epoch_seconds: run_time.epoch_seconds,
            should_adopt_events: config.update,
        },
    )?;
    let lease_context = IndexRunLeaseContext::new(&run_id);
    lease_context.refresh_after_acquisition(&connection)?;
    let mut heartbeat =
        match LiveIndexLeaseHeartbeat::start(db_path, lease_context.clone(), heartbeat_interval) {
            Ok(heartbeat) => heartbeat,
            Err(error) => {
                let stats = IndexRunStats::new(run_id.clone(), csv_file.clone(), timestamp.clone());
                return Err(finalize_failed_live_run(
                    &connection,
                    &lease_context,
                    None,
                    stats,
                    expected_journal_count,
                    &timestamp,
                    error,
                ));
            }
        };
    if let Err(error) = preflight_live_meta_catalog(&connection, &lease_context, &rows, &csv_file) {
        let stats = IndexRunStats::new(run_id.clone(), csv_file.clone(), timestamp.clone());
        return Err(finalize_failed_live_run(
            &connection,
            &lease_context,
            Some(&mut heartbeat),
            stats,
            expected_journal_count,
            &timestamp,
            error,
        ));
    }
    let journal_context = LiveJournalRowsContext {
        rows: &rows,
        csv_path,
        db_path,
        csv_file: &csv_file,
        run_id: &run_id,
        timestamp: &timestamp,
        lease_context: Some(lease_context.clone()),
        worker_id: 0,
        process_count: 1,
        config,
    };
    let journal_rows_outcome = if config.process_count > 1 && rows.len() > 1 {
        run_live_journal_rows_in_worker_processes(&journal_context, launcher)
    } else {
        run_live_journal_rows_locally(&connection, &journal_context)
    };
    let journal_rows_outcome = match journal_rows_outcome {
        Ok(outcome) => outcome,
        Err(failure) => {
            let failure = *failure;
            return Err(finalize_failed_live_run(
                &connection,
                &lease_context,
                Some(&mut heartbeat),
                failure.partial.stats,
                expected_journal_count,
                &timestamp,
                failure.error,
            ));
        }
    };
    if let Err(error) = heartbeat.check().and_then(|_| {
        lease_context
            .assert_owner(&connection)
            .map_err(LiveIndexError::from)
    }) {
        return Err(finalize_failed_live_run(
            &connection,
            &lease_context,
            Some(&mut heartbeat),
            journal_rows_outcome.stats,
            expected_journal_count,
            &timestamp,
            error,
        ));
    }

    let manifest_path = config.update.then(|| {
        config
            .project_root
            .join("data")
            .join("push_state")
            .join(format!(
                "{}.changes.json",
                db_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("index")
            ))
    });
    heartbeat.stop_and_check()?;
    let mut final_stats = journal_rows_outcome.stats;
    final_stats.total_journals = expected_journal_count;
    let manifest_publication = manifest_path
        .as_deref()
        .map(|path| LiveManifestPublication {
            db_name: &db_name,
            run_id: &run_id,
            generated_at: &timestamp,
            path,
        });
    if let Err(error) = persist_owned_live_run(
        &connection,
        &lease_context,
        &final_stats,
        Some(&timestamp),
        manifest_publication,
    ) {
        return Err(finalize_failed_live_run(
            &connection,
            &lease_context,
            Some(&mut heartbeat),
            final_stats,
            expected_journal_count,
            &timestamp,
            error,
        ));
    }
    let notify_exit_code = if config.notify {
        Some(run_notify_for_manifest(
            config,
            &db_name,
            manifest_path
                .as_deref()
                .expect("validated notify configuration should have a manifest"),
        )?)
    } else {
        None
    };

    Ok(LiveCsvIndexOutcome {
        csv_path: csv_path.display().to_string(),
        db_path: db_path.display().to_string(),
        run_id,
        status: "succeeded".to_string(),
        journal_count: rows.len(),
        written_article_count: journal_rows_outcome.written_article_count,
        source_attempt_count: journal_rows_outcome.source_attempt_count,
        manifest_path: manifest_path.map(|path| path.display().to_string()),
        notify_exit_code,
    })
}

struct LiveManifestPublication<'a> {
    db_name: &'a str,
    run_id: &'a str,
    generated_at: &'a str,
    path: &'a Path,
}

fn persist_owned_live_run(
    connection: &Connection,
    lease_context: &IndexRunLeaseContext,
    stats: &IndexRunStats,
    publication_timestamp: Option<&str>,
    manifest: Option<LiveManifestPublication<'_>>,
) -> Result<(), LiveIndexError> {
    with_immediate_index_transaction(connection, |transaction| {
        lease_context.assert_owner(transaction)?;
        if let Some(timestamp) = publication_timestamp {
            mark_article_listing_ready(transaction, timestamp)?;
            optimize_index_db(transaction)?;
        }
        if let Some(manifest) = manifest {
            write_change_manifest_from_events(
                transaction,
                manifest.db_name,
                manifest.run_id,
                manifest.generated_at,
                manifest.path,
            )?;
        }
        persist_index_run_stats(transaction, stats)?;
        lease_context.release(transaction)?;
        Ok::<(), LiveIndexError>(())
    })
}

#[allow(clippy::too_many_arguments)]
fn finalize_failed_live_run(
    connection: &Connection,
    lease_context: &IndexRunLeaseContext,
    heartbeat: Option<&mut LiveIndexLeaseHeartbeat>,
    mut stats: IndexRunStats,
    expected_journal_count: i64,
    timestamp: &str,
    error: LiveIndexError,
) -> LiveIndexError {
    if let Some(heartbeat) = heartbeat {
        if let Err(heartbeat_error) = heartbeat.stop_and_check() {
            return heartbeat_error;
        }
    }
    if matches!(&error, LiveIndexError::Lease(_)) {
        return error;
    }
    stats.total_journals = expected_journal_count;
    stats.finish("failed", timestamp.to_string(), Some(error.to_string()));
    match persist_owned_live_run(connection, lease_context, &stats, None, None) {
        Ok(()) => error,
        Err(finalize_error) => finalize_error,
    }
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

fn validate_unique_journal_identities(rows: &[CsvRow]) -> Result<(), LiveIndexError> {
    let mut identities = BTreeMap::new();
    for row in rows {
        let title = journal_title_from_row(row);
        let identity = journal_identity_from_row(row).ok_or_else(|| {
            LiveIndexError::InvalidConfig(format!(
                "Journal metadata row has no id, ISSN, or title: {title}"
            ))
        })?;
        let journal_id = build_journal_id(row).ok_or_else(|| {
            LiveIndexError::InvalidConfig(format!(
                "Journal metadata row has no stable identity: {title}"
            ))
        })?;
        if let Some((existing_title, existing_identity)) =
            identities.insert(journal_id, (title.clone(), identity.clone()))
        {
            return Err(LiveIndexError::InvalidConfig(format!(
                "Duplicate journal identity {journal_id} for {existing_title} ({existing_identity}) and {title} ({identity})"
            )));
        }
    }
    Ok(())
}

fn preflight_live_meta_catalog(
    connection: &Connection,
    lease_context: &IndexRunLeaseContext,
    rows: &[CsvRow],
    csv_file: &str,
) -> Result<(), LiveIndexError> {
    with_immediate_index_transaction(connection, |transaction| {
        lease_context.assert_owner(transaction)?;
        for row in rows {
            let journal_id = build_journal_id(row).ok_or_else(|| {
                LiveIndexError::InvalidConfig(format!(
                    "Journal metadata row has no stable identity: {}",
                    journal_title_from_row(row)
                ))
            })?;
            let journal = build_neutral_catalog_journal(journal_id, row);
            let metadata = build_meta_record(journal_id, csv_file, row);
            sync_journal_catalog_entry(transaction, &journal, &metadata)?;
        }
        for row in rows {
            let journal_id = build_journal_id(row).ok_or_else(|| {
                LiveIndexError::InvalidConfig(format!(
                    "Journal metadata row has no stable identity: {}",
                    journal_title_from_row(row)
                ))
            })?;
            let metadata = build_meta_record(journal_id, csv_file, row);
            if !journal_catalog_entry_is_current(transaction, &metadata)? {
                return Err(LiveIndexError::InvalidConfig(format!(
                    "Journal metadata preflight verification failed for {}",
                    journal_title_from_row(row)
                )));
            }
        }
        Ok::<(), LiveIndexError>(())
    })
}

fn build_neutral_catalog_journal(journal_id: i64, row: &CsvRow) -> JournalRecord {
    JournalRecord {
        journal_id,
        library_id: source_from_row(row),
        platform_journal_id: optional_csv_value(row, "id"),
        title: optional_csv_value(row, "title"),
        issn: optional_csv_value(row, "issn"),
        eissn: None,
        scimago_rank: None,
        cover_url: None,
        available: None,
        toc_data_approved_and_live: None,
        has_articles: None,
    }
}

fn journal_identity_from_row(row: &CsvRow) -> Option<String> {
    ["id", "issn", "title"]
        .into_iter()
        .find_map(|key| optional_csv_value(row, key))
}

fn optional_csv_value(row: &CsvRow, key: &str) -> Option<String> {
    row.get(key).and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
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
    run_notify_command_for_manifest(
        &config.application_executable,
        config,
        db_name,
        manifest_path,
    )
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
        .arg("notify")
        .arg("--secret-key-file")
        .arg(&config.secret_key_file)
        .arg("--db")
        .arg(db_name)
        .arg("--changes-file")
        .arg(manifest_path)
        .arg("--state-dir")
        .arg(&state_dir)
        .arg("--project-root")
        .arg(&config.project_root);
    if config.notify_dry_run {
        command.arg("--dry-run");
    }
    let status = command
        .status()
        .map_err(|error| LiveIndexError::Notify(error.to_string()))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use litradar_sources::LiveScholarlyConfig;
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{
        csv_paths, live_worker_command, open_live_index_connection, parse_csv_line, read_csv_rows,
        run_live_csv_index_with_runtime, run_live_index, run_live_index_heartbeat,
        run_live_journal_rows_in_worker_processes, run_notify_command_for_manifest,
        validate_required_source_config, validate_sources, validate_unique_journal_identities,
        LiveIndexConfig, LiveIndexError, LiveIndexHeartbeatState, LiveIndexWorkerRequest,
        LiveIndexWorkerResponse, LiveIndexWorkerStats, LiveJournalRowsContext, LiveRunTime,
        LiveWorkerLauncher, ProcessLiveWorkerLauncher,
    };
    use crate::schema::{
        begin_index_run, init_index_db, journal_catalog_entry_is_current, IndexRunLeaseContext,
        IndexRunStartRequest,
    };
    use crate::stats::IndexRunStats;
    use crate::transforms::{
        build_journal_id, build_meta_record, journal_title_from_row, source_from_row, CsvRow,
    };

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
    fn journal_identity_validation_names_both_conflicting_rows() {
        let rows = vec![
            worker_row("same-id", "First Journal"),
            worker_row("same-id", "Second Journal"),
        ];

        let error = validate_unique_journal_identities(&rows)
            .expect_err("duplicate stable identities should fail");

        assert!(matches!(
            error,
            LiveIndexError::InvalidConfig(message)
                if message.contains("First Journal")
                    && message.contains("Second Journal")
                    && message.contains("same-id")
        ));
    }

    #[test]
    fn duplicate_identity_rejects_before_database_or_worker_side_effects() {
        let root = tempdir().expect("temp root should be created");
        let csv_path = root.path().join("data").join("meta").join("journals.csv");
        let db_path = root
            .path()
            .join("data")
            .join("index")
            .join("journals.sqlite");
        fs::create_dir_all(csv_path.parent().expect("CSV should have a parent"))
            .expect("metadata directory should be created");
        fs::write(
            &csv_path,
            "source,id,title,issn\nscholarly,same-id,First Journal,1234-5678\nscholarly,same-id,Second Journal,2345-6789\n",
        )
        .expect("duplicate CSV should be written");
        let launcher = RecordingWorkerLauncher::default();

        let error = run_live_csv_index_with_runtime(
            &live_config(root.path()),
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect_err("duplicate identities should reject the live run");

        assert!(matches!(error, LiveIndexError::InvalidConfig(_)));
        assert!(!db_path.exists());
        assert!(launcher.requests.borrow().is_empty());
    }

    #[test]
    fn metadata_preflight_is_current_before_workers_launch() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        let launcher = RecordingWorkerLauncher {
            should_observe_catalog: true,
            ..RecordingWorkerLauncher::default()
        };

        run_live_csv_index_with_runtime(
            &live_config(root.path()),
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect("preflighted run should succeed");

        assert_eq!(*launcher.observed_catalog_is_current.borrow(), Some(true));
        let connection = open_test_index(&db_path);
        assert_eq!(table_count(&connection, "journals"), 2);
        assert_eq!(table_count(&connection, "journal_meta"), 2);
        let provider_flag_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM journals WHERE available IS NOT NULL OR toc_data_approved_and_live IS NOT NULL OR has_articles IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("provider flags should query");
        assert_eq!(provider_flag_count, 0);
    }

    #[test]
    fn metadata_preflight_verification_failure_rolls_back_and_launches_no_worker() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        let connection = open_test_index(&db_path);
        connection
            .execute_batch(
                "
                CREATE TRIGGER discard_preflight_metadata
                AFTER INSERT ON journal_meta
                BEGIN
                    DELETE FROM journal_meta WHERE journal_id = NEW.journal_id;
                END;
                ",
            )
            .expect("preflight verification failpoint should install");
        drop(connection);
        let launcher = RecordingWorkerLauncher::default();

        let error = run_live_csv_index_with_runtime(
            &live_config(root.path()),
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect_err("discarded metadata should fail preflight verification");

        let connection = open_test_index(&db_path);
        let parent_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = ?1",
                [test_run_time().run_id("journals")],
                |row| row.get(0),
            )
            .expect("failed parent should load");
        assert!(matches!(
            error,
            LiveIndexError::InvalidConfig(message)
                if message.contains("preflight verification") && message.contains("Journal A")
        ));
        assert!(launcher.requests.borrow().is_empty());
        assert_eq!(parent_status, "failed");
        assert_eq!(table_count(&connection, "journals"), 0);
        assert_eq!(table_count(&connection, "journal_meta"), 0);
        assert_eq!(table_count(&connection, "index_run_lease"), 0);
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
        assert_eq!(outcome.csvs[0].written_article_count, 0);
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
    fn live_update_adopts_publishes_and_cleans_pending_events() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        insert_pending_event(&db_path, "run-pending", 101);
        let mut config = live_config(root.path());
        config.update = true;
        let launcher = RecordingWorkerLauncher::default();

        let outcome = run_live_csv_index_with_runtime(
            &config,
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect("update should publish adopted events");

        let expected_run_id = test_run_time().run_id("journals");
        let connection = Connection::open(&db_path).expect("index database should open");
        let parent_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = ?1",
                [&expected_run_id],
                |row| row.get(0),
            )
            .expect("successful parent should load");
        let event_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("event count should load");
        let lease_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM index_run_lease", [], |row| row.get(0))
            .expect("lease count should load");
        let manifest_path = root
            .path()
            .join("data")
            .join("push_state")
            .join("journals.changes.json");
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&manifest_path).expect("manifest should read"),
        )
        .expect("manifest should parse");

        assert_eq!(outcome.run_id, expected_run_id);
        assert_eq!(parent_status, "succeeded");
        assert_eq!(event_count, 0);
        assert_eq!(lease_count, 0);
        assert_eq!(manifest["run_id"], outcome.run_id);
        assert_eq!(manifest["notifiable_article_ids"], serde_json::json!([101]));
        assert!(launcher
            .requests
            .borrow()
            .iter()
            .all(|request| request.lease_run_id.as_deref() == Some(outcome.run_id.as_str())));
    }

    #[test]
    fn live_update_refreshes_lease_after_acquisition_exceeds_expiry() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        insert_pending_event(&db_path, "run-pending", 202);
        let mut config = live_config(root.path());
        config.update = true;
        let launcher = RecordingWorkerLauncher::default();
        let expired_acquisition_time = LiveRunTime {
            epoch_seconds: 0,
            epoch_nanoseconds: 1,
        };

        let outcome = run_live_csv_index_with_runtime(
            &config,
            &csv_path,
            &db_path,
            &launcher,
            expired_acquisition_time,
            Duration::from_secs(60),
        )
        .expect("exact owner should refresh after long event adoption");

        let connection = Connection::open(&db_path).expect("index database should open");
        let parent_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = ?1",
                [&outcome.run_id],
                |row| row.get(0),
            )
            .expect("refreshed parent should load");
        assert_eq!(parent_status, "succeeded");
        assert_eq!(table_count(&connection, "index_change_events"), 0);
        assert_eq!(table_count(&connection, "index_run_lease"), 0);
    }

    #[test]
    fn stale_parent_is_interrupted_before_workers_and_non_update_preserves_events() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        let connection = open_test_index(&db_path);
        begin_index_run(
            &connection,
            &IndexRunStartRequest {
                run_id: "run-stale",
                csv_file: "journals.csv",
                started_at: "3999999700",
                total_journals: 2,
                now_epoch_seconds: test_run_time().epoch_seconds - 300,
                should_adopt_events: false,
            },
        )
        .expect("stale run should start");
        drop(connection);
        insert_pending_event(&db_path, "run-stale", 202);
        let launcher = RecordingWorkerLauncher {
            should_observe_parent: true,
            ..RecordingWorkerLauncher::default()
        };
        let config = live_config(root.path());

        run_live_csv_index_with_runtime(
            &config,
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect("stale run should be recovered");

        let expected_run_id = test_run_time().run_id("journals");
        assert_eq!(
            launcher.observed_parent.borrow().as_ref(),
            Some(&(expected_run_id.clone(), "running".to_string(), 2))
        );
        let connection = Connection::open(&db_path).expect("index database should open");
        let stale_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = 'run-stale'",
                [],
                |row| row.get(0),
            )
            .expect("stale parent should load");
        let pending_run_id: String = connection
            .query_row("SELECT run_id FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("pending event should remain");
        assert_eq!(stale_status, "interrupted");
        assert_eq!(pending_run_id, "run-stale");
    }

    #[test]
    fn active_lease_rejects_before_worker_launch() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        let connection = open_test_index(&db_path);
        begin_index_run(
            &connection,
            &IndexRunStartRequest {
                run_id: "run-active",
                csv_file: "journals.csv",
                started_at: "4000000000",
                total_journals: 2,
                now_epoch_seconds: test_run_time().epoch_seconds,
                should_adopt_events: false,
            },
        )
        .expect("active run should start");
        drop(connection);
        let launcher = RecordingWorkerLauncher::default();
        let later_time = LiveRunTime {
            epoch_seconds: test_run_time().epoch_seconds + 1,
            epoch_nanoseconds: test_run_time().epoch_nanoseconds + 1,
        };

        let error = run_live_csv_index_with_runtime(
            &live_config(root.path()),
            &csv_path,
            &db_path,
            &launcher,
            later_time,
            Duration::from_secs(60),
        )
        .expect_err("active lease should reject another run");

        assert!(matches!(error, LiveIndexError::Lease(_)));
        assert!(launcher.requests.borrow().is_empty());
    }

    #[test]
    fn worker_failure_persists_failed_parent_and_retains_adopted_events() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        insert_pending_event(&db_path, "run-pending", 303);
        let mut config = live_config(root.path());
        config.update = true;
        let launcher = RecordingWorkerLauncher {
            failed_worker_id: Some(1),
            ..RecordingWorkerLauncher::default()
        };

        let error = run_live_csv_index_with_runtime(
            &config,
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect_err("worker failure should fail the live run");

        let expected_run_id = test_run_time().run_id("journals");
        let connection = Connection::open(&db_path).expect("index database should open");
        let parent_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = ?1",
                [&expected_run_id],
                |row| row.get(0),
            )
            .expect("failed parent should load");
        let event_run_id: String = connection
            .query_row("SELECT run_id FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("adopted event should remain");
        assert!(matches!(error, LiveIndexError::Worker(_)));
        assert_eq!(parent_status, "failed");
        assert_eq!(event_run_id, expected_run_id);
        assert_eq!(table_count(&connection, "index_run_lease"), 0);
    }

    #[test]
    fn manifest_failure_persists_failed_parent_and_retains_adopted_events() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        insert_pending_event(&db_path, "run-pending", 304);
        fs::write(
            root.path().join("data").join("push_state"),
            "not a directory",
        )
        .expect("manifest parent blocker should be written");
        let mut config = live_config(root.path());
        config.update = true;
        let launcher = RecordingWorkerLauncher::default();

        let error = run_live_csv_index_with_runtime(
            &config,
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_secs(60),
        )
        .expect_err("manifest failure should fail the live run");

        let expected_run_id = test_run_time().run_id("journals");
        let connection = Connection::open(&db_path).expect("index database should open");
        let parent_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = ?1",
                [&expected_run_id],
                |row| row.get(0),
            )
            .expect("failed parent should load");
        let event_run_id: String = connection
            .query_row("SELECT run_id FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("adopted event should remain");
        assert!(matches!(error, LiveIndexError::Io(_)));
        assert_eq!(parent_status, "failed");
        assert_eq!(event_run_id, expected_run_id);
        assert_eq!(table_count(&connection, "index_run_lease"), 0);
        assert_eq!(table_count(&connection, "listing_state"), 0);
    }

    #[test]
    fn heartbeat_ownership_loss_prevents_success_publication() {
        let root = tempdir().expect("temp root should be created");
        let (csv_path, db_path) = write_parallel_live_csv(root.path());
        insert_pending_event(&db_path, "run-pending", 404);
        let mut config = live_config(root.path());
        config.update = true;
        let launcher = RecordingWorkerLauncher {
            should_steal_lease: true,
            ..RecordingWorkerLauncher::default()
        };

        let error = run_live_csv_index_with_runtime(
            &config,
            &csv_path,
            &db_path,
            &launcher,
            test_run_time(),
            Duration::from_millis(1),
        )
        .expect_err("ownership loss should prevent publication");

        let expected_run_id = test_run_time().run_id("journals");
        let connection = Connection::open(&db_path).expect("index database should open");
        let current_status: String = connection
            .query_row(
                "SELECT status FROM index_runs WHERE run_id = ?1",
                [&expected_run_id],
                |row| row.get(0),
            )
            .expect("orphaned parent should remain visible");
        let lease_run_id: String = connection
            .query_row(
                "SELECT run_id FROM index_run_lease WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("takeover lease should load");
        assert!(matches!(error, LiveIndexError::Lease(_)));
        assert_eq!(current_status, "running");
        assert_eq!(lease_run_id, "run-takeover");
        assert!(!root
            .path()
            .join("data/push_state/journals.changes.json")
            .exists());
    }

    #[test]
    fn heartbeat_retries_transient_database_lock_until_writer_releases() {
        let root = tempdir().expect("temp root should be created");
        let db_path = root.path().join("journals.sqlite");
        let connection = open_test_index(&db_path);
        let acquired_at = LiveRunTime::now().epoch_seconds - 1;
        let started_at = acquired_at.to_string();
        begin_index_run(
            &connection,
            &IndexRunStartRequest {
                run_id: "run-heartbeat-lock",
                csv_file: "journals.csv",
                started_at: &started_at,
                total_journals: 1,
                now_epoch_seconds: acquired_at,
                should_adopt_events: false,
            },
        )
        .expect("heartbeat fixture run should start");
        let heartbeat_connection =
            open_live_index_connection(&db_path).expect("heartbeat connection should open");
        heartbeat_connection
            .busy_timeout(Duration::from_millis(1))
            .expect("heartbeat busy timeout should be configurable");
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .expect("writer lock should be acquired");
        let state = Arc::new((
            Mutex::new(LiveIndexHeartbeatState::default()),
            Condvar::new(),
        ));
        let thread_state = Arc::clone(&state);
        let handle = thread::spawn(move || {
            run_live_index_heartbeat(
                heartbeat_connection,
                IndexRunLeaseContext::new("run-heartbeat-lock"),
                Duration::from_millis(1),
                thread_state,
            );
        });

        thread::sleep(Duration::from_millis(100));
        let error_while_locked = state
            .0
            .lock()
            .expect("heartbeat state should lock")
            .error
            .clone();
        connection
            .execute_batch("ROLLBACK")
            .expect("writer lock should be released");
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut heartbeat_at = acquired_at;
        while heartbeat_at <= acquired_at && Instant::now() < deadline {
            heartbeat_at = connection
                .query_row(
                    "SELECT heartbeat_at FROM index_run_lease WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .expect("lease heartbeat should load");
            thread::sleep(Duration::from_millis(5));
        }
        {
            let mut current_state = state.0.lock().expect("heartbeat state should lock");
            current_state.should_stop = true;
            state.1.notify_all();
        }
        handle.join().expect("heartbeat thread should stop");

        assert_eq!(error_while_locked, None);
        assert!(heartbeat_at > acquired_at);
    }

    #[test]
    fn run_id_uses_nanoseconds_within_the_same_second() {
        let first = LiveRunTime {
            epoch_seconds: 10,
            epoch_nanoseconds: 10_000_000_001,
        };
        let second = LiveRunTime {
            epoch_seconds: 10,
            epoch_nanoseconds: 10_000_000_002,
        };

        assert_eq!(first.timestamp(), "10");
        assert_ne!(first.run_id("journals"), second.run_id("journals"));
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
        assert_eq!(outcome.written_article_count, 3);

        let requests = launcher.requests.borrow();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            row_titles(&requests[0].rows),
            vec!["Journal A", "Journal C"]
        );
        assert_eq!(row_titles(&requests[1].rows), vec!["Journal B"]);
    }

    #[test]
    fn worker_response_size_does_not_scale_with_written_article_count() {
        let mut stats = IndexRunStats::new(
            "run-bounded".to_string(),
            "journals.csv".to_string(),
            "2026-07-13T00:00:00Z".to_string(),
        );
        stats.finish("succeeded", "2026-07-13T00:00:01Z".to_string(), None);
        let response = LiveIndexWorkerResponse {
            worker_id: 0,
            status: "succeeded".to_string(),
            stats: LiveIndexWorkerStats::from(stats),
            written_article_count: 10_000_000,
            source_attempt_count: 1,
            error: None,
        };

        let payload = serde_json::to_vec(&response).expect("worker response should serialize");

        assert!(payload.len() < 1_000);
        assert!(!String::from_utf8(payload)
            .expect("worker response should be UTF-8")
            .contains("written_article_ids"));
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
    fn worker_spawn_failure_removes_request_file() {
        let root = tempdir().expect("temp root should be created");
        let rows = vec![worker_row("a", "Journal A")];
        let mut config = live_config(root.path());
        config.process_count = 1;
        let csv_path = root.path().join("journals.csv");
        let db_path = root.path().join("journals.sqlite");
        let context = worker_rows_context(&csv_path, &db_path, &config, &rows);
        let launcher = ProcessLiveWorkerLauncher::new(root.path().join("missing-worker"));
        let before = live_worker_request_paths();

        let failure = run_live_journal_rows_in_worker_processes(&context, &launcher)
            .expect_err("missing worker executable should fail");

        assert!(matches!(failure.error, LiveIndexError::Worker(_)));
        assert_eq!(live_worker_request_paths(), before);
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
                semantic_scholar_worker_id: 0,
                semantic_scholar_process_count: 1,
                semantic_scholar_base_interval_ms: 1_000,
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
                semantic_scholar_worker_id: 0,
                semantic_scholar_process_count: 1,
                semantic_scholar_base_interval_ms: 1_000,
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
                semantic_scholar_worker_id: 0,
                semantic_scholar_process_count: 1,
                semantic_scholar_base_interval_ms: 1_000,
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
        assert!(args.trim_start().starts_with("notify "));
        assert!(args.contains("--db"));
        assert!(args.contains("fixture.sqlite"));
        assert!(args.contains("--changes-file"));
        assert!(args.contains("fixture.changes.json"));
        assert!(args.contains("--state-dir"));
        assert!(args.contains("push_state"));
        assert!(args.contains("--project-root"));
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

    #[test]
    fn live_worker_command_uses_the_same_application_and_index_subcommand() {
        let command = live_worker_command(
            Path::new("/app/litradar"),
            Path::new("requests/worker-1.json"),
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), "/app/litradar");
        assert_eq!(
            arguments,
            ["index", "--live-worker-request", "requests/worker-1.json"]
        );
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
        should_observe_parent: bool,
        observed_parent: RefCell<Option<(String, String, i64)>>,
        should_observe_catalog: bool,
        observed_catalog_is_current: RefCell<Option<bool>>,
        should_steal_lease: bool,
    }

    impl LiveWorkerLauncher for RecordingWorkerLauncher {
        fn run_workers(
            &self,
            requests: Vec<LiveIndexWorkerRequest>,
        ) -> Result<Vec<LiveIndexWorkerResponse>, LiveIndexError> {
            self.requests.replace(requests.clone());
            if self.should_observe_parent {
                let request = requests
                    .first()
                    .expect("observed worker requests should not be empty");
                let connection = open_test_index(&request.db_path);
                let parent = connection
                    .query_row(
                        "SELECT run_id, status, total_journals FROM index_runs WHERE run_id = ?1",
                        [request.run_id.as_str()],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .expect("running parent should be visible before workers launch");
                self.observed_parent.replace(Some(parent));
            }
            if self.should_observe_catalog {
                let request = requests
                    .first()
                    .expect("observed worker requests should not be empty");
                let connection = open_test_index(&request.db_path);
                let is_current = requests.iter().flat_map(|item| &item.rows).all(|row| {
                    let journal_id = build_journal_id(row)
                        .expect("worker metadata row should have a stable identity");
                    let metadata = build_meta_record(journal_id, &request.csv_file, row);
                    journal_catalog_entry_is_current(&connection, &metadata)
                        .expect("worker-visible metadata should verify")
                });
                self.observed_catalog_is_current.replace(Some(is_current));
            }
            if self.should_steal_lease {
                let request = requests
                    .first()
                    .expect("lease-stealing worker requests should not be empty");
                let connection = open_test_index(&request.db_path);
                connection
                    .execute(
                        "
                        INSERT INTO index_runs (
                            run_id, csv_file, started_at, finished_at, status,
                            total_journals, succeeded_journals, failed_journals,
                            resumed_journals, error_summary
                        ) VALUES (
                            'run-takeover', 'journals.csv', '4000000001', NULL,
                            'running', 0, 0, 0, 0, NULL
                        )
                        ",
                        [],
                    )
                    .expect("takeover parent should insert");
                connection
                    .execute(
                        "
                        UPDATE index_run_lease
                        SET run_id = 'run-takeover', heartbeat_at = 4000000001,
                            expires_at = 4000000301
                        WHERE id = 1
                        ",
                        [],
                    )
                    .expect("lease should move to takeover run");
                thread::sleep(Duration::from_millis(20));
            }
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
        for row in &request.rows {
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
        }
        stats.finish(status, request.timestamp.clone(), error.clone());
        LiveIndexWorkerResponse {
            worker_id: request.worker_id,
            status: status.to_string(),
            stats: LiveIndexWorkerStats::from(stats),
            written_article_count: request.rows.len() as i64,
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
            lease_context: None,
            worker_id: 0,
            process_count: config.process_count,
            config,
        }
    }

    fn test_run_time() -> LiveRunTime {
        LiveRunTime {
            epoch_seconds: 4_000_000_000,
            epoch_nanoseconds: 4_000_000_000_000_000_123,
        }
    }

    fn write_parallel_live_csv(root: &Path) -> (PathBuf, PathBuf) {
        let csv_path = root.join("data").join("meta").join("journals.csv");
        let db_path = root.join("data").join("index").join("journals.sqlite");
        fs::create_dir_all(csv_path.parent().expect("CSV should have a parent"))
            .expect("metadata directory should be created");
        fs::create_dir_all(db_path.parent().expect("database should have a parent"))
            .expect("index directory should be created");
        fs::write(
            &csv_path,
            "source,id,title,issn\nscholarly,a,Journal A,1234-5678\nscholarly,b,Journal B,2345-6789\n",
        )
        .expect("parallel live CSV should be written");
        (csv_path, db_path)
    }

    fn open_test_index(db_path: &Path) -> Connection {
        let connection = Connection::open(db_path).expect("index database should open");
        init_index_db(&connection).expect("index schema should initialize");
        connection
    }

    fn insert_pending_event(db_path: &Path, run_id: &str, article_id: i64) {
        let connection = open_test_index(db_path);
        connection
            .execute(
                "
                INSERT INTO index_change_events (
                    run_id, worker_id, article_id, event_type, membership_type,
                    journal_id, issue_id, is_backfill, created_at
                ) VALUES (?1, 'worker-old', ?2, 'add', 'inpress', 1, NULL, 0, '3999999000')
                ",
                rusqlite::params![run_id, article_id],
            )
            .expect("pending event should insert");
    }

    fn table_count(connection: &Connection, table_name: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
                row.get(0)
            })
            .expect("table count should load")
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

    fn live_worker_request_paths() -> Vec<PathBuf> {
        let prefix = format!("litradar-live-worker-{}-", std::process::id());
        let mut paths = fs::read_dir(std::env::temp_dir())
            .expect("temporary directory should be readable")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .filter(|name| name.starts_with(&prefix) && name.ends_with(".json"))
                    .map(|_| entry.path())
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
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
            application_executable: root.join("litradar"),
            project_root: root.to_path_buf(),
            secret_key_file: root.join("secret.key"),
            file: None,
            worker_count: 32,
            process_count: 2,
            issue_batch_size: 10,
            timeout_seconds: 1,
            resume: false,
            update: false,
            notify: false,
            notify_dry_run: true,
            scholarly_config: LiveScholarlyConfig::from_value_pools(
                1,
                "openalex",
                "semantic",
                "crossref@example.test",
            ),
        }
    }
}
