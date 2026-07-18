//! Provider-neutral live catalog indexing orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_domain::{JournalCatalogEntry, ProviderBatch};
use litradar_provider::{
    IndexContentProvider, ProviderError, ProviderRegistration, ProviderRegistryError,
};
use litradar_sources::{
    cnki_index_registration, scholarly_index_registration, LiveCnkiConfig, LiveCnkiTransport,
    LiveScholarlyConfig, LiveScholarlyTransport, CNKI_PROVIDER_NAME,
    OPENALEX_MAX_WORKERS_PER_PROCESS, SCHOLARLY_PROVIDER_NAME,
};
use rusqlite::{Connection, ErrorCode};
use serde::{Deserialize, Serialize};

use crate::changes::{
    discard_content_change_events, write_content_change_manifest, ChangeWriteError,
};
use crate::control::{
    acquire_lease, commit_content_then_checkpoint, heartbeat_lease, open_control_db,
    read_checkpoint, release_lease, CheckpointScope, ContentCheckpointCommitError,
    ControlDatabaseError,
};
use crate::schema::{
    open_content_db, optimize_content_db, write_content_batch, ContentDatabaseError,
};
use crate::stats::IndexRunMetrics;
use crate::transforms::{read_catalog_csv, CatalogContractError};
use crate::worker_protocol::{
    read_message, write_message, ParentMessage, ProtocolError,
    WorkerFailure as LiveIndexWorkerFailure, WorkerFailureClass as LiveIndexWorkerFailureClass,
    WorkerJournalAssignment, WorkerMessage, WorkerOperation as LiveIndexWorkerOperation,
    WorkerRequest as LiveIndexWorkerRequest, PROTOCOL_VERSION,
};

const LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS: u64 = 30;
const MAX_PROVIDER_PAGES_PER_JOURNAL: usize = 100_000;
const SCHOLARLY_MAX_PROCESS_COUNT: usize = 3;
const WORKER_PROTOCOL_FAILURE_MESSAGE: &str = "worker protocol operation failed";

/// Live index run configuration.
#[derive(Debug, Clone)]
pub struct LiveIndexConfig {
    /// Canonical application executable used for worker and notification subprocesses.
    pub application_executable: PathBuf,
    /// Project root containing the `data` directory.
    pub project_root: PathBuf,
    /// Deployment secret key file forwarded to notification handoff.
    pub secret_key_file: PathBuf,
    /// Optional canonical CSV filename under `data/meta`.
    pub file: Option<String>,
    /// Number of bounded source workers, including OpenAlex DOI enrichment requests.
    pub worker_count: usize,
    /// Number of journal worker processes.
    pub process_count: usize,
    /// Number of issues reserved for one provider-side detail batch.
    pub issue_batch_size: usize,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// Whether a completed provider-scoped journal checkpoint may be skipped.
    pub resume: bool,
    /// Whether to rescan content and publish a change manifest.
    pub update: bool,
    /// Whether to run `notify` after an update manifest is written.
    pub notify: bool,
    /// Whether notify handoff should use dry-run mode.
    pub notify_dry_run: bool,
    /// Scholarly source runtime configuration.
    pub scholarly_config: LiveScholarlyConfig,
    /// Validated catalog-stem to indexing-provider routes loaded outside index databases.
    pub index_provider_routes: BTreeMap<String, String>,
}

/// Live index command outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveIndexOutcome {
    /// Final run status.
    pub status: String,
    /// Human-readable message for skipped work.
    pub message: Option<String>,
    /// Per-catalog outcomes.
    pub csvs: Vec<LiveCsvIndexOutcome>,
}

/// Live index outcome for one maintained catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveCsvIndexOutcome {
    /// Canonical catalog CSV path.
    pub csv_path: String,
    /// Stable catalog-derived content database path.
    pub db_path: String,
    /// Core-owned run identifier.
    pub run_id: String,
    /// Final run status.
    pub status: String,
    /// Indexed journal count.
    pub journal_count: usize,
    /// New or changed canonical article count.
    pub written_article_count: i64,
    /// Canonical provider page count.
    pub source_attempt_count: usize,
    /// Optional provider-neutral update manifest path.
    pub manifest_path: Option<String>,
    /// Optional notify process exit code.
    pub notify_exit_code: Option<i32>,
}

/// Live index workflow failure.
#[derive(Debug)]
pub enum LiveIndexError {
    /// Filesystem operation failed.
    Io(std::io::Error),
    /// Worker request or response JSON failed.
    Json(serde_json::Error),
    /// Canonical catalog parsing or validation failed.
    Catalog(CatalogContractError),
    /// Opening a specific content database failed.
    ContentDatabase {
        /// Exact content database path requiring operator attention.
        path: PathBuf,
        /// Provider-neutral schema or write failure.
        source: ContentDatabaseError,
    },
    /// A common content/checkpoint commit failed.
    Commit(ContentCheckpointCommitError),
    /// Disposable control database or lease operation failed.
    Control(ControlDatabaseError),
    /// Provider registration failed.
    Registry(ProviderRegistryError),
    /// A provider could not be constructed from current runtime configuration.
    ProviderSetup(String),
    /// A canonical provider operation failed.
    Provider(ProviderError),
    /// Runtime configuration is invalid or incomplete.
    InvalidConfig(String),
    /// A journal worker process failed or returned invalid output.
    Worker(String),
    /// Notification handoff failed.
    Notify(String),
    /// The parent heartbeat could not preserve lease ownership.
    Heartbeat(String),
}

impl fmt::Display for LiveIndexError {
    /// Format a safe live index diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Catalog(error) => write!(formatter, "{error}"),
            Self::ContentDatabase { path, source } => write!(
                formatter,
                "index database {} cannot be used: {source}",
                path.display()
            ),
            Self::Commit(error) => write!(formatter, "{error}"),
            Self::Control(error) => write!(formatter, "{error}"),
            Self::Registry(error) => write!(formatter, "{error}"),
            Self::ProviderSetup(message)
            | Self::InvalidConfig(message)
            | Self::Worker(message)
            | Self::Notify(message)
            | Self::Heartbeat(message) => formatter.write_str(message),
            Self::Provider(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for LiveIndexError {
    /// Return the underlying typed failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Catalog(error) => Some(error),
            Self::ContentDatabase { source, .. } => Some(source),
            Self::Commit(error) => Some(error),
            Self::Control(error) => Some(error),
            Self::Registry(error) => Some(error),
            Self::Provider(error) => Some(error),
            Self::ProviderSetup(_)
            | Self::InvalidConfig(_)
            | Self::Worker(_)
            | Self::Notify(_)
            | Self::Heartbeat(_) => None,
        }
    }
}

impl From<std::io::Error> for LiveIndexError {
    /// Convert filesystem failures.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for LiveIndexError {
    /// Convert worker JSON failures.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<CatalogContractError> for LiveIndexError {
    /// Convert catalog contract failures.
    fn from(error: CatalogContractError) -> Self {
        Self::Catalog(error)
    }
}

impl From<ContentCheckpointCommitError> for LiveIndexError {
    /// Convert ordered content/checkpoint failures.
    fn from(error: ContentCheckpointCommitError) -> Self {
        Self::Commit(error)
    }
}

impl From<ControlDatabaseError> for LiveIndexError {
    /// Convert disposable control failures.
    fn from(error: ControlDatabaseError) -> Self {
        Self::Control(error)
    }
}

impl From<ProviderRegistryError> for LiveIndexError {
    /// Convert provider registration failures.
    fn from(error: ProviderRegistryError) -> Self {
        Self::Registry(error)
    }
}

impl From<ProviderError> for LiveIndexError {
    /// Convert safe provider failures.
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl From<ChangeWriteError> for LiveIndexError {
    /// Convert provider-neutral manifest failures.
    fn from(error: ChangeWriteError) -> Self {
        match error {
            ChangeWriteError::Io(error) => Self::Io(error),
            ChangeWriteError::Json(error) => Self::Json(error),
            ChangeWriteError::Sqlite(error) => {
                Self::Worker(format!("content outbox operation failed: {error}"))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum StoredCheckpoint {
    Provider { value: String },
    Complete,
}

#[derive(Debug, Clone)]
struct DirectIndexRequest {
    catalog_name: String,
    provider_name: String,
    run_id: String,
    timestamp: String,
    worker_id: usize,
    resume: bool,
    update: bool,
    entries: Vec<JournalCatalogEntry>,
}

/// Parent-owned context required to commit worker batches.
#[derive(Debug, Clone)]
pub(crate) struct ParentWriterContext {
    /// Stable maintained catalog stem.
    pub(crate) catalog_name: String,
    /// Stable registered indexing provider.
    pub(crate) provider_name: String,
    /// Core-owned run identifier.
    pub(crate) run_id: String,
    /// Safe content and checkpoint timestamp.
    pub(crate) timestamp: String,
}

/// One safe parent writer observation emitted after a durable acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriterCommitObservation {
    /// Worker whose batch committed.
    pub(crate) worker_id: usize,
    /// Monotonic worker sequence that committed.
    pub(crate) sequence: u64,
    /// Provider page index that committed.
    pub(crate) page_index: usize,
    /// Milliseconds from parent receipt through acknowledgement flush.
    pub(crate) service_ms: u64,
    /// Canonical articles observed in the committed batch.
    pub(crate) articles_seen: usize,
}

impl LiveIndexWorkerFailure {
    /// Classify one typed worker error without retaining its free-form message.
    fn from_error(error: &LiveIndexError) -> Self {
        match error {
            LiveIndexError::Io(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Io,
                LiveIndexWorkerOperation::FileSystem,
            ),
            LiveIndexError::Json(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Json,
                LiveIndexWorkerOperation::WorkerJson,
            ),
            LiveIndexError::Catalog(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Catalog,
                LiveIndexWorkerOperation::CatalogRead,
            ),
            LiveIndexError::ContentDatabase { source, .. } => {
                Self::from_content(LiveIndexWorkerOperation::ContentDatabaseOpen, source)
            }
            LiveIndexError::Commit(ContentCheckpointCommitError::Content(source)) => {
                Self::from_content(LiveIndexWorkerOperation::ContentCommit, source)
            }
            LiveIndexError::Commit(ContentCheckpointCommitError::Control(source)) => {
                Self::from_control(LiveIndexWorkerOperation::CheckpointCommit, source)
            }
            LiveIndexError::Control(source) => {
                Self::from_control(LiveIndexWorkerOperation::ControlDatabase, source)
            }
            LiveIndexError::Registry(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Registry,
                LiveIndexWorkerOperation::ProviderRegistry,
            ),
            LiveIndexError::ProviderSetup(_) => Self::fixed(
                LiveIndexWorkerFailureClass::ProviderSetup,
                LiveIndexWorkerOperation::ProviderSetup,
            ),
            LiveIndexError::Provider(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Provider,
                LiveIndexWorkerOperation::ProviderRequest,
            ),
            LiveIndexError::InvalidConfig(_) => Self::fixed(
                LiveIndexWorkerFailureClass::InvalidConfig,
                LiveIndexWorkerOperation::Configuration,
            ),
            LiveIndexError::Worker(message) => Self::fixed(
                LiveIndexWorkerFailureClass::Worker,
                if message == WORKER_PROTOCOL_FAILURE_MESSAGE {
                    LiveIndexWorkerOperation::WorkerProtocol
                } else {
                    LiveIndexWorkerOperation::WorkerProcess
                },
            ),
            LiveIndexError::Notify(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Notify,
                LiveIndexWorkerOperation::Notification,
            ),
            LiveIndexError::Heartbeat(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Heartbeat,
                LiveIndexWorkerOperation::Heartbeat,
            ),
        }
    }

    /// Classify one content-domain failure at a fixed operation boundary.
    fn from_content(operation: LiveIndexWorkerOperation, error: &ContentDatabaseError) -> Self {
        match error {
            ContentDatabaseError::Sqlite(error) => Self::from_sqlite(operation, error),
            ContentDatabaseError::Json(_)
            | ContentDatabaseError::Contract(_)
            | ContentDatabaseError::Identity(_)
            | ContentDatabaseError::Merge(_)
            | ContentDatabaseError::RebuildRequired { .. }
            | ContentDatabaseError::InvalidCurrentSchema(_)
            | ContentDatabaseError::ArticleIdCollision { .. } => {
                Self::fixed(LiveIndexWorkerFailureClass::Content, operation)
            }
        }
    }

    /// Classify one control-domain failure at a fixed operation boundary.
    fn from_control(operation: LiveIndexWorkerOperation, error: &ControlDatabaseError) -> Self {
        match error {
            ControlDatabaseError::Sqlite(error) => Self::from_sqlite(operation, error),
            ControlDatabaseError::Io(_) => Self::fixed(LiveIndexWorkerFailureClass::Io, operation),
            ControlDatabaseError::UnsupportedVersion { .. }
            | ControlDatabaseError::ActiveLease { .. }
            | ControlDatabaseError::OwnershipLost { .. } => {
                Self::fixed(LiveIndexWorkerFailureClass::Control, operation)
            }
        }
    }

    /// Retain only typed SQLite codes from one rusqlite failure.
    fn from_sqlite(operation: LiveIndexWorkerOperation, error: &rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(failure, _) => Self {
                class: LiveIndexWorkerFailureClass::Sqlite,
                operation,
                sqlite_code: Some(format!("{:?}", failure.code)),
                sqlite_extended_code: Some(failure.extended_code),
                is_busy_or_locked: matches!(
                    failure.code,
                    ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                ),
            },
            _ => Self::fixed(LiveIndexWorkerFailureClass::Sqlite, operation),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LiveRunTime {
    epoch_seconds: i64,
    epoch_milliseconds: u64,
    epoch_nanoseconds: u128,
}

impl LiveRunTime {
    fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            epoch_seconds: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
            epoch_milliseconds: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            epoch_nanoseconds: duration.as_nanos(),
        }
    }

    fn timestamp(self) -> String {
        self.epoch_seconds.to_string()
    }

    fn run_id(self, catalog_name: &str) -> String {
        format!("{catalog_name}-{}", self.epoch_nanoseconds)
    }
}

struct LeaseHeartbeat {
    stop: Sender<()>,
    handle: Option<JoinHandle<Result<(), String>>>,
}

impl LeaseHeartbeat {
    fn start(
        control_path: PathBuf,
        catalog_name: String,
        provider_name: String,
        run_id: String,
        interval: Duration,
    ) -> Self {
        let (stop, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let connection = open_control_db(control_path).map_err(|error| error.to_string())?;
            loop {
                match receiver.recv_timeout(interval) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                    Err(RecvTimeoutError::Timeout) => {
                        heartbeat_lease(
                            &connection,
                            &catalog_name,
                            &provider_name,
                            &run_id,
                            LiveRunTime::now().epoch_seconds,
                        )
                        .map_err(|error| error.to_string())?;
                    }
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop_and_check(&mut self) -> Result<(), LiveIndexError> {
        let _ = self.stop.send(());
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        handle
            .join()
            .map_err(|_| LiveIndexError::Heartbeat("index heartbeat thread panicked".to_string()))?
            .map_err(LiveIndexError::Heartbeat)
    }
}

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Run live indexing for selected provider-free maintained catalogs.
///
/// # Arguments
///
/// * `config` - Runtime paths, provider routes, concurrency, and source configuration.
///
/// # Returns
///
/// Per-catalog provider-neutral index outcomes.
pub fn run_live_index(config: &LiveIndexConfig) -> Result<LiveIndexOutcome, LiveIndexError> {
    validate_live_config(config)?;
    let meta_dir = config.project_root.join("data").join("meta");
    if !meta_dir.exists() {
        return Err(LiveIndexError::InvalidConfig(format!(
            "managed catalog directory does not exist: {}",
            meta_dir.display()
        )));
    }
    let paths = catalog_paths(&meta_dir, config.file.as_deref())?;
    if paths.is_empty() {
        return Ok(LiveIndexOutcome {
            status: "skipped".to_string(),
            message: Some("no canonical catalog CSV files were selected".to_string()),
            csvs: Vec::new(),
        });
    }
    let mut outcomes = Vec::with_capacity(paths.len());
    for path in paths {
        outcomes.push(run_catalog(config, &path)?);
    }
    Ok(LiveIndexOutcome {
        status: "succeeded".to_string(),
        message: None,
        csvs: outcomes,
    })
}

/// Run one serialized fetch-worker request over the process standard streams.
///
/// # Arguments
///
/// * `request_path` - Disposable JSON request path created by the parent process.
///
/// # Returns
///
/// Success after a terminal protocol message is flushed.
pub fn run_live_index_worker_from_file_path(
    request_path: impl AsRef<Path>,
) -> Result<(), LiveIndexError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_live_index_worker_with_io(request_path.as_ref(), stdin.lock(), stdout.lock())
}

fn run_live_index_worker_with_io(
    request_path: &Path,
    mut reader: impl Read,
    mut writer: impl Write,
) -> Result<(), LiveIndexError> {
    let request: LiveIndexWorkerRequest =
        serde_json::from_str(&std::fs::read_to_string(request_path)?)?;
    run_fetch_worker_stream(&request, &mut reader, &mut writer)
}

fn validate_live_config(config: &LiveIndexConfig) -> Result<(), LiveIndexError> {
    if config.worker_count == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "worker_count must be greater than zero".to_string(),
        ));
    }
    if config.worker_count > OPENALEX_MAX_WORKERS_PER_PROCESS
        && config
            .index_provider_routes
            .values()
            .any(|provider| provider == SCHOLARLY_PROVIDER_NAME)
    {
        return Err(LiveIndexError::InvalidConfig(format!(
            "worker_count must be at most {OPENALEX_MAX_WORKERS_PER_PROCESS} for scholarly indexing"
        )));
    }
    if config.process_count == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "process_count must be greater than zero".to_string(),
        ));
    }
    if config.process_count > SCHOLARLY_MAX_PROCESS_COUNT
        && config
            .index_provider_routes
            .values()
            .any(|provider| provider == SCHOLARLY_PROVIDER_NAME)
    {
        return Err(LiveIndexError::InvalidConfig(format!(
            "process_count must be at most {SCHOLARLY_MAX_PROCESS_COUNT} for scholarly indexing"
        )));
    }
    if config.issue_batch_size == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "issue_batch_size must be greater than zero".to_string(),
        ));
    }
    if config.timeout_seconds == 0 {
        return Err(LiveIndexError::InvalidConfig(
            "timeout_seconds must be greater than zero".to_string(),
        ));
    }
    if config.notify && !config.update {
        return Err(LiveIndexError::InvalidConfig(
            "--notify requires an update manifest".to_string(),
        ));
    }
    if config.index_provider_routes.is_empty() {
        return Err(LiveIndexError::InvalidConfig(
            "index_provider_routes must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn catalog_paths(meta_dir: &Path, file: Option<&str>) -> Result<Vec<PathBuf>, LiveIndexError> {
    if let Some(file) = file {
        let file_path = Path::new(file);
        if file_path.file_name().and_then(|value| value.to_str()) != Some(file)
            || file_path.extension().and_then(|value| value.to_str()) != Some("csv")
        {
            return Err(LiveIndexError::InvalidConfig(
                "--file must be one CSV filename without directory components".to_string(),
            ));
        }
        let path = meta_dir.join(file_path);
        return Ok(path.exists().then_some(path).into_iter().collect());
    }
    let mut paths = std::fs::read_dir(meta_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("csv"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn run_catalog(
    config: &LiveIndexConfig,
    csv_path: &Path,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let catalog_name = csv_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            LiveIndexError::InvalidConfig(format!(
                "catalog path has no UTF-8 stem: {}",
                csv_path.display()
            ))
        })?
        .to_string();
    let provider_name = config
        .index_provider_routes
        .get(&catalog_name)
        .ok_or_else(|| {
            LiveIndexError::InvalidConfig(format!(
                "index_provider_routes has no route for catalog {catalog_name}"
            ))
        })?
        .clone();
    let entries = read_catalog_csv(csv_path)?;
    let index_dir = config.project_root.join("data").join("index");
    let control_dir = config.project_root.join("data").join("index-control");
    std::fs::create_dir_all(&index_dir)?;
    std::fs::create_dir_all(&control_dir)?;
    let content_path = index_dir.join(format!("{catalog_name}.sqlite"));
    let control_path = control_dir.join(format!("{catalog_name}.sqlite"));
    let uses_worker_processes = config.process_count > 1 && entries.len() > 1;
    let control = open_control_db(&control_path)?;
    let run_time = LiveRunTime::now();
    let run_id = run_time.run_id(&catalog_name);
    let timestamp = run_time.timestamp();
    acquire_lease(
        &control,
        &catalog_name,
        &provider_name,
        &run_id,
        run_time.epoch_seconds,
    )?;
    let content = match open_content_db(&content_path) {
        Ok(content) => content,
        Err(source) => {
            let _ = release_lease(&control, &catalog_name, &provider_name, &run_id);
            return Err(LiveIndexError::ContentDatabase {
                path: content_path,
                source,
            });
        }
    };
    let writer_context = ParentWriterContext {
        catalog_name: catalog_name.clone(),
        provider_name: provider_name.clone(),
        run_id: run_id.clone(),
        timestamp: timestamp.clone(),
    };
    let (execution, heartbeat_result) = if uses_worker_processes {
        let prepared = prepare_worker_requests(
            config,
            &control,
            &writer_context,
            run_time.epoch_milliseconds,
            &entries,
        );
        let execution = prepared.and_then(|(requests, metrics)| {
            run_worker_processes(
                config,
                &content,
                &control,
                &writer_context,
                requests,
                metrics,
            )
        });
        (execution, Ok(()))
    } else {
        let mut heartbeat = LeaseHeartbeat::start(
            control_path.clone(),
            catalog_name.clone(),
            provider_name.clone(),
            run_id.clone(),
            Duration::from_secs(LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS),
        );
        let request = DirectIndexRequest {
            catalog_name: catalog_name.clone(),
            provider_name: provider_name.clone(),
            run_id: run_id.clone(),
            timestamp: timestamp.clone(),
            worker_id: 0,
            resume: config.resume,
            update: config.update,
            entries: entries.clone(),
        };
        let execution = run_direct_request(
            config,
            &content,
            &control,
            &request,
            run_time.epoch_milliseconds,
        );
        let heartbeat_result = heartbeat.stop_and_check();
        (execution, heartbeat_result)
    };

    let metrics = match execution {
        Ok(metrics) => metrics,
        Err(error) => {
            let _ = release_lease(&control, &catalog_name, &provider_name, &run_id);
            let mut failed = IndexRunMetrics {
                journals_total: entries.len(),
                journals_failed: entries.len(),
                ..IndexRunMetrics::default()
            };
            if let LiveIndexError::Worker(_) = error {
                failed.journals_failed = 1;
            }
            failed.emit_terminal(&run_id, &catalog_name, &provider_name, "all", "failure");
            return Err(error);
        }
    };
    if let Err(error) = heartbeat_result {
        let _ = release_lease(&control, &catalog_name, &provider_name, &run_id);
        return Err(error);
    }
    let finalization = (|| -> Result<(String, Option<PathBuf>), LiveIndexError> {
        optimize_content_db(&content).map_err(|source| LiveIndexError::ContentDatabase {
            path: content_path.clone(),
            source,
        })?;
        let db_name = content_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("index.sqlite")
            .to_string();
        let manifest_path = config.update.then(|| {
            config
                .project_root
                .join("data")
                .join("push_state")
                .join(format!("{catalog_name}.changes.json"))
        });
        if let Some(path) = manifest_path.as_deref() {
            write_content_change_manifest(&content, &db_name, &run_id, &timestamp, path)?;
        } else {
            discard_content_change_events(&content).map_err(|error| {
                LiveIndexError::Worker(format!("content outbox acknowledgement failed: {error}"))
            })?;
        }
        Ok((db_name, manifest_path))
    })();
    let release_result = release_lease(&control, &catalog_name, &provider_name, &run_id);
    let (db_name, manifest_path) = finalization?;
    release_result?;
    let notify_exit_code = if config.notify {
        Some(run_notify_for_manifest(
            config,
            &db_name,
            manifest_path
                .as_deref()
                .expect("validated notify run has a manifest path"),
        )?)
    } else {
        None
    };
    metrics.emit_terminal(&run_id, &catalog_name, &provider_name, "all", "success");
    Ok(LiveCsvIndexOutcome {
        csv_path: csv_path.display().to_string(),
        db_path: content_path.display().to_string(),
        run_id,
        status: "succeeded".to_string(),
        journal_count: entries.len(),
        written_article_count: i64::try_from(metrics.articles_changed).unwrap_or(i64::MAX),
        source_attempt_count: metrics.pages_committed,
        manifest_path: manifest_path.map(|path| path.display().to_string()),
        notify_exit_code,
    })
}

fn prepare_worker_requests(
    config: &LiveIndexConfig,
    control: &Connection,
    context: &ParentWriterContext,
    schedule_epoch_unix_millis: u64,
    entries: &[JournalCatalogEntry],
) -> Result<(Vec<LiveIndexWorkerRequest>, IndexRunMetrics), LiveIndexError> {
    let mut metrics = IndexRunMetrics {
        journals_total: entries.len(),
        ..IndexRunMetrics::default()
    };
    let mut assignments = Vec::with_capacity(entries.len());
    for (journal_ordinal, entry) in entries.iter().cloned().enumerate() {
        let stored = if config.resume && !config.update {
            let scope = CheckpointScope::Journal {
                catalog_id: entry.catalog_id.clone(),
            };
            read_checkpoint(
                control,
                &context.catalog_name,
                &context.provider_name,
                &scope,
            )?
            .map(|value| decode_checkpoint(&value))
            .transpose()?
        } else {
            None
        };
        match stored {
            Some(StoredCheckpoint::Complete) => metrics.journals_resumed += 1,
            Some(StoredCheckpoint::Provider { value }) => {
                assignments.push(WorkerJournalAssignment {
                    journal_ordinal,
                    entry,
                    initial_checkpoint: Some(value),
                });
            }
            None => assignments.push(WorkerJournalAssignment {
                journal_ordinal,
                entry,
                initial_checkpoint: None,
            }),
        }
    }
    if assignments.is_empty() {
        return Ok((Vec::new(), metrics));
    }
    let process_count = config.process_count.min(assignments.len()).max(1);
    let mut partitions = vec![Vec::new(); process_count];
    for (index, assignment) in assignments.into_iter().enumerate() {
        partitions[index % process_count].push(assignment);
    }
    let requests = partitions
        .into_iter()
        .enumerate()
        .map(|(worker_id, assignments)| LiveIndexWorkerRequest {
            protocol_version: PROTOCOL_VERSION,
            catalog_name: context.catalog_name.clone(),
            provider_name: context.provider_name.clone(),
            run_id: context.run_id.clone(),
            worker_id,
            process_count,
            source_worker_count: config.worker_count,
            schedule_epoch_unix_millis,
            timeout_seconds: config.timeout_seconds,
            scholarly_config: config.scholarly_config.clone(),
            assignments,
        })
        .collect();
    Ok((requests, metrics))
}

fn run_worker_processes(
    config: &LiveIndexConfig,
    content: &Connection,
    control: &Connection,
    context: &ParentWriterContext,
    requests: Vec<LiveIndexWorkerRequest>,
    metrics: IndexRunMetrics,
) -> Result<IndexRunMetrics, LiveIndexError> {
    let request_dir = config
        .project_root
        .join("data")
        .join("index-control")
        .join("worker-requests");
    run_worker_processes_with_launcher(
        &request_dir,
        content,
        control,
        context,
        requests,
        metrics,
        Duration::from_secs(LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS),
        |request_path, worker_id| {
            let child = Command::new(&config.application_executable)
                .arg("index")
                .arg("--live-worker-request")
                .arg(request_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|_| {
                    LiveIndexError::Worker(format!("worker process {worker_id} could not start"))
                })?;
            LaunchedWorkerProcess::from_child_stdio(child, worker_id)
        },
        |_| {},
    )
}

/// Emit one failed worker event using only fixed classifications and typed SQLite codes.
fn emit_worker_failure(worker_id: usize, failure: &LiveIndexWorkerFailure) {
    if let Some(sqlite_code) = failure.sqlite_code.as_deref() {
        tracing::error!(
            event = "index.worker.failed",
            component = "index",
            worker_id,
            failure_class = failure.class.as_str(),
            operation = failure.operation.as_str(),
            has_sqlite_code = true,
            sqlite_code,
            sqlite_extended_code = failure.sqlite_extended_code.unwrap_or_default(),
            is_busy_or_locked = failure.is_busy_or_locked,
        );
    } else {
        tracing::error!(
            event = "index.worker.failed",
            component = "index",
            worker_id,
            failure_class = failure.class.as_str(),
            operation = failure.operation.as_str(),
            has_sqlite_code = false,
            is_busy_or_locked = failure.is_busy_or_locked,
        );
    }
}

/// Build a generic parent failure from safe structured worker fields.
fn worker_failure_error(worker_id: usize, failure: &LiveIndexWorkerFailure) -> LiveIndexError {
    LiveIndexError::Worker(format!(
        "worker {worker_id} failed during {} ({})",
        failure.operation.as_str(),
        failure.class.as_str()
    ))
}

fn protocol_failure(worker_id: usize) -> LiveIndexError {
    let failure = LiveIndexWorkerFailure::fixed(
        LiveIndexWorkerFailureClass::Worker,
        LiveIndexWorkerOperation::WorkerProtocol,
    );
    emit_worker_failure(worker_id, &failure);
    worker_failure_error(worker_id, &failure)
}

fn process_failure(worker_id: usize) -> LiveIndexError {
    let failure = LiveIndexWorkerFailure::fixed(
        LiveIndexWorkerFailureClass::Worker,
        LiveIndexWorkerOperation::WorkerProcess,
    );
    emit_worker_failure(worker_id, &failure);
    worker_failure_error(worker_id, &failure)
}

enum WorkerReaderEvent {
    Message {
        worker_id: usize,
        message: Box<WorkerMessage>,
        received_at: Instant,
    },
    Ended {
        worker_id: usize,
    },
    Invalid {
        worker_id: usize,
    },
}

struct SpawnedWorker {
    worker_id: usize,
    request_path: PathBuf,
    child: Option<Child>,
    stdin: Option<BufWriter<Box<dyn Write + Send>>>,
    reader: Option<JoinHandle<()>>,
}

/// Process handle and bidirectional protocol streams returned by a worker launcher.
pub(crate) struct LaunchedWorkerProcess {
    child: Child,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

impl LaunchedWorkerProcess {
    /// Take the standard input and output pipes from a production worker process.
    ///
    /// # Arguments
    ///
    /// * `child` - Spawned worker with piped standard input and output.
    /// * `worker_id` - Stable worker identifier used for safe failure attribution.
    ///
    /// # Returns
    ///
    /// Process and protocol streams ready for supervision.
    pub(crate) fn from_child_stdio(
        mut child: Child,
        worker_id: usize,
    ) -> Result<Self, LiveIndexError> {
        let Some(writer) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(process_failure(worker_id));
        };
        let Some(reader) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(process_failure(worker_id));
        };
        Ok(Self {
            child,
            reader: Box::new(reader),
            writer: Box::new(writer),
        })
    }

    /// Build a process-real test worker with explicit protocol streams.
    ///
    /// # Arguments
    ///
    /// * `child` - Spawned fixture process.
    /// * `reader` - Child-to-parent protocol stream.
    /// * `writer` - Parent-to-child acknowledgement stream.
    ///
    /// # Returns
    ///
    /// Process and protocol streams ready for production supervision logic.
    #[cfg(test)]
    pub(crate) fn from_test_streams(
        child: Child,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Self {
        Self {
            child,
            reader: Box::new(reader),
            writer: Box::new(writer),
        }
    }
}

struct WorkerProgress {
    assignments: Vec<WorkerJournalAssignment>,
    assignment_position: usize,
    next_page_index: usize,
    next_sequence: u64,
    terminal_received: bool,
}

impl WorkerProgress {
    fn from_request(request: &LiveIndexWorkerRequest) -> Self {
        Self {
            assignments: request.assignments.clone(),
            assignment_position: 0,
            next_page_index: 0,
            next_sequence: 0,
            terminal_received: false,
        }
    }
}

/// Supervise fetch-only child processes through the parent-owned SQLite writer.
///
/// # Arguments
///
/// * `request_dir` - Disposable worker request directory.
/// * `content` - Parent-owned content connection.
/// * `control` - Parent-owned control connection.
/// * `context` - Stable commit and lease context.
/// * `requests` - Versioned worker assignments.
/// * `metrics` - Aggregate metrics prepared from parent checkpoint reads.
/// * `heartbeat_interval` - Lease renewal interval.
/// * `launcher` - Production or test-only child process launcher.
/// * `observer` - Safe post-ACK writer observation callback.
///
/// # Returns
///
/// Aggregate metrics after every worker stream and process completes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_worker_processes_with_launcher<Launcher, Observer>(
    request_dir: &Path,
    content: &Connection,
    control: &Connection,
    context: &ParentWriterContext,
    requests: Vec<LiveIndexWorkerRequest>,
    metrics: IndexRunMetrics,
    heartbeat_interval: Duration,
    mut launcher: Launcher,
    mut observer: Observer,
) -> Result<IndexRunMetrics, LiveIndexError>
where
    Launcher: FnMut(&Path, usize) -> Result<LaunchedWorkerProcess, LiveIndexError>,
    Observer: FnMut(WriterCommitObservation),
{
    if requests.is_empty() {
        return Ok(metrics);
    }
    let expected_process_count = requests.len();
    let mut journal_ordinals = BTreeSet::new();
    for (worker_id, request) in requests.iter().enumerate() {
        let has_invalid_assignment = request
            .assignments
            .iter()
            .any(|assignment| !journal_ordinals.insert(assignment.journal_ordinal));
        if request.protocol_version != PROTOCOL_VERSION
            || request.worker_id != worker_id
            || request.process_count != expected_process_count
            || request.catalog_name != context.catalog_name
            || request.provider_name != context.provider_name
            || request.run_id != context.run_id
            || request.assignments.is_empty()
            || has_invalid_assignment
        {
            return Err(protocol_failure(worker_id));
        }
    }
    std::fs::create_dir_all(request_dir)?;
    let (sender, receiver) = mpsc::sync_channel(requests.len());
    let mut children = Vec::with_capacity(requests.len());
    let mut spawn_error = None;
    for request in &requests {
        let request_path = request_dir.join(format!(
            "{}-worker-{}.json",
            request.run_id, request.worker_id
        ));
        let request_bytes = match serde_json::to_vec(request) {
            Ok(bytes) => bytes,
            Err(error) => {
                spawn_error = Some(LiveIndexError::Json(error));
                break;
            }
        };
        if let Err(error) = std::fs::write(&request_path, request_bytes) {
            let _ = std::fs::remove_file(&request_path);
            spawn_error = Some(LiveIndexError::Io(error));
            break;
        }
        let launched = match launcher(&request_path, request.worker_id) {
            Ok(launched) => launched,
            Err(error) => {
                let _ = std::fs::remove_file(&request_path);
                spawn_error = Some(error);
                break;
            }
        };
        children.push(attach_worker_process(
            launched,
            request.worker_id,
            request_path,
            sender.clone(),
        ));
    }
    drop(sender);
    if let Some(error) = spawn_error {
        drop(receiver);
        stop_worker_processes(&mut children);
        join_worker_readers(&mut children);
        return Err(error);
    }
    let mut progress = requests
        .iter()
        .map(WorkerProgress::from_request)
        .collect::<Vec<_>>();
    let execution = supervise_worker_processes(
        content,
        control,
        context,
        &mut children,
        &mut progress,
        metrics,
        heartbeat_interval,
        &receiver,
        &mut observer,
    );
    drop(receiver);
    if execution.is_err() {
        stop_worker_processes(&mut children);
    }
    join_worker_readers(&mut children);
    execution
}

fn attach_worker_process(
    launched: LaunchedWorkerProcess,
    worker_id: usize,
    request_path: PathBuf,
    sender: SyncSender<WorkerReaderEvent>,
) -> SpawnedWorker {
    let LaunchedWorkerProcess {
        child,
        reader: stdout,
        writer: stdin,
    } = launched;
    let reader = thread::spawn(move || {
        read_worker_messages(worker_id, BufReader::new(stdout), sender);
    });
    SpawnedWorker {
        worker_id,
        request_path,
        child: Some(child),
        stdin: Some(BufWriter::new(stdin)),
        reader: Some(reader),
    }
}

fn read_worker_messages(
    worker_id: usize,
    mut reader: impl Read,
    sender: SyncSender<WorkerReaderEvent>,
) {
    loop {
        match read_message(&mut reader) {
            Ok(message) => {
                if sender
                    .send(WorkerReaderEvent::Message {
                        worker_id,
                        message: Box::new(message),
                        received_at: Instant::now(),
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(ProtocolError::EndOfStream) => {
                let _ = sender.send(WorkerReaderEvent::Ended { worker_id });
                return;
            }
            Err(ProtocolError::Io(_) | ProtocolError::Json(_)) => {
                let _ = sender.send(WorkerReaderEvent::Invalid { worker_id });
                return;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn supervise_worker_processes(
    content: &Connection,
    control: &Connection,
    context: &ParentWriterContext,
    children: &mut [SpawnedWorker],
    progress: &mut [WorkerProgress],
    mut metrics: IndexRunMetrics,
    heartbeat_interval: Duration,
    receiver: &Receiver<WorkerReaderEvent>,
    observer: &mut impl FnMut(WriterCommitObservation),
) -> Result<IndexRunMetrics, LiveIndexError> {
    let mut remaining_workers = children.len();
    let mut next_heartbeat = Instant::now() + heartbeat_interval;
    while remaining_workers > 0 {
        if Instant::now() >= next_heartbeat {
            heartbeat_lease(
                control,
                &context.catalog_name,
                &context.provider_name,
                &context.run_id,
                LiveRunTime::now().epoch_seconds,
            )
            .map_err(|error| LiveIndexError::Heartbeat(error.to_string()))?;
            next_heartbeat = Instant::now() + heartbeat_interval;
        }
        let wait = next_heartbeat.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(wait) {
            Ok(WorkerReaderEvent::Message {
                worker_id,
                message,
                received_at,
            }) => {
                if let Some(observation) = handle_worker_message(
                    content,
                    control,
                    context,
                    children,
                    progress,
                    &mut metrics,
                    worker_id,
                    *message,
                    received_at,
                )? {
                    observer(observation);
                }
            }
            Ok(WorkerReaderEvent::Ended { worker_id }) => {
                let Some(worker_progress) = progress.get(worker_id) else {
                    return Err(protocol_failure(worker_id));
                };
                if !worker_progress.terminal_received {
                    return Err(protocol_failure(worker_id));
                }
                finish_worker_process(children, worker_id)?;
                remaining_workers -= 1;
            }
            Ok(WorkerReaderEvent::Invalid { worker_id }) => {
                return Err(protocol_failure(worker_id));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(protocol_failure(0));
            }
        }
    }
    Ok(metrics)
}

#[allow(clippy::too_many_arguments)]
fn handle_worker_message(
    content: &Connection,
    control: &Connection,
    context: &ParentWriterContext,
    children: &mut [SpawnedWorker],
    progress: &mut [WorkerProgress],
    metrics: &mut IndexRunMetrics,
    pipe_worker_id: usize,
    message: WorkerMessage,
    received_at: Instant,
) -> Result<Option<WriterCommitObservation>, LiveIndexError> {
    let Some(worker_progress) = progress.get_mut(pipe_worker_id) else {
        return Err(protocol_failure(pipe_worker_id));
    };
    if worker_progress.terminal_received {
        return Err(protocol_failure(pipe_worker_id));
    }
    match message {
        WorkerMessage::Batch {
            protocol_version,
            worker_id,
            sequence,
            journal_ordinal,
            page_index,
            batch,
        } => {
            if protocol_version != PROTOCOL_VERSION
                || worker_id != pipe_worker_id
                || sequence != worker_progress.next_sequence
                || page_index != worker_progress.next_page_index
                || page_index >= MAX_PROVIDER_PAGES_PER_JOURNAL
            {
                return Err(protocol_failure(pipe_worker_id));
            }
            let Some(assignment) = worker_progress
                .assignments
                .get(worker_progress.assignment_position)
                .cloned()
            else {
                return Err(protocol_failure(pipe_worker_id));
            };
            if journal_ordinal != assignment.journal_ordinal
                || batch.catalog_id != assignment.entry.catalog_id
            {
                return Err(protocol_failure(pipe_worker_id));
            }
            let stored_checkpoint = checkpoint_after_batch(&batch)?;
            let is_complete = matches!(stored_checkpoint, StoredCheckpoint::Complete);
            let encoded_checkpoint = serde_json::to_string(&stored_checkpoint)?;
            let scope = CheckpointScope::Journal {
                catalog_id: assignment.entry.catalog_id.clone(),
            };
            let content_revision = format!(
                "{}:{}:{}",
                context.run_id, assignment.entry.catalog_id, page_index
            );
            let outcome = commit_content_then_checkpoint(
                control,
                &context.catalog_name,
                &context.provider_name,
                &scope,
                &encoded_checkpoint,
                &context.timestamp,
                || {
                    write_content_batch(
                        content,
                        &assignment.entry,
                        &batch,
                        &content_revision,
                        &context.timestamp,
                    )
                },
            )
            .map_err(|error| {
                let error = LiveIndexError::Commit(error);
                emit_worker_failure(pipe_worker_id, &LiveIndexWorkerFailure::from_error(&error));
                error
            })?;
            metrics.record_write(outcome);
            let commit_service_ms = duration_millis(received_at.elapsed());
            tracing::debug!(
                event = "index.writer.batch_committed",
                component = "index",
                worker_id = pipe_worker_id,
                sequence,
                journal_ordinal,
                page_index,
                is_complete,
                service_ms = commit_service_ms,
                articles_seen = outcome.articles_seen,
                articles_changed = outcome.articles_changed,
                identity_aliases_added = outcome.identity_aliases_added,
                change_events_emitted = outcome.change_events_emitted,
            );
            let Some(worker) = children.get_mut(pipe_worker_id) else {
                return Err(protocol_failure(pipe_worker_id));
            };
            let Some(stdin) = worker.stdin.as_mut() else {
                return Err(protocol_failure(pipe_worker_id));
            };
            write_message(
                stdin,
                &ParentMessage::Committed {
                    protocol_version: PROTOCOL_VERSION,
                    worker_id: pipe_worker_id,
                    sequence,
                    journal_ordinal,
                    page_index,
                    is_complete,
                },
            )
            .map_err(|_| protocol_failure(pipe_worker_id))?;
            let observation = WriterCommitObservation {
                worker_id: pipe_worker_id,
                sequence,
                page_index,
                service_ms: duration_millis(received_at.elapsed()),
                articles_seen: outcome.articles_seen,
            };
            worker_progress.next_sequence = worker_progress
                .next_sequence
                .checked_add(1)
                .ok_or_else(|| protocol_failure(pipe_worker_id))?;
            if is_complete {
                worker_progress.assignment_position += 1;
                worker_progress.next_page_index = 0;
                metrics.journals_succeeded += 1;
            } else {
                worker_progress.next_page_index += 1;
            }
            return Ok(Some(observation));
        }
        WorkerMessage::Succeeded {
            protocol_version,
            worker_id,
            sequence,
        } => {
            if protocol_version != PROTOCOL_VERSION
                || worker_id != pipe_worker_id
                || sequence != worker_progress.next_sequence
                || worker_progress.assignment_position != worker_progress.assignments.len()
            {
                return Err(protocol_failure(pipe_worker_id));
            }
            worker_progress.terminal_received = true;
        }
        WorkerMessage::Failed {
            protocol_version,
            worker_id,
            sequence,
            failure,
        } => {
            if protocol_version != PROTOCOL_VERSION
                || worker_id != pipe_worker_id
                || sequence != worker_progress.next_sequence
            {
                return Err(protocol_failure(pipe_worker_id));
            }
            emit_worker_failure(pipe_worker_id, &failure);
            return Err(worker_failure_error(pipe_worker_id, &failure));
        }
    }
    Ok(None)
}

fn finish_worker_process(
    children: &mut [SpawnedWorker],
    worker_id: usize,
) -> Result<(), LiveIndexError> {
    let Some(worker) = children.get_mut(worker_id) else {
        return Err(protocol_failure(worker_id));
    };
    if worker.worker_id != worker_id {
        return Err(protocol_failure(worker_id));
    }
    worker.stdin = None;
    let Some(mut child) = worker.child.take() else {
        return Err(protocol_failure(worker_id));
    };
    let status = child.wait();
    let _ = std::fs::remove_file(&worker.request_path);
    let status = status.map_err(|_| process_failure(worker_id))?;
    if !status.success() {
        return Err(process_failure(worker_id));
    }
    Ok(())
}

fn stop_worker_processes(children: &mut [SpawnedWorker]) {
    for worker in children {
        worker.stdin = None;
        if let Some(mut child) = worker.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&worker.request_path);
    }
}

fn join_worker_readers(children: &mut [SpawnedWorker]) {
    for worker in children {
        if let Some(reader) = worker.reader.take() {
            let _ = reader.join();
        }
    }
}

fn run_direct_request(
    config: &LiveIndexConfig,
    content: &Connection,
    control: &Connection,
    request: &DirectIndexRequest,
    schedule_epoch_unix_millis: u64,
) -> Result<IndexRunMetrics, LiveIndexError> {
    let registration = build_index_registration(
        &request.provider_name,
        config
            .scholarly_config
            .clone()
            .with_worker_context(request.worker_id, 1)
            .with_schedule_epoch(schedule_epoch_unix_millis),
        config.worker_count,
        config.timeout_seconds,
    )?;
    let provider = registration.index_content().cloned().ok_or_else(|| {
        LiveIndexError::InvalidConfig(format!(
            "provider {} does not declare indexing capability",
            request.provider_name
        ))
    })?;
    index_entries_with_provider(content, control, provider.as_ref(), request)
}

fn run_fetch_worker_stream(
    request: &LiveIndexWorkerRequest,
    reader: &mut impl Read,
    writer: &mut impl Write,
) -> Result<(), LiveIndexError> {
    let mut sequence = 0_u64;
    let execution = fetch_worker_assignments(request, reader, writer, &mut sequence);
    let message = match execution {
        Ok(()) => WorkerMessage::Succeeded {
            protocol_version: PROTOCOL_VERSION,
            worker_id: request.worker_id,
            sequence,
        },
        Err(error) => WorkerMessage::Failed {
            protocol_version: PROTOCOL_VERSION,
            worker_id: request.worker_id,
            sequence,
            failure: LiveIndexWorkerFailure::from_error(&error),
        },
    };
    write_message(writer, &message)
        .map_err(|_| LiveIndexError::Worker(WORKER_PROTOCOL_FAILURE_MESSAGE.to_string()))
}

fn fetch_worker_assignments(
    request: &LiveIndexWorkerRequest,
    reader: &mut impl Read,
    writer: &mut impl Write,
    sequence: &mut u64,
) -> Result<(), LiveIndexError> {
    if request.protocol_version != PROTOCOL_VERSION
        || request.process_count == 0
        || request.worker_id >= request.process_count
    {
        return Err(LiveIndexError::InvalidConfig(
            "worker protocol request is invalid".to_string(),
        ));
    }
    let unique_ordinals = request
        .assignments
        .iter()
        .map(|assignment| assignment.journal_ordinal)
        .collect::<BTreeSet<_>>();
    if unique_ordinals.len() != request.assignments.len() {
        return Err(LiveIndexError::InvalidConfig(
            "worker journal assignments are invalid".to_string(),
        ));
    }
    let registration = build_index_registration(
        &request.provider_name,
        request
            .scholarly_config
            .clone()
            .with_worker_context(request.worker_id, request.process_count)
            .with_schedule_epoch(request.schedule_epoch_unix_millis),
        request.source_worker_count,
        request.timeout_seconds,
    )?;
    let provider = registration.index_content().cloned().ok_or_else(|| {
        LiveIndexError::InvalidConfig(format!(
            "provider {} does not declare indexing capability",
            request.provider_name
        ))
    })?;
    fetch_worker_assignments_with_provider(request, provider.as_ref(), reader, writer, sequence)
}

fn fetch_worker_assignments_with_provider(
    request: &LiveIndexWorkerRequest,
    provider: &dyn IndexContentProvider,
    reader: &mut impl Read,
    writer: &mut impl Write,
    sequence: &mut u64,
) -> Result<(), LiveIndexError> {
    for assignment in &request.assignments {
        let mut provider_checkpoint = assignment.initial_checkpoint.clone();
        let mut seen_checkpoints = BTreeSet::new();
        if let Some(value) = &provider_checkpoint {
            seen_checkpoints.insert(value.clone());
        }
        for page_index in 0..MAX_PROVIDER_PAGES_PER_JOURNAL {
            let batch = provider.fetch(&assignment.entry, provider_checkpoint.as_deref())?;
            if batch.catalog_id != assignment.entry.catalog_id {
                return Err(LiveIndexError::InvalidConfig(
                    "provider batch catalog identity is invalid".to_string(),
                ));
            }
            let stored_checkpoint = checkpoint_after_batch(&batch)?;
            let is_complete = matches!(stored_checkpoint, StoredCheckpoint::Complete);
            write_message(
                writer,
                &WorkerMessage::Batch {
                    protocol_version: PROTOCOL_VERSION,
                    worker_id: request.worker_id,
                    sequence: *sequence,
                    journal_ordinal: assignment.journal_ordinal,
                    page_index,
                    batch,
                },
            )
            .map_err(|_| LiveIndexError::Worker(WORKER_PROTOCOL_FAILURE_MESSAGE.to_string()))?;
            let acknowledgement: ParentMessage = read_message(reader)
                .map_err(|_| LiveIndexError::Worker(WORKER_PROTOCOL_FAILURE_MESSAGE.to_string()))?;
            match acknowledgement {
                ParentMessage::Committed {
                    protocol_version,
                    worker_id,
                    sequence: acknowledged_sequence,
                    journal_ordinal,
                    page_index: acknowledged_page_index,
                    is_complete: acknowledged_complete,
                } if protocol_version == PROTOCOL_VERSION
                    && worker_id == request.worker_id
                    && acknowledged_sequence == *sequence
                    && journal_ordinal == assignment.journal_ordinal
                    && acknowledged_page_index == page_index
                    && acknowledged_complete == is_complete => {}
                ParentMessage::Committed { .. } => {
                    return Err(LiveIndexError::Worker(
                        WORKER_PROTOCOL_FAILURE_MESSAGE.to_string(),
                    ));
                }
            }
            *sequence = sequence.checked_add(1).ok_or_else(|| {
                LiveIndexError::InvalidConfig("worker sequence limit exceeded".to_string())
            })?;
            if is_complete {
                break;
            }
            let StoredCheckpoint::Provider { value } = stored_checkpoint else {
                unreachable!("complete checkpoint returned above")
            };
            if !seen_checkpoints.insert(value.clone()) {
                return Err(LiveIndexError::InvalidConfig(
                    "index provider returned a repeated checkpoint".to_string(),
                ));
            }
            provider_checkpoint = Some(value);
            if page_index + 1 == MAX_PROVIDER_PAGES_PER_JOURNAL {
                return Err(LiveIndexError::InvalidConfig(
                    "provider page limit exceeded".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn build_index_registration(
    provider_name: &str,
    scholarly_config: LiveScholarlyConfig,
    source_worker_count: usize,
    timeout_seconds: u64,
) -> Result<ProviderRegistration, LiveIndexError> {
    match provider_name {
        SCHOLARLY_PROVIDER_NAME => {
            let has_semantic_scholar_key = scholarly_config.has_semantic_scholar_key();
            let transport = LiveScholarlyTransport::new_with_openalex_workers(
                scholarly_config,
                source_worker_count,
            )
            .map_err(|_| {
                LiveIndexError::ProviderSetup(
                    "scholarly indexing provider could not initialize".to_string(),
                )
            })?;
            Ok(scholarly_index_registration(
                transport,
                has_semantic_scholar_key,
            )?)
        }
        CNKI_PROVIDER_NAME => {
            let transport =
                LiveCnkiTransport::new(LiveCnkiConfig { timeout_seconds }).map_err(|_| {
                    LiveIndexError::ProviderSetup(
                        "CNKI indexing provider could not initialize".to_string(),
                    )
                })?;
            Ok(cnki_index_registration(transport)?)
        }
        name => Err(LiveIndexError::InvalidConfig(format!(
            "index provider {name} is not registered"
        ))),
    }
}

fn index_entries_with_provider(
    content: &Connection,
    control: &Connection,
    provider: &dyn IndexContentProvider,
    request: &DirectIndexRequest,
) -> Result<IndexRunMetrics, LiveIndexError> {
    let mut metrics = IndexRunMetrics {
        journals_total: request.entries.len(),
        ..IndexRunMetrics::default()
    };
    for entry in &request.entries {
        heartbeat_lease(
            control,
            &request.catalog_name,
            &request.provider_name,
            &request.run_id,
            LiveRunTime::now().epoch_seconds,
        )
        .map_err(|error| LiveIndexError::Heartbeat(error.to_string()))?;
        let scope = CheckpointScope::Journal {
            catalog_id: entry.catalog_id.clone(),
        };
        let stored = if request.resume && !request.update {
            read_checkpoint(
                control,
                &request.catalog_name,
                &request.provider_name,
                &scope,
            )?
            .map(|value| decode_checkpoint(&value))
            .transpose()?
        } else {
            None
        };
        if matches!(stored, Some(StoredCheckpoint::Complete)) {
            metrics.journals_resumed += 1;
            continue;
        }
        let mut provider_checkpoint = match stored {
            Some(StoredCheckpoint::Provider { value }) => Some(value),
            Some(StoredCheckpoint::Complete) | None => None,
        };
        let mut seen_checkpoints = BTreeSet::new();
        if let Some(value) = &provider_checkpoint {
            seen_checkpoints.insert(value.clone());
        }
        for page_index in 0..MAX_PROVIDER_PAGES_PER_JOURNAL {
            heartbeat_lease(
                control,
                &request.catalog_name,
                &request.provider_name,
                &request.run_id,
                LiveRunTime::now().epoch_seconds,
            )
            .map_err(|error| LiveIndexError::Heartbeat(error.to_string()))?;
            let batch = provider.fetch(entry, provider_checkpoint.as_deref())?;
            let stored_checkpoint = checkpoint_after_batch(&batch)?;
            let encoded_checkpoint = serde_json::to_string(&stored_checkpoint)?;
            let content_revision =
                format!("{}:{}:{}", request.run_id, entry.catalog_id, page_index);
            let outcome = commit_content_then_checkpoint(
                control,
                &request.catalog_name,
                &request.provider_name,
                &scope,
                &encoded_checkpoint,
                &request.timestamp,
                || {
                    write_content_batch(
                        content,
                        entry,
                        &batch,
                        &content_revision,
                        &request.timestamp,
                    )
                },
            )?;
            metrics.record_write(outcome);
            if matches!(stored_checkpoint, StoredCheckpoint::Complete) {
                metrics.journals_succeeded += 1;
                break;
            }
            let StoredCheckpoint::Provider { value } = stored_checkpoint else {
                unreachable!("complete checkpoint returned above")
            };
            if !seen_checkpoints.insert(value.clone()) {
                return Err(LiveIndexError::InvalidConfig(
                    "index provider returned a repeated checkpoint".to_string(),
                ));
            }
            provider_checkpoint = Some(value);
            if page_index + 1 == MAX_PROVIDER_PAGES_PER_JOURNAL {
                return Err(LiveIndexError::InvalidConfig(format!(
                    "provider page limit exceeded for catalog entry {}",
                    entry.catalog_id
                )));
            }
        }
    }
    metrics.emit_terminal(
        &request.run_id,
        &request.catalog_name,
        &request.provider_name,
        &request.worker_id.to_string(),
        "success",
    );
    Ok(metrics)
}

fn decode_checkpoint(value: &str) -> Result<StoredCheckpoint, LiveIndexError> {
    serde_json::from_str(value).map_err(|_| {
        LiveIndexError::InvalidConfig(
            "provider control checkpoint is invalid; remove the disposable control database"
                .to_string(),
        )
    })
}

fn checkpoint_after_batch(batch: &ProviderBatch) -> Result<StoredCheckpoint, LiveIndexError> {
    if batch.is_complete {
        if batch.next_checkpoint.is_some() {
            return Err(LiveIndexError::InvalidConfig(
                "complete provider batch must not include a next checkpoint".to_string(),
            ));
        }
        return Ok(StoredCheckpoint::Complete);
    }
    let value = batch.next_checkpoint.clone().ok_or_else(|| {
        LiveIndexError::InvalidConfig(
            "incomplete provider batch must include a next checkpoint".to_string(),
        )
    })?;
    Ok(StoredCheckpoint::Provider { value })
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn run_notify_for_manifest(
    config: &LiveIndexConfig,
    db_name: &str,
    manifest_path: &Path,
) -> Result<i32, LiveIndexError> {
    let state_dir = config.project_root.join("data").join("push_state");
    let mut command = Command::new(&config.application_executable);
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{self, BufReader, Cursor, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::{Command, Stdio};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use litradar_domain::{
        ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft,
        JournalRankings, ProviderBatch,
    };
    use litradar_provider::{IndexContentProvider, ProviderError};
    use rusqlite::{Connection, ErrorCode};
    use tempfile::tempdir;
    use tracing_subscriber::fmt::MakeWriter;

    use super::{
        emit_worker_failure, fetch_worker_assignments_with_provider, index_entries_with_provider,
        prepare_worker_requests, run_live_index, run_live_index_worker_with_io,
        run_worker_processes_with_launcher, validate_live_config, worker_failure_error,
        DirectIndexRequest, LaunchedWorkerProcess, LeaseHeartbeat, LiveIndexConfig, LiveIndexError,
        LiveIndexWorkerFailure, LiveIndexWorkerFailureClass, LiveIndexWorkerOperation,
        LiveIndexWorkerRequest, LiveRunTime, ParentWriterContext, StoredCheckpoint,
        OPENALEX_MAX_WORKERS_PER_PROCESS,
    };
    use crate::control::{
        acquire_lease, commit_content_then_checkpoint, open_control_db, read_checkpoint,
        release_lease, write_checkpoint, CheckpointScope, ContentCheckpointCommitError,
    };
    use crate::schema::{open_content_db, write_content_batch, ContentDatabaseError};
    use crate::stats::IndexRunMetrics;
    use crate::worker_protocol::{
        read_message, write_message, ParentMessage, WorkerJournalAssignment, WorkerMessage,
        PROTOCOL_VERSION,
    };

    #[derive(Clone, Default)]
    struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        /// Build a JSON tracing subscriber backed by this capture buffer.
        fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .with_writer(self.clone())
                .json()
                .flatten_event(true)
                .finish()
        }

        /// Return captured JSON Lines as UTF-8 text.
        fn text(&self) -> String {
            String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured worker logs should be UTF-8")
        }
    }

    struct CapturedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedLogs {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    struct StaticProvider {
        calls: Mutex<usize>,
    }

    impl StaticProvider {
        fn new() -> Self {
            Self {
                calls: Mutex::new(0),
            }
        }
    }

    impl IndexContentProvider for StaticProvider {
        fn fetch(
            &self,
            catalog: &JournalCatalogEntry,
            checkpoint: Option<&str>,
        ) -> Result<ProviderBatch, ProviderError> {
            assert!(checkpoint.is_none());
            *self.calls.lock().expect("call count should lock") += 1;
            Ok(canonical_batch(catalog))
        }
    }

    struct TwoPageProvider {
        second_fetch: mpsc::Sender<()>,
    }

    impl IndexContentProvider for TwoPageProvider {
        fn fetch(
            &self,
            catalog: &JournalCatalogEntry,
            checkpoint: Option<&str>,
        ) -> Result<ProviderBatch, ProviderError> {
            let mut batch = canonical_batch(catalog);
            match checkpoint {
                None => {
                    batch.is_complete = false;
                    batch.next_checkpoint = Some("cursor-1".to_string());
                }
                Some("cursor-1") => {
                    self.second_fetch
                        .send(())
                        .expect("second fetch observation should send");
                }
                Some(_) => panic!("unexpected provider checkpoint"),
            }
            Ok(batch)
        }
    }

    fn catalog(id: &str) -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: id.to_string(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
    }

    fn canonical_batch(catalog: &JournalCatalogEntry) -> ProviderBatch {
        ProviderBatch {
            catalog_id: catalog.catalog_id.clone(),
            journal: JournalDraft {
                catalog_id: catalog.catalog_id.clone(),
                observed_title: Some(catalog.title.clone()),
                observed_issns: catalog.all_issns.clone(),
                observed_title_aliases: Vec::new(),
            },
            issues: vec![IssueDraft {
                catalog_id: catalog.catalog_id.clone(),
                publication_year: Some(2026),
                title: None,
                volume: Some("1".to_string()),
                number: Some("2".to_string()),
                date: Some("2026-07".to_string()),
            }],
            articles: vec![ArticleDraft {
                catalog_id: catalog.catalog_id.clone(),
                title: "Shared Article".to_string(),
                publication_year: Some(2026),
                date: Some("2026-07-18".to_string()),
                issue_title: None,
                volume: Some("1".to_string()),
                issue_number: Some("2".to_string()),
                authors: vec![ArticleAuthorDraft {
                    display_name: "Ada Lovelace".to_string(),
                }],
                start_page: Some("1".to_string()),
                end_page: Some("8".to_string()),
                abstract_text: None,
                doi: Some("10.1000/shared".to_string()),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_doi: None,
            }],
            is_complete: true,
            next_checkpoint: None,
        }
    }

    fn direct_request(provider_name: &str, run_id: &str) -> DirectIndexRequest {
        DirectIndexRequest {
            catalog_name: "chinese_journals".to_string(),
            provider_name: provider_name.to_string(),
            run_id: run_id.to_string(),
            timestamp: "2026-07-18T00:00:00Z".to_string(),
            worker_id: 0,
            resume: true,
            update: false,
            entries: vec![catalog("journal-1")],
        }
    }

    fn fetch_worker_request(provider_name: &str, run_id: &str) -> LiveIndexWorkerRequest {
        LiveIndexWorkerRequest {
            protocol_version: PROTOCOL_VERSION,
            catalog_name: "chinese_journals".to_string(),
            provider_name: provider_name.to_string(),
            run_id: run_id.to_string(),
            worker_id: 0,
            process_count: 1,
            source_worker_count: 1,
            schedule_epoch_unix_millis: 0,
            timeout_seconds: 10,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            assignments: vec![WorkerJournalAssignment {
                journal_ordinal: 0,
                entry: catalog("journal-1"),
                initial_checkpoint: None,
            }],
        }
    }

    #[test]
    fn provider_switch_uses_new_checkpoint_namespace_and_same_content_ids() {
        let directory = tempdir().expect("temporary directory should create");
        let provider = StaticProvider::new();
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let request_a = direct_request("provider-a", "run-a");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let now = LiveRunTime::now().epoch_seconds;
        acquire_lease(
            &control,
            &request_a.catalog_name,
            &request_a.provider_name,
            &request_a.run_id,
            now,
        )
        .expect("provider A lease should acquire");
        index_entries_with_provider(&content, &control, &provider, &request_a)
            .expect("provider A should index");
        let article_id = content
            .query_row("SELECT article_id FROM articles", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("article id should read");
        release_lease(
            &control,
            &request_a.catalog_name,
            &request_a.provider_name,
            &request_a.run_id,
        )
        .expect("provider A lease should release");

        let request_b = direct_request("provider-b", "run-b");
        acquire_lease(
            &control,
            &request_b.catalog_name,
            &request_b.provider_name,
            &request_b.run_id,
            now,
        )
        .expect("provider B lease should acquire");
        index_entries_with_provider(&content, &control, &provider, &request_b)
            .expect("provider B should index");
        let replayed_id = content
            .query_row("SELECT article_id FROM articles", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("article id should remain");
        assert_eq!(article_id, replayed_id);
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM articles", [], |row| row
                    .get::<_, i64>(0))
                .expect("article count should read"),
            1
        );
        let scope = CheckpointScope::Journal {
            catalog_id: "journal-1".to_string(),
        };
        assert!(
            read_checkpoint(&control, "chinese_journals", "provider-a", &scope)
                .expect("provider A checkpoint should read")
                .is_some()
        );
        assert!(
            read_checkpoint(&control, "chinese_journals", "provider-b", &scope)
                .expect("provider B checkpoint should read")
                .is_some()
        );
    }

    #[test]
    fn deleting_control_state_replays_without_changing_content_cardinality() {
        let directory = tempdir().expect("temporary directory should create");
        let provider = StaticProvider::new();
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let request = direct_request("provider-a", "run-a");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let now = LiveRunTime::now().epoch_seconds;
        acquire_lease(
            &control,
            &request.catalog_name,
            &request.provider_name,
            &request.run_id,
            now,
        )
        .expect("lease should acquire");
        index_entries_with_provider(&content, &control, &provider, &request)
            .expect("first run should index");
        drop(control);
        std::fs::remove_file(&control_path).expect("control database should delete");
        let replay_control = open_control_db(&control_path).expect("control should recreate");
        let mut replay = request.clone();
        replay.run_id = "run-b".to_string();
        acquire_lease(
            &replay_control,
            &replay.catalog_name,
            &replay.provider_name,
            &replay.run_id,
            now,
        )
        .expect("replay lease should acquire");
        let metrics = index_entries_with_provider(&content, &replay_control, &provider, &replay)
            .expect("control-loss replay should succeed");
        assert_eq!(metrics.articles_changed, 0);
        for table in ["journals", "issues", "articles", "article_change_events"] {
            let count = content
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("row count should read");
            assert_eq!(count, 1, "unexpected replay count for {table}");
        }
    }

    #[test]
    fn worker_partitioning_preserves_every_catalog_entry_once() {
        let config = LiveIndexConfig {
            application_executable: "litradar".into(),
            project_root: ".".into(),
            secret_key_file: "secret.key".into(),
            file: None,
            worker_count: 2,
            process_count: 3,
            issue_batch_size: 2,
            timeout_seconds: 10,
            resume: true,
            update: false,
            notify: false,
            notify_dry_run: true,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            index_provider_routes: BTreeMap::from([(
                "catalog".to_string(),
                "scholarly".to_string(),
            )]),
        };
        let entries = (0..7)
            .map(|index| catalog(&format!("journal-{index}")))
            .collect::<Vec<_>>();
        let directory = tempdir().expect("temporary control directory should create");
        let control = open_control_db(directory.path().join("control.sqlite"))
            .expect("control database should open");
        let context = ParentWriterContext {
            catalog_name: "catalog".to_string(),
            provider_name: "scholarly".to_string(),
            run_id: "run".to_string(),
            timestamp: "time".to_string(),
        };
        let (requests, metrics) =
            prepare_worker_requests(&config, &control, &context, 123_456, &entries)
                .expect("worker requests should prepare");
        assert_eq!(requests.len(), 3);
        assert_eq!(metrics.journals_total, entries.len());
        assert_eq!(metrics.journals_resumed, 0);
        assert!(requests
            .iter()
            .all(|request| request.source_worker_count == 2));
        assert!(requests
            .iter()
            .all(|request| request.schedule_epoch_unix_millis == 123_456));
        let mut excessive_workers = config.clone();
        excessive_workers.worker_count = OPENALEX_MAX_WORKERS_PER_PROCESS + 1;
        assert!(matches!(
            validate_live_config(&excessive_workers),
            Err(LiveIndexError::InvalidConfig(message))
                if message == "worker_count must be at most 6 for scholarly indexing"
        ));
        let mut excessive_processes = config.clone();
        excessive_processes.process_count = 4;
        assert!(matches!(
            validate_live_config(&excessive_processes),
            Err(LiveIndexError::InvalidConfig(message))
                if message == "process_count must be at most 3 for scholarly indexing"
        ));
        let directory = tempdir().expect("temporary directory should create");
        excessive_processes.project_root = directory.path().to_path_buf();
        assert!(matches!(
            run_live_index(&excessive_processes),
            Err(LiveIndexError::InvalidConfig(message))
                if message == "process_count must be at most 3 for scholarly indexing"
        ));
        assert!(!directory.path().join("data").exists());
        let ids = requests
            .iter()
            .flat_map(|request| request.assignments.iter())
            .map(|assignment| assignment.entry.catalog_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), entries.len());
        assert!(requests
            .iter()
            .all(|request| !request.assignments.is_empty()));
        let request_json =
            serde_json::to_string(&requests[0]).expect("worker request should serialize");
        assert!(!request_json.contains("content_path"));
        assert!(!request_json.contains("control_path"));
    }

    #[test]
    fn single_writer_parent_preloads_complete_and_provider_checkpoints() {
        let mut config = LiveIndexConfig {
            application_executable: "litradar".into(),
            project_root: ".".into(),
            secret_key_file: "secret.key".into(),
            file: None,
            worker_count: 2,
            process_count: 3,
            issue_batch_size: 2,
            timeout_seconds: 10,
            resume: true,
            update: false,
            notify: false,
            notify_dry_run: true,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            index_provider_routes: BTreeMap::new(),
        };
        let entries = vec![catalog("complete"), catalog("resumable")];
        let directory = tempdir().expect("temporary control directory should create");
        config.project_root = directory.path().to_path_buf();
        let control = open_control_db(directory.path().join("control.sqlite"))
            .expect("control database should open");
        let context = ParentWriterContext {
            catalog_name: "catalog".to_string(),
            provider_name: "provider".to_string(),
            run_id: "run".to_string(),
            timestamp: "2026-07-19T00:00:00Z".to_string(),
        };
        let complete_scope = CheckpointScope::Journal {
            catalog_id: "complete".to_string(),
        };
        let resumable_scope = CheckpointScope::Journal {
            catalog_id: "resumable".to_string(),
        };
        write_checkpoint(
            &control,
            &context.catalog_name,
            &context.provider_name,
            &complete_scope,
            &serde_json::to_string(&StoredCheckpoint::Complete)
                .expect("complete checkpoint should serialize"),
            &context.timestamp,
        )
        .expect("complete checkpoint should write");
        write_checkpoint(
            &control,
            &context.catalog_name,
            &context.provider_name,
            &resumable_scope,
            &serde_json::to_string(&StoredCheckpoint::Provider {
                value: "cursor-resume".to_string(),
            })
            .expect("provider checkpoint should serialize"),
            &context.timestamp,
        )
        .expect("provider checkpoint should write");

        let (requests, metrics) = prepare_worker_requests(&config, &control, &context, 7, &entries)
            .expect("parent should preload checkpoints");

        assert_eq!(metrics.journals_total, 2);
        assert_eq!(metrics.journals_resumed, 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].assignments.len(), 1);
        assert_eq!(requests[0].assignments[0].entry.catalog_id, "resumable");
        assert_eq!(
            requests[0].assignments[0].initial_checkpoint.as_deref(),
            Some("cursor-resume")
        );
    }

    #[test]
    fn worker_protocol_rejects_duplicate_parent_assignments_before_launch() {
        let directory = tempdir().expect("temporary writer directory should create");
        let content = open_content_db(directory.path().join("content.sqlite"))
            .expect("content database should open");
        let control = open_control_db(directory.path().join("control.sqlite"))
            .expect("control database should open");
        let context = ParentWriterContext {
            catalog_name: "chinese_journals".to_string(),
            provider_name: "fixture".to_string(),
            run_id: "run-duplicate".to_string(),
            timestamp: "2026-07-19T00:00:00Z".to_string(),
        };
        let mut first = fetch_worker_request(&context.provider_name, &context.run_id);
        first.process_count = 2;
        let mut second = first.clone();
        second.worker_id = 1;
        let request_dir = directory.path().join("worker-requests");

        let error = run_worker_processes_with_launcher(
            &request_dir,
            &content,
            &control,
            &context,
            vec![first, second],
            IndexRunMetrics::default(),
            Duration::from_secs(1),
            |_, _| panic!("invalid assignments must fail before process launch"),
            |_| {},
        )
        .expect_err("duplicate journal assignments should fail closed");

        assert!(matches!(error, LiveIndexError::Worker(_)));
        assert!(!request_dir.exists());
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM articles", [], |row| row
                    .get::<_, i64>(0))
                .expect("article count should read"),
            0
        );
    }

    #[test]
    fn single_writer_worker_waits_for_durable_ack_before_next_fetch() {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("loopback protocol listener should bind");
        let address = listener
            .local_addr()
            .expect("loopback protocol address should resolve");
        let request = fetch_worker_request("fixture", "run-backpressure");
        let worker_request = request.clone();
        let (second_fetch_sender, second_fetch_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let stream =
                TcpStream::connect(address).expect("worker protocol stream should connect");
            let mut reader = BufReader::new(
                stream
                    .try_clone()
                    .expect("worker protocol reader should clone"),
            );
            let mut writer = stream;
            let provider = TwoPageProvider {
                second_fetch: second_fetch_sender,
            };
            let mut sequence = 0;
            fetch_worker_assignments_with_provider(
                &worker_request,
                &provider,
                &mut reader,
                &mut writer,
                &mut sequence,
            )
            .map(|()| sequence)
        });
        let (stream, _) = listener
            .accept()
            .expect("parent protocol stream should accept");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("parent protocol reader should clone"),
        );
        let mut writer = stream;

        let first: WorkerMessage =
            read_message(&mut reader).expect("first provider page should arrive");
        assert!(second_fetch_receiver.try_recv().is_err());
        let WorkerMessage::Batch {
            sequence,
            journal_ordinal,
            page_index,
            batch,
            ..
        } = first
        else {
            panic!("worker should emit a batch before waiting")
        };
        assert!(!batch.is_complete);
        write_message(
            &mut writer,
            &ParentMessage::Committed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence,
                journal_ordinal,
                page_index,
                is_complete: false,
            },
        )
        .expect("first durable acknowledgement should send");
        second_fetch_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("next fetch should start only after acknowledgement");

        let second: WorkerMessage =
            read_message(&mut reader).expect("second provider page should arrive");
        let WorkerMessage::Batch {
            sequence,
            journal_ordinal,
            page_index,
            batch,
            ..
        } = second
        else {
            panic!("worker should emit the second batch")
        };
        assert!(batch.is_complete);
        write_message(
            &mut writer,
            &ParentMessage::Committed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence,
                journal_ordinal,
                page_index,
                is_complete: true,
            },
        )
        .expect("final durable acknowledgement should send");

        assert_eq!(
            worker
                .join()
                .expect("worker protocol thread should join")
                .expect("worker protocol should complete"),
            2
        );
    }

    #[test]
    fn worker_protocol_stdio_transport_round_trips_one_message() {
        let child = spawn_stdio_echo_process();
        let launched = LaunchedWorkerProcess::from_child_stdio(child, 0)
            .expect("stdio worker pipes should attach");
        let LaunchedWorkerProcess {
            mut child,
            reader,
            mut writer,
        } = launched;
        let message = ParentMessage::Committed {
            protocol_version: PROTOCOL_VERSION,
            worker_id: 0,
            sequence: 4,
            journal_ordinal: 2,
            page_index: 3,
            is_complete: true,
        };

        write_message(&mut writer, &message).expect("protocol message should write to child stdin");
        drop(writer);
        let actual: ParentMessage = read_message(&mut BufReader::new(reader))
            .expect("protocol message should return from child stdout");
        let status = child.wait().expect("stdio echo child should be reaped");

        assert_eq!(actual, message);
        assert!(status.success());
    }

    #[cfg(target_os = "windows")]
    fn spawn_stdio_echo_process() -> std::process::Child {
        Command::new("cmd")
            .args(["/D", "/S", "/C", "more"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Windows stdio echo child should start")
    }

    #[cfg(not(target_os = "windows"))]
    fn spawn_stdio_echo_process() -> std::process::Child {
        Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("stdio echo child should start")
    }

    #[test]
    fn single_writer_worker_entrypoint_streams_terminal_without_database_paths() {
        let directory = tempdir().expect("temporary directory should create");
        let mut request = fetch_worker_request("scholarly", "run-worker");
        request.assignments.clear();
        let request_path = directory.path().join("worker-request.json");
        let request_bytes = serde_json::to_vec(&request).expect("worker request should serialize");
        let request_text =
            String::from_utf8(request_bytes.clone()).expect("worker request should be UTF-8");
        std::fs::write(&request_path, request_bytes).expect("worker request should write");

        let captured = CapturedLogs::default();
        let mut output = Vec::new();
        tracing::subscriber::with_default(captured.subscriber(), || {
            run_live_index_worker_with_io(&request_path, Cursor::new(Vec::<u8>::new()), &mut output)
                .expect("worker entrypoint should stream terminal JSON")
        });
        let message: WorkerMessage = read_message(&mut Cursor::new(output))
            .expect("terminal worker message should deserialize");

        assert!(matches!(
            message,
            WorkerMessage::Succeeded {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence: 0,
            }
        ));
        assert!(!request_text.contains("content_path"));
        assert!(!request_text.contains("control_path"));
        assert!(!captured.text().contains("index.writer"));
    }

    #[test]
    fn single_writer_fetch_worker_source_has_no_sqlite_authority() {
        let source = include_str!("live.rs");
        let entrypoint_start = source
            .find("pub fn run_live_index_worker_from_file_path")
            .expect("worker entrypoint should exist");
        let entrypoint_end = source[entrypoint_start..]
            .find("fn validate_live_config")
            .map(|offset| entrypoint_start + offset)
            .expect("worker entrypoint boundary should exist");
        let fetch_start = source
            .find("fn run_fetch_worker_stream")
            .expect("fetch worker stream should exist");
        let fetch_end = source[fetch_start..]
            .find("fn build_index_registration")
            .map(|offset| fetch_start + offset)
            .expect("fetch worker boundary should exist");
        let worker_source = format!(
            "{}\n{}",
            &source[entrypoint_start..entrypoint_end],
            &source[fetch_start..fetch_end]
        );

        for forbidden in [
            "open_content_db(",
            "open_control_db(",
            "write_content_batch(",
            "commit_content_then_checkpoint(",
            "heartbeat_lease(",
            "write_checkpoint(",
        ] {
            assert!(
                !worker_source.contains(forbidden),
                "fetch worker retained forbidden persistence authority: {forbidden}"
            );
        }
    }

    #[test]
    fn worker_protocol_failure_boundary_is_redacted() {
        let captured = CapturedLogs::default();
        let sensitive_sentinel = "C:\\private\\catalog.sqlite secret-key@example.invalid";
        let boundary_error =
            LiveIndexError::Worker(super::WORKER_PROTOCOL_FAILURE_MESSAGE.to_string());
        let failure = LiveIndexWorkerFailure::from_error(&boundary_error);
        let message = WorkerMessage::Failed {
            protocol_version: PROTOCOL_VERSION,
            worker_id: 2,
            sequence: 0,
            failure: failure.clone(),
        };
        tracing::subscriber::with_default(captured.subscriber(), || {
            emit_worker_failure(2, &failure);
        });
        let combined = format!(
            "{}\n{}\n{}",
            captured.text(),
            serde_json::to_string(&message).expect("worker message should serialize"),
            worker_failure_error(2, &failure)
        );

        assert_eq!(failure.class, LiveIndexWorkerFailureClass::Worker);
        assert_eq!(failure.operation, LiveIndexWorkerOperation::WorkerProtocol);
        assert!(combined.contains("index.worker.failed"));
        assert!(combined.contains("\"operation\":\"worker_protocol\""));
        assert!(!combined.contains(sensitive_sentinel));
    }

    #[test]
    fn single_writer_checkpoint_failure_replays_committed_content_idempotently() {
        let directory = tempdir().expect("temporary directory should create");
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let catalog = catalog("journal-single-writer-replay");
        let batch = canonical_batch(&catalog);
        let scope = CheckpointScope::Journal {
            catalog_id: catalog.catalog_id.clone(),
        };
        control
            .execute_batch(
                "CREATE TRIGGER fail_single_writer_checkpoint
                 BEFORE INSERT ON provider_checkpoints
                 BEGIN SELECT RAISE(ABORT, 'forced checkpoint failure'); END;",
            )
            .expect("checkpoint failpoint should install");

        let checkpoint_error = commit_content_then_checkpoint(
            &control,
            "chinese_journals",
            "provider-a",
            &scope,
            "complete",
            "2026-07-18T00:00:00Z",
            || {
                write_content_batch(
                    &content,
                    &catalog,
                    &batch,
                    "revision-single-writer",
                    "2026-07-18T00:00:00Z",
                )
            },
        )
        .expect_err("checkpoint failure should follow committed content");
        assert!(matches!(
            checkpoint_error,
            ContentCheckpointCommitError::Control(_)
        ));
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM articles", [], |row| row
                    .get::<_, i64>(0))
                .expect("article count should read"),
            1
        );
        assert_eq!(
            read_checkpoint(&control, "chinese_journals", "provider-a", &scope)
                .expect("checkpoint should read"),
            None
        );
        control
            .execute_batch("DROP TRIGGER fail_single_writer_checkpoint")
            .expect("checkpoint failpoint should drop");

        let replay = commit_content_then_checkpoint(
            &control,
            "chinese_journals",
            "provider-a",
            &scope,
            "complete",
            "2026-07-18T00:01:00Z",
            || {
                write_content_batch(
                    &content,
                    &catalog,
                    &batch,
                    "revision-single-writer",
                    "2026-07-18T00:00:00Z",
                )
            },
        )
        .expect("single-writer replay should advance the checkpoint");
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(replay.change_events_emitted, 0);
        assert_eq!(
            read_checkpoint(&control, "chinese_journals", "provider-a", &scope)
                .expect("checkpoint should read")
                .as_deref(),
            Some("complete")
        );
    }

    #[test]
    fn worker_failure_message_retains_structured_sqlite_codes() {
        let directory = tempdir().expect("temporary SQLite directory should create");
        let database_path = directory.path().join("busy.sqlite");
        let holder = Connection::open(&database_path).expect("holder connection should open");
        holder
            .execute_batch(
                "CREATE TABLE writes (value INTEGER NOT NULL);
                 BEGIN IMMEDIATE;
                 INSERT INTO writes VALUES (1);",
            )
            .expect("holder should own the write transaction");
        let contender = Connection::open(&database_path).expect("contender should open");
        contender
            .busy_timeout(Duration::ZERO)
            .expect("contender busy timeout should configure");
        let sqlite_error = contender
            .execute("INSERT INTO writes VALUES (2)", [])
            .expect_err("uncoordinated contender should be busy or locked");
        let expected = match &sqlite_error {
            rusqlite::Error::SqliteFailure(failure, _) => {
                assert!(matches!(
                    failure.code,
                    ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                ));
                (format!("{:?}", failure.code), failure.extended_code)
            }
            other => panic!("expected typed SQLite failure, received {other:?}"),
        };
        let error = LiveIndexError::Commit(ContentCheckpointCommitError::Content(
            ContentDatabaseError::Sqlite(sqlite_error),
        ));

        let failure = LiveIndexWorkerFailure::from_error(&error);
        assert_eq!(failure.class, LiveIndexWorkerFailureClass::Sqlite);
        assert_eq!(failure.operation, LiveIndexWorkerOperation::ContentCommit);
        assert_eq!(failure.sqlite_code.as_deref(), Some(expected.0.as_str()));
        assert_eq!(failure.sqlite_extended_code, Some(expected.1));
        assert!(failure.is_busy_or_locked);
        let message = WorkerMessage::Failed {
            protocol_version: PROTOCOL_VERSION,
            worker_id: 2,
            sequence: 0,
            failure: failure.clone(),
        };
        let payload = serde_json::to_value(&message).expect("worker message should serialize");
        assert!(payload.get("error").is_none());
        assert_eq!(payload["failure"]["class"], "sqlite");
        assert_eq!(payload["failure"]["operation"], "content_commit");
        let captured = CapturedLogs::default();
        tracing::subscriber::with_default(captured.subscriber(), || {
            emit_worker_failure(2, &failure);
        });
        let event: serde_json::Value = serde_json::from_str(
            captured
                .text()
                .lines()
                .next()
                .expect("worker failure event should be captured"),
        )
        .expect("worker failure event should be JSON");
        assert_eq!(event["event"], "index.worker.failed");
        assert_eq!(event["worker_id"], 2);
        assert_eq!(event["failure_class"], "sqlite");
        assert_eq!(event["operation"], "content_commit");
        assert_eq!(event["sqlite_code"], expected.0);
        assert_eq!(event["sqlite_extended_code"], expected.1);
        assert_eq!(event["is_busy_or_locked"], true);
    }

    #[test]
    fn worker_failure_event_excludes_free_form_sensitive_values() {
        let sentinels = [
            "C:\\private\\worker.sqlite",
            "openalex-key-sentinel",
            "operator-sentinel@example.test",
            "10.9999/sentinel-doi",
            "Sentinel Article Title",
            "cursor-sentinel-value",
            "response-body-sentinel",
            "Bearer sentinel-token-value",
        ];
        let error = LiveIndexError::Worker(sentinels.join(" | "));
        let failure = LiveIndexWorkerFailure::from_error(&error);
        let message = WorkerMessage::Failed {
            protocol_version: PROTOCOL_VERSION,
            worker_id: 5,
            sequence: 0,
            failure: failure.clone(),
        };
        let captured = CapturedLogs::default();
        tracing::subscriber::with_default(captured.subscriber(), || {
            emit_worker_failure(5, &failure);
        });
        let parent_error = worker_failure_error(5, &failure);
        let combined = format!(
            "{}\n{}\n{parent_error}",
            serde_json::to_string(&message).expect("worker message should serialize"),
            captured.text()
        );

        assert_eq!(failure.class, LiveIndexWorkerFailureClass::Worker);
        assert_eq!(failure.operation, LiveIndexWorkerOperation::WorkerProcess);
        assert!(failure.sqlite_code.is_none());
        assert!(failure.sqlite_extended_code.is_none());
        assert!(!failure.is_busy_or_locked);
        assert!(combined.contains("index.worker.failed"));
        assert!(combined.contains("\"worker_id\":5"));
        assert!(combined.contains("\"failure_class\":\"worker\""));
        let event: serde_json::Value = serde_json::from_str(
            captured
                .text()
                .lines()
                .next()
                .expect("worker failure event should be captured"),
        )
        .expect("worker failure event should be JSON");
        assert_eq!(event["has_sqlite_code"], false);
        assert!(event.get("sqlite_code").is_none());
        assert!(event.get("sqlite_extended_code").is_none());
        for sentinel in sentinels {
            assert!(
                !combined.contains(sentinel),
                "worker boundary exposed sensitive sentinel"
            );
        }
    }

    #[test]
    fn parent_heartbeat_renews_provider_scoped_lease() {
        let directory = tempdir().expect("temporary directory should create");
        let control_path = directory.path().join("control.sqlite");
        let control = open_control_db(&control_path).expect("control should open");
        let now = LiveRunTime::now().epoch_seconds;
        acquire_lease(&control, "catalog", "provider", "run", now).expect("lease should acquire");
        let mut heartbeat = LeaseHeartbeat::start(
            control_path.clone(),
            "catalog".to_string(),
            "provider".to_string(),
            "run".to_string(),
            Duration::from_millis(10),
        );
        std::thread::sleep(Duration::from_millis(35));
        heartbeat.stop_and_check().expect("heartbeat should stop");
        let heartbeat_at = control
            .query_row(
                "SELECT heartbeat_at FROM provider_leases WHERE catalog_name = 'catalog'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("heartbeat timestamp should read");
        assert!(heartbeat_at >= now);
    }
}
