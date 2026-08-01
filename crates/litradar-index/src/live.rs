//! Provider-neutral live catalog indexing orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_domain::{IndexFetchContext, IndexSyncMode, JournalCatalogEntry, ProviderProgress};
use litradar_provider::{
    IndexContentProvider, ProviderError, ProviderRegistration, ProviderRegistryError,
};
pub use litradar_sources::ProviderProxySelection;
use litradar_sources::{
    cnki_index_registration_with_workers, cnki_oversea_index_registration,
    scholarly_index_registration, LiveCnkiConfig, LiveCnkiTransport, LiveDomesticCnkiConfig,
    LiveDomesticCnkiTransport, LiveScholarlyConfig, LiveScholarlyTransport, ProviderProxy,
    CNKI_OVERSEA_PROVIDER_NAME, CNKI_PROVIDER_NAME, SCHOLARLY_PROVIDER_NAME,
};
use litradar_worker::process_supervisor::SupervisedChild;
use rusqlite::{Connection, ErrorCode};
use serde::Serialize;

use crate::batch::{
    admit_batch, complete_batch, complete_catalog, heartbeat_batch_lease, new_batch_owner_id,
    open_batch_db, release_batch_lease, replace_abandoning_batch, store_catalog_outcome,
    transition_catalog_phase, BatchAdmission, BatchCatalogOutcome, BatchCatalogPhase,
    BatchDatabaseError, CatalogInput, CatalogSelection, IndexBatch, IndexBatchCatalog,
    IndexBatchRequest, BATCH_DATABASE_FILE_NAME,
};
use crate::changes::{
    discard_content_change_events, write_content_change_manifest, ChangeWriteError,
};
use crate::control::{
    abandon_batch_checkpoints, acquire_lease, adopt_legacy_batch_state,
    commit_content_then_progress, has_catalog_alias_sync_state, heartbeat_lease, open_control_db,
    prepare_journal_sync, read_batch_journal_state, release_lease, ContentCheckpointCommitError,
    ControlDatabaseError, JournalSyncPreparation,
};
use crate::identity::{ArticleIdentityError, ArticleMergeError};
use crate::schema::{
    open_content_db, optimize_content_db, reconcile_catalog_identities, write_content_batch,
    ContentDatabaseError,
};
use crate::stats::IndexRunMetrics;
use crate::transforms::CatalogContractError;
use crate::worker_protocol::{
    read_message, write_message, ParentMessage, ProtocolError,
    WorkerBootstrap as LiveIndexWorkerBootstrap, WorkerFailure as LiveIndexWorkerFailure,
    WorkerFailureClass as LiveIndexWorkerFailureClass, WorkerJournalAssignment, WorkerMessage,
    WorkerOperation as LiveIndexWorkerOperation, WorkerRequest as LiveIndexWorkerRequest,
    PROTOCOL_VERSION,
};

const LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS: u64 = 30;
const MAX_PROVIDER_PAGES_PER_JOURNAL: usize = 100_000;
const WORKER_PROTOCOL_FAILURE_MESSAGE: &str = "worker protocol operation failed";
const LEGACY_ALIAS_SYNC_STATE_MESSAGE: &str =
    "legacy catalog alias has provider synchronization state";

/// Live index run configuration.
#[derive(Clone)]
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
    /// Whether to run incremental synchronization and publish a change manifest.
    pub update: bool,
    /// Whether to scan complete Provider history without publishing a change manifest.
    pub full_rescan: bool,
    /// Whether to run `notify` after an update manifest is written.
    pub notify: bool,
    /// Whether notify handoff should use dry-run mode.
    pub notify_dry_run: bool,
    /// Scholarly source runtime configuration.
    pub scholarly_config: LiveScholarlyConfig,
    /// Domestic CNKI captcha solver token loaded from runtime secrets or probe env.
    pub cnki_captcha_token: Option<String>,
    /// Validated per-Provider direct-or-explicit proxy selection.
    pub provider_proxy_selection: ProviderProxySelection,
    /// Validated catalog-stem to indexing-provider routes loaded outside index databases.
    pub index_provider_routes: BTreeMap<String, String>,
}

impl fmt::Debug for LiveIndexConfig {
    /// Format live indexing configuration without exposing provider credentials.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveIndexConfig")
            .field("application_executable", &self.application_executable)
            .field("project_root", &self.project_root)
            .field("secret_key_file", &self.secret_key_file)
            .field("file", &self.file)
            .field("worker_count", &self.worker_count)
            .field("process_count", &self.process_count)
            .field("issue_batch_size", &self.issue_batch_size)
            .field("timeout_seconds", &self.timeout_seconds)
            .field("resume", &self.resume)
            .field("update", &self.update)
            .field("full_rescan", &self.full_rescan)
            .field("notify", &self.notify)
            .field("notify_dry_run", &self.notify_dry_run)
            .field("index_provider_routes", &self.index_provider_routes)
            .field("provider_proxy_selection", &self.provider_proxy_selection)
            .field("provider_credentials", &"[REDACTED]")
            .finish()
    }
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
    /// Disposable project batch database, lease, or compatibility operation failed.
    Batch(String),
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
            Self::Batch(message) => formatter.write_str(message),
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
            | Self::Batch(_)
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

impl From<BatchDatabaseError> for LiveIndexError {
    /// Convert disposable project batch failures.
    fn from(error: BatchDatabaseError) -> Self {
        Self::Batch(error.to_string())
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

#[derive(Debug, Clone)]
struct DirectIndexRequest {
    catalog_name: String,
    provider_name: String,
    batch_id: String,
    run_id: String,
    timestamp: String,
    worker_id: usize,
    resume: bool,
    mode: IndexSyncMode,
    entries: Vec<JournalCatalogEntry>,
}

/// Parent-owned context required to commit worker batches.
#[derive(Debug, Clone)]
pub(crate) struct ParentWriterContext {
    /// Stable maintained catalog stem.
    pub(crate) catalog_name: String,
    /// Stable registered indexing provider.
    pub(crate) provider_name: String,
    /// Active project batch that owns journal progress.
    pub(crate) batch_id: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentCommitErrorKind {
    Json,
    Contract,
    IdentityMissing,
    IdentityConflictingAliases,
    MergeCatalogMismatch,
    MergeConflictingDoi,
    MergeConflictingPmid,
    MergeConflictingOther,
    RebuildRequired,
    InvalidCurrentSchema,
    ArticleIdCollision,
}

impl ContentCommitErrorKind {
    fn from_error(error: &ContentDatabaseError) -> Option<Self> {
        match error {
            ContentDatabaseError::Sqlite(_) => None,
            ContentDatabaseError::Json(_) => Some(Self::Json),
            ContentDatabaseError::Contract(_) => Some(Self::Contract),
            ContentDatabaseError::Identity(ArticleIdentityError::MissingIdentity) => {
                Some(Self::IdentityMissing)
            }
            ContentDatabaseError::Identity(ArticleIdentityError::ConflictingAliases { .. }) => {
                Some(Self::IdentityConflictingAliases)
            }
            ContentDatabaseError::Merge(ArticleMergeError::CatalogMismatch) => {
                Some(Self::MergeCatalogMismatch)
            }
            ContentDatabaseError::Merge(ArticleMergeError::ConflictingIdentifier { field })
                if field.eq_ignore_ascii_case("doi") =>
            {
                Some(Self::MergeConflictingDoi)
            }
            ContentDatabaseError::Merge(ArticleMergeError::ConflictingIdentifier { field })
                if field.eq_ignore_ascii_case("pmid") =>
            {
                Some(Self::MergeConflictingPmid)
            }
            ContentDatabaseError::Merge(ArticleMergeError::ConflictingIdentifier { .. }) => {
                Some(Self::MergeConflictingOther)
            }
            ContentDatabaseError::RebuildRequired { .. } => Some(Self::RebuildRequired),
            ContentDatabaseError::InvalidCurrentSchema(_) => Some(Self::InvalidCurrentSchema),
            ContentDatabaseError::ArticleIdCollision { .. } => Some(Self::ArticleIdCollision),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Contract => "contract",
            Self::IdentityMissing => "identity_missing",
            Self::IdentityConflictingAliases => "identity_conflicting_aliases",
            Self::MergeCatalogMismatch => "merge_catalog_mismatch",
            Self::MergeConflictingDoi => "merge_conflicting_doi",
            Self::MergeConflictingPmid => "merge_conflicting_pmid",
            Self::MergeConflictingOther => "merge_conflicting_other",
            Self::RebuildRequired => "rebuild_required",
            Self::InvalidCurrentSchema => "invalid_current_schema",
            Self::ArticleIdCollision => "article_id_collision",
        }
    }
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
            LiveIndexError::Batch(_) => Self::fixed(
                LiveIndexWorkerFailureClass::Control,
                LiveIndexWorkerOperation::ControlDatabase,
            ),
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
            | ControlDatabaseError::OwnershipLost { .. }
            | ControlDatabaseError::RunModeMismatch { .. }
            | ControlDatabaseError::RunOwnershipLost { .. }
            | ControlDatabaseError::BatchStateMismatch
            | ControlDatabaseError::InvalidSyncState { .. } => {
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

struct BatchLeaseHeartbeat {
    stop: Sender<()>,
    handle: Option<JoinHandle<Result<(), String>>>,
}

impl BatchLeaseHeartbeat {
    fn start(batch_path: PathBuf, batch_id: String, owner_id: String, interval: Duration) -> Self {
        let (stop, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let connection = open_batch_db(batch_path).map_err(|error| error.to_string())?;
            loop {
                match receiver.recv_timeout(interval) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                    Err(RecvTimeoutError::Timeout) => {
                        heartbeat_batch_lease(
                            &connection,
                            &batch_id,
                            &owner_id,
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
            .map_err(|_| LiveIndexError::Heartbeat("batch heartbeat thread panicked".to_string()))?
            .map_err(LiveIndexError::Heartbeat)
    }
}

impl Drop for BatchLeaseHeartbeat {
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
    let inputs = freeze_catalog_inputs(config, paths)?;
    let request = IndexBatchRequest::new(
        inputs,
        if config.file.is_some() {
            CatalogSelection::ExplicitFile
        } else {
            CatalogSelection::All
        },
        requested_sync_mode(config),
        config.issue_batch_size,
        config.notify,
        config.notify_dry_run,
    )?;
    let control_dir = config.project_root.join("data").join("index-control");
    std::fs::create_dir_all(&control_dir)?;
    let batch_path = control_dir.join(BATCH_DATABASE_FILE_NAME);
    let batch_connection = open_batch_db(&batch_path)?;
    let owner_id = new_batch_owner_id();
    let admitted_at = LiveRunTime::now().epoch_seconds;
    let admission = admit_batch(
        &batch_connection,
        &request,
        config.resume,
        &owner_id,
        admitted_at,
    )?;
    let batch = match admission {
        BatchAdmission::Ready(batch) => batch,
        BatchAdmission::Abandoning(abandoning) => {
            let replacement = cleanup_abandoning_batch(config, &abandoning).and_then(|()| {
                Ok(replace_abandoning_batch(
                    &batch_connection,
                    &abandoning.batch_id,
                    &request,
                    &owner_id,
                    LiveRunTime::now().epoch_seconds,
                )?)
            });
            match replacement {
                Ok(batch) => batch,
                Err(error) => {
                    let _ = release_batch_lease(&batch_connection, &abandoning.batch_id, &owner_id);
                    return Err(error);
                }
            }
        }
    };
    tracing::info!(
        event = "index.batch.admitted",
        component = "index",
        batch_id = batch.batch_id,
        resumed = batch.did_resume,
        catalog_count = batch.catalogs.len(),
    );
    let mut heartbeat = BatchLeaseHeartbeat::start(
        batch_path,
        batch.batch_id.clone(),
        owner_id.clone(),
        Duration::from_secs(LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS),
    );
    let execution = prepare_legacy_state(config, &batch, &request)
        .and_then(|()| run_batch_catalogs(config, &batch_connection, &batch, &request));
    let heartbeat_result = heartbeat.stop_and_check();
    let outcomes = match execution {
        Ok(outcomes) => outcomes,
        Err(error) => {
            let _ = release_batch_lease(&batch_connection, &batch.batch_id, &owner_id);
            if let Err(heartbeat_error) = heartbeat_result {
                tracing::error!(
                    event = "index.batch.heartbeat_failed",
                    component = "index",
                    batch_id = batch.batch_id,
                    error = %heartbeat_error,
                );
            }
            return Err(error);
        }
    };
    if let Err(error) = heartbeat_result {
        let _ = release_batch_lease(&batch_connection, &batch.batch_id, &owner_id);
        return Err(error);
    }
    if let Err(error) = complete_batch(
        &batch_connection,
        &batch.batch_id,
        &owner_id,
        LiveRunTime::now().epoch_seconds,
    ) {
        let _ = release_batch_lease(&batch_connection, &batch.batch_id, &owner_id);
        return Err(error.into());
    }
    Ok(LiveIndexOutcome {
        status: "succeeded".to_string(),
        message: None,
        csvs: outcomes,
    })
}

fn freeze_catalog_inputs(
    config: &LiveIndexConfig,
    paths: Vec<PathBuf>,
) -> Result<Vec<CatalogInput>, LiveIndexError> {
    paths
        .into_iter()
        .map(|path| {
            let catalog_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    LiveIndexError::InvalidConfig(
                        "catalog filename must have a UTF-8 stem".to_string(),
                    )
                })?;
            let provider_name =
                config
                    .index_provider_routes
                    .get(catalog_name)
                    .ok_or_else(|| {
                        LiveIndexError::InvalidConfig(format!(
                            "index_provider_routes has no route for catalog {catalog_name}"
                        ))
                    })?;
            Ok(CatalogInput::freeze(&path, provider_name.clone())?)
        })
        .collect()
}

fn cleanup_abandoning_batch(
    config: &LiveIndexConfig,
    abandoning: &IndexBatch,
) -> Result<(), LiveIndexError> {
    let control_dir = config.project_root.join("data").join("index-control");
    for catalog in &abandoning.catalogs {
        let connection =
            open_control_db(control_dir.join(format!("{}.sqlite", catalog.catalog_name)))?;
        abandon_batch_checkpoints(&connection, &abandoning.batch_id)?;
    }
    Ok(())
}

fn prepare_legacy_state(
    config: &LiveIndexConfig,
    batch: &IndexBatch,
    request: &IndexBatchRequest,
) -> Result<(), LiveIndexError> {
    if !config.resume {
        return Ok(());
    }
    let control_dir = config.project_root.join("data").join("index-control");
    for input in &request.catalogs {
        let connection =
            open_control_db(control_dir.join(format!("{}.sqlite", input.catalog_name)))?;
        let adoption = adopt_legacy_batch_state(
            &connection,
            &input.catalog_name,
            &input.provider_name,
            &batch.batch_id,
            request.mode,
            request.selection == CatalogSelection::ExplicitFile,
        )?;
        if adoption.checkpoints_adopted > 0 {
            tracing::info!(
                event = "index.batch.legacy_adopted",
                component = "index",
                batch_id = batch.batch_id,
                catalog = input.catalog_name,
                provider = input.provider_name,
                checkpoints = adoption.checkpoints_adopted,
                completed = adoption.anchors_adopted,
            );
        }
    }
    Ok(())
}

fn run_batch_catalogs(
    config: &LiveIndexConfig,
    batch_connection: &Connection,
    batch: &IndexBatch,
    request: &IndexBatchRequest,
) -> Result<Vec<LiveCsvIndexOutcome>, LiveIndexError> {
    run_batch_catalogs_with(config, batch_connection, batch, request, run_catalog)
}

fn run_batch_catalogs_with<RunCatalog>(
    config: &LiveIndexConfig,
    batch_connection: &Connection,
    batch: &IndexBatch,
    request: &IndexBatchRequest,
    mut run: RunCatalog,
) -> Result<Vec<LiveCsvIndexOutcome>, LiveIndexError>
where
    RunCatalog:
        FnMut(&LiveIndexConfig, &CatalogInput, &str) -> Result<LiveCsvIndexOutcome, LiveIndexError>,
{
    if batch.catalogs.len() != request.catalogs.len() {
        return Err(BatchDatabaseError::InvalidState {
            reason: "active batch catalog count changed after admission",
        }
        .into());
    }
    let mut outcomes = Vec::with_capacity(request.catalogs.len());
    for (stored, input) in batch.catalogs.iter().zip(&request.catalogs) {
        if stored.file_name != input.file_name || stored.catalog_name != input.catalog_name {
            return Err(BatchDatabaseError::InvalidState {
                reason: "active batch catalog order changed after admission",
            }
            .into());
        }
        if stored.phase == BatchCatalogPhase::Completed {
            outcomes.push(completed_catalog_outcome(config, stored, input)?);
            tracing::info!(
                event = "index.batch.catalog_skipped",
                component = "index",
                batch_id = batch.batch_id,
                catalog = input.catalog_name,
                provider = input.provider_name,
            );
            continue;
        }
        if stored.phase == BatchCatalogPhase::Pending {
            transition_catalog_phase(
                batch_connection,
                &batch.batch_id,
                &batch.owner_id,
                stored.ordinal,
                BatchCatalogPhase::Indexing,
                LiveRunTime::now().epoch_seconds,
            )?;
        } else if stored.phase != BatchCatalogPhase::Indexing {
            return Err(BatchDatabaseError::InvalidState {
                reason: "catalog finalization phase requires recovery support",
            }
            .into());
        }
        let outcome = run(config, input, &batch.batch_id)?;
        let persisted = persisted_catalog_outcome(config, input, &outcome);
        store_catalog_outcome(
            batch_connection,
            &batch.batch_id,
            &batch.owner_id,
            stored.ordinal,
            &persisted,
            LiveRunTime::now().epoch_seconds,
        )?;
        complete_catalog(
            batch_connection,
            &batch.batch_id,
            &batch.owner_id,
            stored.ordinal,
            &persisted,
            LiveRunTime::now().epoch_seconds,
        )?;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

fn persisted_catalog_outcome(
    config: &LiveIndexConfig,
    input: &CatalogInput,
    outcome: &LiveCsvIndexOutcome,
) -> BatchCatalogOutcome {
    BatchCatalogOutcome {
        run_id: outcome.run_id.clone(),
        journal_count: outcome.journal_count,
        written_article_count: outcome.written_article_count,
        source_attempt_count: outcome.source_attempt_count,
        manifest_path: outcome.manifest_path.as_ref().map(|_| {
            Path::new("data")
                .join("push_state")
                .join(format!("{}.changes.json", input.catalog_name))
                .to_string_lossy()
                .into_owned()
        }),
        notify_exit_code: config.notify.then_some(outcome.notify_exit_code).flatten(),
    }
}

fn completed_catalog_outcome(
    config: &LiveIndexConfig,
    stored: &IndexBatchCatalog,
    input: &CatalogInput,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let outcome = stored.outcome.as_ref().ok_or_else(|| {
        LiveIndexError::from(BatchDatabaseError::InvalidState {
            reason: "completed catalog has no persisted outcome",
        })
    })?;
    let manifest_path = outcome
        .manifest_path
        .as_ref()
        .map(|path| config.project_root.join(path).display().to_string());
    Ok(LiveCsvIndexOutcome {
        csv_path: input.path.display().to_string(),
        db_path: config
            .project_root
            .join("data")
            .join("index")
            .join(format!("{}.sqlite", input.catalog_name))
            .display()
            .to_string(),
        run_id: outcome.run_id.clone(),
        status: "succeeded".to_string(),
        journal_count: outcome.journal_count,
        written_article_count: outcome.written_article_count,
        source_attempt_count: outcome.source_attempt_count,
        manifest_path,
        notify_exit_code: outcome.notify_exit_code,
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
    let has_scholarly_route = config
        .index_provider_routes
        .values()
        .any(|provider| provider == SCHOLARLY_PROVIDER_NAME);
    let concurrency = litradar_domain::validate_index_concurrency(
        config.worker_count,
        config.process_count,
        has_scholarly_route,
    )
    .map_err(|error| LiveIndexError::InvalidConfig(error.to_string()))?;
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
    if config.update && config.full_rescan {
        return Err(LiveIndexError::InvalidConfig(
            "--update cannot be combined with --full-rescan".to_string(),
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
    if has_scholarly_route {
        if !config.scholarly_config.has_crossref_mailto() {
            return Err(LiveIndexError::InvalidConfig(
                "Crossref mailto is required for scholarly indexing".to_string(),
            ));
        }
        if !config.scholarly_config.has_openalex_key() {
            return Err(LiveIndexError::InvalidConfig(
                "OpenAlex API key is required for scholarly indexing".to_string(),
            ));
        }
        if !config.scholarly_config.has_semantic_scholar_key() {
            return Err(LiveIndexError::InvalidConfig(
                "Semantic Scholar API key is required for scholarly indexing".to_string(),
            ));
        }
    }
    tracing::info!(
        event = "index.concurrency.configured",
        component = "index",
        configured_workers = concurrency.worker_count,
        configured_processes = concurrency.process_count,
        configured_aggregate_capacity = concurrency.aggregate_capacity,
        aggregate_limit = litradar_domain::INDEX_AGGREGATE_CONCURRENCY_MAX,
        has_scholarly_route,
    );
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
    input: &CatalogInput,
    batch_id: &str,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let csv_path = &input.path;
    let catalog_name = input.catalog_name.clone();
    let provider_name = input.provider_name.clone();
    let entries = &input.entries;
    let effective_process_count = config.process_count.min(entries.len());
    let effective_source_worker_count = if matches!(
        provider_name.as_str(),
        SCHOLARLY_PROVIDER_NAME | CNKI_PROVIDER_NAME
    ) {
        config.worker_count
    } else {
        1
    };
    tracing::info!(
        event = "index.concurrency.effective",
        component = "index",
        provider = provider_name,
        configured_workers = config.worker_count,
        configured_processes = config.process_count,
        effective_source_workers = effective_source_worker_count,
        effective_processes = effective_process_count,
        effective_aggregate_capacity =
            effective_source_worker_count.saturating_mul(effective_process_count),
        aggregate_limit = litradar_domain::INDEX_AGGREGATE_CONCURRENCY_MAX,
    );
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
    if let Err(error) =
        prepare_catalog_identities(&content, &control, &content_path, &catalog_name, entries)
    {
        let _ = release_lease(&control, &catalog_name, &provider_name, &run_id);
        return Err(error);
    }
    let writer_context = ParentWriterContext {
        catalog_name: catalog_name.clone(),
        provider_name: provider_name.clone(),
        batch_id: batch_id.to_string(),
        run_id: run_id.clone(),
        timestamp: timestamp.clone(),
    };
    let (execution, heartbeat_result) = if uses_worker_processes {
        let prepared = prepare_worker_requests(
            config,
            &control,
            &writer_context,
            run_time.epoch_milliseconds,
            entries,
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
            batch_id: batch_id.to_string(),
            run_id: run_id.clone(),
            timestamp: timestamp.clone(),
            worker_id: 0,
            resume: config.resume,
            mode: requested_sync_mode(config),
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
            let state = read_batch_journal_state(&control, &catalog_name, &provider_name, batch_id);
            let failed = match state {
                Ok(state) => IndexRunMetrics::from_batch_failure(
                    entries.len(),
                    state.completed,
                    state.in_flight,
                ),
                Err(state_error) => {
                    tracing::error!(
                        event = "index.batch.metrics_failed",
                        component = "index",
                        batch_id,
                        catalog = catalog_name,
                        provider = provider_name,
                        error = %state_error,
                    );
                    IndexRunMetrics {
                        journals_total: entries.len(),
                        journals_failed: 1,
                        ..IndexRunMetrics::default()
                    }
                }
            };
            failed.emit_terminal(&run_id, &catalog_name, &provider_name, "all", "failure");
            return Err(error);
        }
    };
    if let Err(error) = heartbeat_result {
        let _ = release_lease(&control, &catalog_name, &provider_name, &run_id);
        return Err(error);
    }
    let finalization = finalize_content_changes(
        &content,
        &content_path,
        &config.project_root,
        &catalog_name,
        &run_id,
        &timestamp,
        config.update,
    );
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

fn requested_sync_mode(config: &LiveIndexConfig) -> IndexSyncMode {
    if config.full_rescan {
        IndexSyncMode::FullRescan
    } else if config.update {
        IndexSyncMode::Incremental
    } else {
        IndexSyncMode::Bootstrap
    }
}

#[allow(clippy::too_many_arguments)]
fn finalize_content_changes(
    content: &Connection,
    content_path: &Path,
    project_root: &Path,
    catalog_name: &str,
    run_id: &str,
    timestamp: &str,
    should_publish_manifest: bool,
) -> Result<(String, Option<PathBuf>), LiveIndexError> {
    optimize_content_db(content).map_err(|source| LiveIndexError::ContentDatabase {
        path: content_path.to_path_buf(),
        source,
    })?;
    let db_name = content_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("index.sqlite")
        .to_string();
    let manifest_path = should_publish_manifest.then(|| {
        project_root
            .join("data")
            .join("push_state")
            .join(format!("{catalog_name}.changes.json"))
    });
    if let Some(path) = manifest_path.as_deref() {
        write_content_change_manifest(content, &db_name, run_id, timestamp, path)?;
    } else {
        discard_content_change_events(content).map_err(|error| {
            LiveIndexError::Worker(format!("content outbox acknowledgement failed: {error}"))
        })?;
    }
    Ok((db_name, manifest_path))
}

fn prepare_catalog_identities(
    content: &Connection,
    control: &Connection,
    content_path: &Path,
    catalog_name: &str,
    entries: &[JournalCatalogEntry],
) -> Result<(), LiveIndexError> {
    let catalog_aliases = entries
        .iter()
        .flat_map(|entry| entry.catalog_aliases.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if has_catalog_alias_sync_state(control, catalog_name, &catalog_aliases)? {
        return Err(LiveIndexError::InvalidConfig(
            LEGACY_ALIAS_SYNC_STATE_MESSAGE.to_string(),
        ));
    }
    reconcile_catalog_identities(content, entries).map_err(|source| {
        LiveIndexError::ContentDatabase {
            path: content_path.to_path_buf(),
            source,
        }
    })
}

fn prepare_entry_sync(
    control: &Connection,
    context: &ParentWriterContext,
    entry: &JournalCatalogEntry,
    mode: IndexSyncMode,
    should_resume: bool,
) -> Result<JournalSyncPreparation, LiveIndexError> {
    Ok(prepare_journal_sync(
        control,
        &context.catalog_name,
        &context.provider_name,
        &entry.catalog_id,
        &context.batch_id,
        &context.run_id,
        mode,
        should_resume,
        &context.timestamp,
    )?)
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
    let mode = requested_sync_mode(config);
    for (journal_ordinal, entry) in entries.iter().cloned().enumerate() {
        match prepare_entry_sync(control, context, &entry, mode, config.resume)? {
            JournalSyncPreparation::Skip => metrics.journals_resumed += 1,
            JournalSyncPreparation::Run(run) => assignments.push(WorkerJournalAssignment {
                journal_ordinal,
                entry,
                mode: run.mode,
                committed_anchor: run.base_anchor,
                traversal_checkpoint: run.traversal_checkpoint,
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
        config.cnki_captcha_token.as_deref(),
        &config.provider_proxy_selection,
        metrics,
        Duration::from_secs(LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS),
        |request_path, worker_id| {
            let mut command = Command::new(&config.application_executable);
            command
                .arg("index")
                .arg("--live-worker-request")
                .arg(request_path)
                .env_remove("LITRADAR_CNKI_CAPTCHA_TOKEN")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            let child = SupervisedChild::spawn(&mut command).map_err(|_| {
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

fn emit_parent_content_commit_failure(worker_id: usize, error: &ContentDatabaseError) {
    let failure =
        LiveIndexWorkerFailure::from_content(LiveIndexWorkerOperation::ContentCommit, error);
    let Some(content_error_kind) = ContentCommitErrorKind::from_error(error) else {
        emit_worker_failure(worker_id, &failure);
        return;
    };
    tracing::error!(
        event = "index.worker.failed",
        component = "index",
        worker_id,
        failure_class = failure.class.as_str(),
        operation = failure.operation.as_str(),
        has_sqlite_code = false,
        is_busy_or_locked = failure.is_busy_or_locked,
        content_error_kind = content_error_kind.as_str(),
    );
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
    child: Option<SupervisedChild>,
    stdin: Option<BufWriter<Box<dyn Write + Send>>>,
    reader: Option<JoinHandle<()>>,
}

/// Process handle and bidirectional protocol streams returned by a worker launcher.
pub(crate) struct LaunchedWorkerProcess {
    child: SupervisedChild,
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
        mut child: SupervisedChild,
        worker_id: usize,
    ) -> Result<Self, LiveIndexError> {
        let Some(writer) = child.take_stdin() else {
            let _ = child.force_kill_and_wait();
            return Err(process_failure(worker_id));
        };
        let Some(reader) = child.take_stdout() else {
            let _ = child.force_kill_and_wait();
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
        child: SupervisedChild,
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
/// * `cnki_captcha_token` - Memory-only domestic CNKI credential.
/// * `provider_proxy_selection` - Memory-only per-Provider proxy selection.
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
    cnki_captcha_token: Option<&str>,
    provider_proxy_selection: &ProviderProxySelection,
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
        let provider_proxy_url =
            provider_proxy_selection.proxy_url_for_provider(&request.provider_name);
        let launched = match bootstrap_worker_process(
            launched,
            request,
            cnki_captcha_token,
            provider_proxy_url.as_deref(),
        ) {
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

fn worker_bootstrap(
    request: &LiveIndexWorkerRequest,
    cnki_captcha_token: Option<&str>,
    provider_proxy_url: Option<&str>,
) -> LiveIndexWorkerBootstrap {
    LiveIndexWorkerBootstrap {
        protocol_version: PROTOCOL_VERSION,
        worker_id: request.worker_id,
        cnki_captcha_token: if request.provider_name == CNKI_PROVIDER_NAME {
            cnki_captcha_token.map(str::to_owned)
        } else {
            None
        },
        provider_proxy_url: provider_proxy_url.map(str::to_owned),
    }
}

fn bootstrap_worker_process(
    mut launched: LaunchedWorkerProcess,
    request: &LiveIndexWorkerRequest,
    cnki_captcha_token: Option<&str>,
    provider_proxy_url: Option<&str>,
) -> Result<LaunchedWorkerProcess, LiveIndexError> {
    let bootstrap = worker_bootstrap(request, cnki_captcha_token, provider_proxy_url);
    if write_message(&mut launched.writer, &bootstrap).is_err() {
        let _ = launched.child.force_kill_and_wait();
        return Err(protocol_failure(request.worker_id));
    }
    Ok(launched)
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
            let is_complete = matches!(&batch.progress, ProviderProgress::Complete { .. });
            let content_revision = format!(
                "{}:{}:{}",
                context.run_id, assignment.entry.catalog_id, page_index
            );
            let outcome = commit_content_then_progress(
                control,
                &context.catalog_name,
                &context.provider_name,
                &assignment.entry.catalog_id,
                &context.batch_id,
                &context.run_id,
                assignment.mode,
                assignment.committed_anchor.as_deref(),
                &batch.progress,
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
                match &error {
                    LiveIndexError::Commit(ContentCheckpointCommitError::Content(source)) => {
                        emit_parent_content_commit_failure(pipe_worker_id, source);
                    }
                    _ => emit_worker_failure(
                        pipe_worker_id,
                        &LiveIndexWorkerFailure::from_error(&error),
                    ),
                }
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
            let _ = child.force_kill_and_wait();
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
        config.cnki_captcha_token.clone(),
        config
            .provider_proxy_selection
            .for_provider(&request.provider_name),
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
    let execution =
        read_worker_bootstrap(request, reader).and_then(|(cnki_captcha_token, provider_proxy)| {
            fetch_worker_assignments(
                request,
                cnki_captcha_token,
                provider_proxy,
                reader,
                writer,
                &mut sequence,
            )
        });
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

fn read_worker_bootstrap(
    request: &LiveIndexWorkerRequest,
    reader: &mut impl Read,
) -> Result<(Option<String>, ProviderProxy), LiveIndexError> {
    let bootstrap: LiveIndexWorkerBootstrap = read_message(reader)
        .map_err(|_| LiveIndexError::Worker(WORKER_PROTOCOL_FAILURE_MESSAGE.to_string()))?;
    if bootstrap.protocol_version != PROTOCOL_VERSION
        || bootstrap.worker_id != request.worker_id
        || (request.provider_name != CNKI_PROVIDER_NAME && bootstrap.cnki_captcha_token.is_some())
    {
        return Err(LiveIndexError::InvalidConfig(
            "worker bootstrap is invalid".to_string(),
        ));
    }
    let provider_proxy = bootstrap
        .provider_proxy_url
        .map(ProviderProxy::explicit)
        .transpose()
        .map_err(|_| LiveIndexError::InvalidConfig("worker bootstrap is invalid".to_string()))?
        .unwrap_or_else(ProviderProxy::direct);
    Ok((bootstrap.cnki_captcha_token, provider_proxy))
}

fn fetch_worker_assignments(
    request: &LiveIndexWorkerRequest,
    cnki_captcha_token: Option<String>,
    provider_proxy: ProviderProxy,
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
    litradar_domain::validate_index_concurrency(
        request.source_worker_count,
        request.process_count,
        request.provider_name == SCHOLARLY_PROVIDER_NAME,
    )
    .map_err(|error| LiveIndexError::InvalidConfig(error.to_string()))?;
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
        cnki_captcha_token,
        provider_proxy,
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
        let mut provider_checkpoint = assignment.traversal_checkpoint.clone();
        let mut seen_checkpoints = BTreeSet::new();
        if let Some(value) = &provider_checkpoint {
            seen_checkpoints.insert(value.clone());
        }
        for page_index in 0..MAX_PROVIDER_PAGES_PER_JOURNAL {
            let batch = provider.fetch(
                &assignment.entry,
                IndexFetchContext {
                    mode: assignment.mode,
                    committed_anchor: assignment.committed_anchor.as_deref(),
                    traversal_checkpoint: provider_checkpoint.as_deref(),
                },
            )?;
            if batch.catalog_id != assignment.entry.catalog_id {
                return Err(LiveIndexError::InvalidConfig(
                    "provider batch catalog identity is invalid".to_string(),
                ));
            }
            let progress = batch.progress.clone();
            let is_complete = matches!(progress, ProviderProgress::Complete { .. });
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
            let ProviderProgress::Continue { checkpoint } = progress else {
                unreachable!("complete progress returned above")
            };
            if !seen_checkpoints.insert(checkpoint.clone()) {
                return Err(LiveIndexError::InvalidConfig(
                    "index provider returned a repeated checkpoint".to_string(),
                ));
            }
            provider_checkpoint = Some(checkpoint);
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
    cnki_captcha_token: Option<String>,
    provider_proxy: ProviderProxy,
) -> Result<ProviderRegistration, LiveIndexError> {
    match provider_name {
        SCHOLARLY_PROVIDER_NAME => {
            let has_semantic_scholar_key = scholarly_config.has_semantic_scholar_key();
            let transport = LiveScholarlyTransport::new_with_openalex_workers_and_proxy(
                scholarly_config,
                source_worker_count,
                provider_proxy,
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
        CNKI_OVERSEA_PROVIDER_NAME => {
            let transport = LiveCnkiTransport::new_with_proxy(
                LiveCnkiConfig { timeout_seconds },
                provider_proxy,
            )
            .map_err(|_| {
                LiveIndexError::ProviderSetup(
                    "CNKI indexing provider could not initialize".to_string(),
                )
            })?;
            Ok(cnki_oversea_index_registration(transport)?)
        }
        CNKI_PROVIDER_NAME => {
            let transport = LiveDomesticCnkiTransport::new_with_proxy(
                LiveDomesticCnkiConfig {
                    timeout_seconds,
                    captcha_token: cnki_captcha_token,
                },
                provider_proxy,
            )
            .map_err(|_| {
                LiveIndexError::ProviderSetup(
                    "domestic CNKI indexing provider could not initialize".to_string(),
                )
            })?;
            Ok(cnki_index_registration_with_workers(
                transport,
                source_worker_count,
            )?)
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
    let writer_context = ParentWriterContext {
        catalog_name: request.catalog_name.clone(),
        provider_name: request.provider_name.clone(),
        batch_id: request.batch_id.clone(),
        run_id: request.run_id.clone(),
        timestamp: request.timestamp.clone(),
    };
    for (journal_ordinal, entry) in request.entries.iter().enumerate() {
        heartbeat_lease(
            control,
            &request.catalog_name,
            &request.provider_name,
            &request.run_id,
            LiveRunTime::now().epoch_seconds,
        )
        .map_err(|error| LiveIndexError::Heartbeat(error.to_string()))?;
        let run = match prepare_entry_sync(
            control,
            &writer_context,
            entry,
            request.mode,
            request.resume,
        )? {
            JournalSyncPreparation::Skip => {
                metrics.journals_resumed += 1;
                continue;
            }
            JournalSyncPreparation::Run(run) => run,
        };
        let mut provider_checkpoint = run.traversal_checkpoint.clone();
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
            let batch = provider
                .fetch(
                    entry,
                    IndexFetchContext {
                        mode: run.mode,
                        committed_anchor: run.base_anchor.as_deref(),
                        traversal_checkpoint: provider_checkpoint.as_deref(),
                    },
                )
                .map_err(|error| {
                    tracing::error!(
                        event = "index.provider.failed",
                        component = "index",
                        provider = request.provider_name,
                        journal_ordinal = journal_ordinal + 1,
                        catalog_id = entry.catalog_id,
                        failure_kind = ?error.kind(),
                    );
                    LiveIndexError::Provider(error)
                })?;
            let progress = batch.progress.clone();
            let content_revision =
                format!("{}:{}:{}", request.run_id, entry.catalog_id, page_index);
            let outcome = commit_content_then_progress(
                control,
                &request.catalog_name,
                &request.provider_name,
                &entry.catalog_id,
                &request.batch_id,
                &run.run_id,
                run.mode,
                run.base_anchor.as_deref(),
                &progress,
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
            if matches!(progress, ProviderProgress::Complete { .. }) {
                metrics.journals_succeeded += 1;
                break;
            }
            let ProviderProgress::Continue { checkpoint } = progress else {
                unreachable!("complete progress returned above")
            };
            if !seen_checkpoints.insert(checkpoint.clone()) {
                return Err(LiveIndexError::InvalidConfig(
                    "index provider returned a repeated checkpoint".to_string(),
                ));
            }
            provider_checkpoint = Some(checkpoint);
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

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn run_notify_for_manifest(
    config: &LiveIndexConfig,
    db_name: &str,
    manifest_path: &Path,
) -> Result<i32, LiveIndexError> {
    let mut command = Command::new(&config.application_executable);
    command
        .arg("notify")
        .arg("--secret-key-file")
        .arg(&config.secret_key_file)
        .arg("--db")
        .arg(db_name)
        .arg("--changes-file")
        .arg(manifest_path)
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
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use litradar_domain::{
        ArticleAuthorDraft, ArticleDraft, IndexFetchContext, IndexSyncMode, IssueDraft,
        JournalCatalogEntry, JournalDraft, JournalRankings, ProviderBatch, ProviderProgress,
        INDEX_AGGREGATE_CONCURRENCY_MAX, SCHOLARLY_WORKER_COUNT_MAX,
    };
    use litradar_provider::conformance::ContractViolation;
    use litradar_provider::{IndexContentProvider, ProviderError, ProviderErrorKind};
    use rusqlite::{Connection, ErrorCode};
    use tempfile::tempdir;
    use tracing_subscriber::fmt::MakeWriter;

    use super::{
        emit_parent_content_commit_failure, emit_worker_failure,
        fetch_worker_assignments_with_provider, finalize_content_changes,
        index_entries_with_provider, prepare_catalog_identities, prepare_worker_requests,
        read_worker_bootstrap, requested_sync_mode, run_batch_catalogs_with, run_live_index,
        run_live_index_worker_with_io, run_worker_processes_with_launcher, validate_live_config,
        worker_bootstrap, worker_failure_error, ContentCommitErrorKind, DirectIndexRequest,
        LaunchedWorkerProcess, LeaseHeartbeat, LiveIndexConfig, LiveIndexError,
        LiveIndexWorkerBootstrap, LiveIndexWorkerFailure, LiveIndexWorkerFailureClass,
        LiveIndexWorkerOperation, LiveIndexWorkerRequest, LiveRunTime, ParentWriterContext,
        ProviderProxySelection, SupervisedChild, CNKI_PROVIDER_NAME,
    };
    use crate::batch::{
        admit_batch, complete_catalog, init_batch_db, release_batch_lease,
        transition_catalog_phase, BatchAdmission, BatchCatalogOutcome, BatchCatalogPhase,
        CatalogInput, CatalogSelection, IndexBatchRequest,
    };
    use crate::control::{
        acquire_lease, advance_run_checkpoint as advance_run_checkpoint_for_batch,
        commit_content_then_progress as commit_content_then_progress_for_batch,
        complete_sync_run as complete_sync_run_for_batch, open_control_db,
        prepare_journal_sync as prepare_journal_sync_for_batch, read_run_checkpoint,
        read_sync_anchor, release_lease, ContentCheckpointCommitError, ControlDatabaseError,
        JournalSyncPreparation, ProviderRunCheckpoint,
    };
    use crate::identity::{ArticleIdentityError, ArticleMergeError};
    use crate::schema::{open_content_db, write_content_batch, ContentDatabaseError};
    use crate::stats::IndexRunMetrics;
    use crate::worker_protocol::{
        read_message, write_message, ParentMessage, WorkerJournalAssignment, WorkerMessage,
        PROTOCOL_VERSION,
    };

    const TEST_BATCH_ID: &str = "batch-current";
    const PREVIOUS_BATCH_ID: &str = "batch-previous";

    #[allow(clippy::too_many_arguments)]
    fn prepare_journal_sync(
        connection: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        run_id: &str,
        mode: IndexSyncMode,
        should_resume: bool,
        updated_at: &str,
    ) -> Result<JournalSyncPreparation, ControlDatabaseError> {
        prepare_journal_sync_for_batch(
            connection,
            catalog_name,
            provider_name,
            catalog_id,
            TEST_BATCH_ID,
            run_id,
            mode,
            should_resume,
            updated_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_run_checkpoint(
        connection: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        run_id: &str,
        mode: IndexSyncMode,
        base_anchor: Option<&str>,
        checkpoint: &str,
        updated_at: &str,
    ) -> Result<(), ControlDatabaseError> {
        advance_run_checkpoint_for_batch(
            connection,
            catalog_name,
            provider_name,
            catalog_id,
            TEST_BATCH_ID,
            run_id,
            mode,
            base_anchor,
            checkpoint,
            updated_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_content_then_progress<Outcome, WriteContent>(
        control_connection: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        run_id: &str,
        mode: IndexSyncMode,
        base_anchor: Option<&str>,
        progress: &ProviderProgress,
        updated_at: &str,
        write_content: WriteContent,
    ) -> Result<Outcome, ContentCheckpointCommitError>
    where
        WriteContent: FnOnce() -> Result<Outcome, ContentDatabaseError>,
    {
        commit_content_then_progress_for_batch(
            control_connection,
            catalog_name,
            provider_name,
            catalog_id,
            TEST_BATCH_ID,
            run_id,
            mode,
            base_anchor,
            progress,
            updated_at,
            write_content,
        )
    }

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

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fixture write failed",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fixture flush failed",
            ))
        }
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
            context: IndexFetchContext<'_>,
        ) -> Result<ProviderBatch, ProviderError> {
            assert!(context.traversal_checkpoint.is_none());
            *self.calls.lock().expect("call count should lock") += 1;
            Ok(canonical_batch(catalog))
        }
    }

    struct FailingProvider;

    impl IndexContentProvider for FailingProvider {
        fn fetch(
            &self,
            _catalog: &JournalCatalogEntry,
            _context: IndexFetchContext<'_>,
        ) -> Result<ProviderBatch, ProviderError> {
            Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "sensitive provider diagnostic",
            ))
        }
    }

    struct TwoPageProvider {
        second_fetch: mpsc::Sender<()>,
    }

    impl IndexContentProvider for TwoPageProvider {
        fn fetch(
            &self,
            catalog: &JournalCatalogEntry,
            context: IndexFetchContext<'_>,
        ) -> Result<ProviderBatch, ProviderError> {
            let mut batch = canonical_batch(catalog);
            match context.traversal_checkpoint {
                None => {
                    batch.progress = ProviderProgress::Continue {
                        checkpoint: "cursor-1".to_string(),
                    };
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ObservedFetchContext {
        mode: IndexSyncMode,
        committed_anchor: Option<String>,
        traversal_checkpoint: Option<String>,
    }

    struct RecordingProvider {
        observations: Mutex<Vec<ObservedFetchContext>>,
        next_anchor: Option<String>,
    }

    impl RecordingProvider {
        fn new(next_anchor: Option<&str>) -> Self {
            Self {
                observations: Mutex::new(Vec::new()),
                next_anchor: next_anchor.map(str::to_string),
            }
        }
    }

    impl IndexContentProvider for RecordingProvider {
        fn fetch(
            &self,
            catalog: &JournalCatalogEntry,
            context: IndexFetchContext<'_>,
        ) -> Result<ProviderBatch, ProviderError> {
            self.observations
                .lock()
                .expect("observations should lock")
                .push(ObservedFetchContext {
                    mode: context.mode,
                    committed_anchor: context.committed_anchor.map(str::to_string),
                    traversal_checkpoint: context.traversal_checkpoint.map(str::to_string),
                });
            let mut batch = canonical_batch(catalog);
            batch.progress = ProviderProgress::Complete {
                next_anchor: self.next_anchor.clone(),
            };
            Ok(batch)
        }
    }

    struct InterruptingProvider {
        call_count: Mutex<usize>,
    }

    impl InterruptingProvider {
        fn new() -> Self {
            Self {
                call_count: Mutex::new(0),
            }
        }
    }

    impl IndexContentProvider for InterruptingProvider {
        fn fetch(
            &self,
            catalog: &JournalCatalogEntry,
            context: IndexFetchContext<'_>,
        ) -> Result<ProviderBatch, ProviderError> {
            let mut call_count = self.call_count.lock().expect("call count should lock");
            *call_count += 1;
            match *call_count {
                1 => {
                    assert_eq!(context.committed_anchor, Some("anchor-old"));
                    assert_eq!(context.traversal_checkpoint, None);
                    let mut batch = canonical_batch(catalog);
                    batch.progress = ProviderProgress::Continue {
                        checkpoint: "cursor-after-head".to_string(),
                    };
                    Ok(batch)
                }
                2 => Err(ProviderError::new(
                    ProviderErrorKind::TemporarilyUnavailable,
                    "fixture interruption",
                )),
                _ => panic!("interrupting provider received an unexpected fetch"),
            }
        }
    }

    fn catalog(id: &str) -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: id.to_string(),
            catalog_aliases: Vec::new(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
    }

    fn batch_input(file_name: &str, provider_name: &str, digest_byte: u8) -> CatalogInput {
        let catalog_name = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("fixture catalog should have a stem")
            .to_string();
        CatalogInput {
            path: Path::new("data").join("meta").join(file_name),
            file_name: file_name.to_string(),
            catalog_name: catalog_name.clone(),
            csv_sha256: format!("{digest_byte:02x}").repeat(32),
            provider_name: provider_name.to_string(),
            entries: vec![catalog(&format!("{catalog_name}-journal"))],
        }
    }

    fn environment_catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "issn-1472-3409".to_string(),
            catalog_aliases: vec!["issn-0308-518x".to_string()],
            title: "Environment and Planning A: Economy and Space".to_string(),
            issn: Some("0308-518X".to_string()),
            eissn: Some("1472-3409".to_string()),
            all_issns: vec!["1472-3409".to_string(), "0308-518X".to_string()],
            title_aliases: vec!["Environment and Planning A".to_string()],
            area: Some("Regional, Environmental & Resource Studies".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn legacy_environment_catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "issn-0308-518x".to_string(),
            catalog_aliases: Vec::new(),
            title: "Environment and Planning A".to_string(),
            issn: Some("0308-518X".to_string()),
            eissn: None,
            all_issns: vec!["0308-518X".to_string()],
            title_aliases: Vec::new(),
            area: Some("Legacy Area".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn canonical_batch_for_catalog(catalog: &JournalCatalogEntry) -> ProviderBatch {
        let mut batch = canonical_batch(catalog);
        for issue in &mut batch.issues {
            issue.catalog_id.clone_from(&catalog.catalog_id);
        }
        for article in &mut batch.articles {
            article.catalog_id.clone_from(&catalog.catalog_id);
        }
        batch
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
                retraction_dois: Vec::new(),
            }],
            progress: ProviderProgress::Complete { next_anchor: None },
        }
    }

    fn direct_request(provider_name: &str, run_id: &str) -> DirectIndexRequest {
        DirectIndexRequest {
            catalog_name: "chinese_journals".to_string(),
            provider_name: provider_name.to_string(),
            batch_id: TEST_BATCH_ID.to_string(),
            run_id: run_id.to_string(),
            timestamp: "2026-07-18T00:00:00Z".to_string(),
            worker_id: 0,
            resume: true,
            mode: IndexSyncMode::Bootstrap,
            entries: vec![catalog("journal-1")],
        }
    }

    fn prepared_run(preparation: JournalSyncPreparation) -> ProviderRunCheckpoint {
        match preparation {
            JournalSyncPreparation::Run(run) => run,
            JournalSyncPreparation::Skip => panic!("fixture journal should not skip"),
        }
    }

    fn seed_completed_sync(
        control: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        committed_anchor: Option<&str>,
        timestamp: &str,
    ) {
        seed_completed_sync_for_batch(
            control,
            catalog_name,
            provider_name,
            catalog_id,
            PREVIOUS_BATCH_ID,
            committed_anchor,
            timestamp,
        );
    }

    fn seed_completed_sync_for_batch(
        control: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        batch_id: &str,
        committed_anchor: Option<&str>,
        timestamp: &str,
    ) {
        let run = prepared_run(
            prepare_journal_sync_for_batch(
                control,
                catalog_name,
                provider_name,
                catalog_id,
                batch_id,
                "fixture-complete-run",
                IndexSyncMode::Incremental,
                false,
                timestamp,
            )
            .expect("fixture completion run should prepare"),
        );
        complete_sync_run_for_batch(
            control,
            catalog_name,
            provider_name,
            catalog_id,
            batch_id,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            committed_anchor,
            timestamp,
        )
        .expect("fixture completion should commit");
    }

    fn seed_incremental_traversal(
        control: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        cursor: &str,
        timestamp: &str,
    ) {
        seed_traversal(
            control,
            catalog_name,
            provider_name,
            catalog_id,
            IndexSyncMode::Incremental,
            cursor,
            timestamp,
        );
    }

    fn seed_traversal(
        control: &Connection,
        catalog_name: &str,
        provider_name: &str,
        catalog_id: &str,
        mode: IndexSyncMode,
        cursor: &str,
        timestamp: &str,
    ) {
        seed_completed_sync(
            control,
            catalog_name,
            provider_name,
            catalog_id,
            Some("anchor-old"),
            timestamp,
        );
        let run = prepared_run(
            prepare_journal_sync(
                control,
                catalog_name,
                provider_name,
                catalog_id,
                "previous-run",
                mode,
                true,
                timestamp,
            )
            .expect("fixture traversal should prepare"),
        );
        advance_run_checkpoint(
            control,
            catalog_name,
            provider_name,
            catalog_id,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            cursor,
            timestamp,
        )
        .expect("fixture traversal should advance");
    }

    #[test]
    fn direct_provider_failure_reports_catalog_and_kind() {
        let directory = tempdir().expect("temporary directory should create");
        let content = open_content_db(directory.path().join("content.sqlite"))
            .expect("content database should open");
        let control = open_control_db(directory.path().join("control.sqlite"))
            .expect("control database should open");
        let mut request = direct_request("cnki", "run-direct-provider-failure");
        request.entries[0].catalog_id = "issn-0253-9772".to_string();
        acquire_lease(
            &control,
            &request.catalog_name,
            &request.provider_name,
            &request.run_id,
            LiveRunTime::now().epoch_seconds,
        )
        .expect("lease should acquire");
        let captured = CapturedLogs::default();

        let error = tracing::subscriber::with_default(captured.subscriber(), || {
            index_entries_with_provider(&content, &control, &FailingProvider, &request)
        })
        .expect_err("provider failure should stop the direct run");
        let event = captured
            .text()
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("event should parse")
            })
            .find(|event| event["event"] == "index.provider.failed")
            .expect("provider failure event should be emitted");

        assert!(matches!(error, LiveIndexError::Provider(_)));
        assert_eq!(event["provider"], "cnki");
        assert_eq!(event["journal_ordinal"], 1);
        assert_eq!(event["catalog_id"], "issn-0253-9772");
        assert_eq!(event["failure_kind"], "NotFound");
        assert!(!captured.text().contains("sensitive provider diagnostic"));
    }

    #[test]
    fn direct_and_worker_paths_resume_the_same_frozen_context_for_each_mode() {
        let directory = tempdir().expect("temporary directory should create");
        for (label, mode) in [
            ("incremental", IndexSyncMode::Incremental),
            ("full-rescan", IndexSyncMode::FullRescan),
        ] {
            let direct_content = open_content_db(
                directory
                    .path()
                    .join(format!("{label}-direct-content.sqlite")),
            )
            .expect("direct content should open");
            let direct_control = open_control_db(
                directory
                    .path()
                    .join(format!("{label}-direct-control.sqlite")),
            )
            .expect("direct control should open");
            seed_traversal(
                &direct_control,
                "chinese_journals",
                "provider-a",
                "journal-1",
                mode,
                "cursor-frozen",
                "2026-07-18T00:00:00Z",
            );
            let mut direct = direct_request("provider-a", &format!("{label}-direct-current-run"));
            direct.mode = mode;
            acquire_lease(
                &direct_control,
                &direct.catalog_name,
                &direct.provider_name,
                &direct.run_id,
                LiveRunTime::now().epoch_seconds,
            )
            .expect("direct lease should acquire");
            let provider = RecordingProvider::new(Some("anchor-new"));
            index_entries_with_provider(&direct_content, &direct_control, &provider, &direct)
                .expect("direct frozen run should complete");
            let direct_context = provider
                .observations
                .lock()
                .expect("direct observations should lock")[0]
                .clone();

            let worker_control = open_control_db(
                directory
                    .path()
                    .join(format!("{label}-worker-control.sqlite")),
            )
            .expect("worker control should open");
            seed_traversal(
                &worker_control,
                "chinese_journals",
                "provider-a",
                "journal-1",
                mode,
                "cursor-frozen",
                "2026-07-18T00:00:00Z",
            );
            let worker_context = ParentWriterContext {
                catalog_name: "chinese_journals".to_string(),
                provider_name: "provider-a".to_string(),
                batch_id: TEST_BATCH_ID.to_string(),
                run_id: format!("{label}-worker-current-run"),
                timestamp: "2026-07-18T00:01:00Z".to_string(),
            };
            let mut config = worker_test_config("provider-a", None);
            config.update = mode == IndexSyncMode::Incremental;
            config.full_rescan = mode == IndexSyncMode::FullRescan;
            let (requests, metrics) = prepare_worker_requests(
                &config,
                &worker_control,
                &worker_context,
                0,
                &[catalog("journal-1")],
            )
            .expect("worker frozen run should prepare");
            assert_eq!(metrics.journals_resumed, 0);
            let assignment = &requests[0].assignments[0];
            assert_eq!(
                direct_context,
                ObservedFetchContext {
                    mode: assignment.mode,
                    committed_anchor: assignment.committed_anchor.clone(),
                    traversal_checkpoint: assignment.traversal_checkpoint.clone(),
                }
            );
            assert_eq!(direct_context.mode, mode);
            assert_eq!(
                direct_context.committed_anchor.as_deref(),
                Some("anchor-old")
            );
            assert_eq!(
                direct_context.traversal_checkpoint.as_deref(),
                Some("cursor-frozen")
            );
        }
    }

    #[test]
    fn live_config_selects_exact_sync_modes_and_rejects_conflicts() {
        let mut config = worker_test_config("provider-a", None);
        assert_eq!(requested_sync_mode(&config), IndexSyncMode::Bootstrap);

        config.update = true;
        assert_eq!(requested_sync_mode(&config), IndexSyncMode::Incremental);

        config.update = false;
        config.full_rescan = true;
        assert_eq!(requested_sync_mode(&config), IndexSyncMode::FullRescan);

        config.update = true;
        assert!(matches!(
            validate_live_config(&config),
            Err(LiveIndexError::InvalidConfig(message))
                if message == "--update cannot be combined with --full-rescan"
        ));
    }

    #[test]
    fn no_resume_replaces_traversal_without_discarding_the_committed_anchor() {
        let directory = tempdir().expect("temporary directory should create");
        let content =
            open_content_db(directory.path().join("content.sqlite")).expect("content should open");
        let control =
            open_control_db(directory.path().join("control.sqlite")).expect("control should open");
        seed_incremental_traversal(
            &control,
            "chinese_journals",
            "provider-a",
            "journal-1",
            "cursor-discarded",
            "2026-07-18T00:00:00Z",
        );
        let mut request = direct_request("provider-a", "no-resume-run");
        request.mode = IndexSyncMode::Incremental;
        request.resume = false;
        acquire_lease(
            &control,
            &request.catalog_name,
            &request.provider_name,
            &request.run_id,
            LiveRunTime::now().epoch_seconds,
        )
        .expect("lease should acquire");
        let provider = RecordingProvider::new(Some("anchor-new"));

        index_entries_with_provider(&content, &control, &provider, &request)
            .expect("no-resume run should complete");
        let observations = provider
            .observations
            .lock()
            .expect("observations should lock");

        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].committed_anchor.as_deref(),
            Some("anchor-old")
        );
        assert_eq!(observations[0].traversal_checkpoint, None);
    }

    #[test]
    fn matching_resume_rejects_a_different_frozen_sync_mode() {
        let directory = tempdir().expect("temporary directory should create");
        let control =
            open_control_db(directory.path().join("control.sqlite")).expect("control should open");
        seed_incremental_traversal(
            &control,
            "chinese_journals",
            "provider-a",
            "journal-1",
            "cursor-incremental",
            "2026-07-18T00:00:00Z",
        );
        let context = ParentWriterContext {
            catalog_name: "chinese_journals".to_string(),
            provider_name: "provider-a".to_string(),
            batch_id: TEST_BATCH_ID.to_string(),
            run_id: "full-rescan-run".to_string(),
            timestamp: "2026-07-18T00:01:00Z".to_string(),
        };
        let mut config = worker_test_config("provider-a", None);
        config.full_rescan = true;

        let error =
            prepare_worker_requests(&config, &control, &context, 0, &[catalog("journal-1")])
                .expect_err("different frozen mode must fail closed");

        assert!(matches!(
            error,
            LiveIndexError::Control(ControlDatabaseError::RunModeMismatch {
                stored: IndexSyncMode::Incremental,
                requested: IndexSyncMode::FullRescan,
            })
        ));
    }

    #[test]
    fn late_catalog_retry_calls_only_the_first_unfinished_catalog() {
        let batch_connection = Connection::open_in_memory().expect("batch database should open");
        init_batch_db(&batch_connection).expect("batch schema should initialize");
        let request = IndexBatchRequest::new(
            vec![
                batch_input("ccf.csv", "provider-a", 1),
                batch_input("chinese.csv", "provider-a", 2),
                batch_input("english.csv", "provider-a", 3),
            ],
            CatalogSelection::All,
            IndexSyncMode::Incremental,
            20,
            false,
            false,
        )
        .expect("batch request should build");
        let now = LiveRunTime::now().epoch_seconds;
        let first_owner = "first-owner";
        let first = match admit_batch(&batch_connection, &request, true, first_owner, now)
            .expect("batch should create")
        {
            BatchAdmission::Ready(batch) => batch,
            BatchAdmission::Abandoning(_) => panic!("new batch should be ready"),
        };
        for ordinal in 0..2 {
            transition_catalog_phase(
                &batch_connection,
                &first.batch_id,
                first_owner,
                ordinal,
                BatchCatalogPhase::Indexing,
                now,
            )
            .expect("catalog should enter indexing");
            complete_catalog(
                &batch_connection,
                &first.batch_id,
                first_owner,
                ordinal,
                &BatchCatalogOutcome {
                    run_id: format!("completed-{ordinal}"),
                    journal_count: 1,
                    written_article_count: 1,
                    source_attempt_count: 1,
                    manifest_path: None,
                    notify_exit_code: None,
                },
                now,
            )
            .expect("catalog should complete");
        }
        release_batch_lease(&batch_connection, &first.batch_id, first_owner)
            .expect("failed invocation should release its lease");
        let retry_owner = "retry-owner";
        let retry = match admit_batch(&batch_connection, &request, true, retry_owner, now)
            .expect("compatible retry should resume")
        {
            BatchAdmission::Ready(batch) => batch,
            BatchAdmission::Abandoning(_) => panic!("compatible batch should be ready"),
        };
        let mut calls = Vec::new();
        let mut config = worker_test_config("provider-a", None);
        let directory = tempdir().expect("temporary project should create");
        config.project_root = directory.path().to_path_buf();

        let error = run_batch_catalogs_with(
            &config,
            &batch_connection,
            &retry,
            &request,
            |_, input, _| {
                calls.push(input.catalog_name.clone());
                Err(LiveIndexError::ProviderSetup(
                    "fixture unfinished catalog".to_string(),
                ))
            },
        )
        .expect_err("unfinished English catalog should retain the failure");

        assert!(matches!(error, LiveIndexError::ProviderSetup(_)));
        assert_eq!(calls, vec!["english"]);
    }

    #[test]
    fn manifest_failure_retains_outbox_and_full_rescan_acknowledges_without_a_manifest() {
        let directory = tempdir().expect("temporary directory should create");
        let content_path = directory.path().join("catalog.sqlite");
        let content = open_content_db(&content_path).expect("content should open");
        let entry = catalog("journal-1");
        write_content_batch(
            &content,
            &entry,
            &canonical_batch(&entry),
            "fixture-run",
            "2026-07-18T00:00:00Z",
        )
        .expect("fixture content should write");
        std::fs::create_dir_all(directory.path().join("data"))
            .expect("data directory should create");
        std::fs::write(directory.path().join("data").join("push_state"), b"blocked")
            .expect("blocking manifest parent should write");

        let error = finalize_content_changes(
            &content,
            &content_path,
            directory.path(),
            "catalog",
            "fixture-run",
            "2026-07-18T00:00:00Z",
            true,
        )
        .expect_err("manifest write should fail");
        assert!(matches!(error, LiveIndexError::Io(_)));
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM article_change_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("outbox count should read"),
            1
        );

        let (_, manifest_path) = finalize_content_changes(
            &content,
            &content_path,
            directory.path(),
            "catalog",
            "full-rescan-run",
            "2026-07-18T00:01:00Z",
            false,
        )
        .expect("full rescan finalization should acknowledge without a manifest");
        assert_eq!(manifest_path, None);
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM article_change_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("acknowledged outbox count should read"),
            0
        );
    }

    #[test]
    fn interrupted_update_reuses_frozen_anchor_after_new_content_commits() {
        let directory = tempdir().expect("temporary directory should create");
        let content =
            open_content_db(directory.path().join("content.sqlite")).expect("content should open");
        let control =
            open_control_db(directory.path().join("control.sqlite")).expect("control should open");
        seed_completed_sync(
            &control,
            "chinese_journals",
            "provider-a",
            "journal-1",
            Some("anchor-old"),
            "2026-07-18T00:00:00Z",
        );
        let mut interrupted = direct_request("provider-a", "interrupted-run");
        interrupted.mode = IndexSyncMode::Incremental;
        let now = LiveRunTime::now().epoch_seconds;
        acquire_lease(
            &control,
            &interrupted.catalog_name,
            &interrupted.provider_name,
            &interrupted.run_id,
            now,
        )
        .expect("interrupted lease should acquire");
        let error = index_entries_with_provider(
            &content,
            &control,
            &InterruptingProvider::new(),
            &interrupted,
        )
        .expect_err("second Provider page should interrupt the run");
        assert!(matches!(error, LiveIndexError::Provider(_)));
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM articles", [], |row| row
                    .get::<_, i64>(0))
                .expect("committed article count should read"),
            1
        );
        let frozen = read_run_checkpoint(
            &control,
            &interrupted.catalog_name,
            &interrupted.provider_name,
            "journal-1",
        )
        .expect("frozen run should read")
        .expect("frozen run should remain");
        assert_eq!(frozen.base_anchor.as_deref(), Some("anchor-old"));
        assert_eq!(
            frozen.traversal_checkpoint.as_deref(),
            Some("cursor-after-head")
        );
        release_lease(
            &control,
            &interrupted.catalog_name,
            &interrupted.provider_name,
            &interrupted.run_id,
        )
        .expect("interrupted lease should release");

        let mut retry = direct_request("provider-a", "retry-run");
        retry.mode = IndexSyncMode::Incremental;
        acquire_lease(
            &control,
            &retry.catalog_name,
            &retry.provider_name,
            &retry.run_id,
            now,
        )
        .expect("retry lease should acquire");
        let retry_provider = RecordingProvider::new(Some("anchor-new"));
        index_entries_with_provider(&content, &control, &retry_provider, &retry)
            .expect("retry should complete from frozen traversal");
        let observations = retry_provider
            .observations
            .lock()
            .expect("retry observations should lock");
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].committed_anchor.as_deref(),
            Some("anchor-old")
        );
        assert_eq!(
            observations[0].traversal_checkpoint.as_deref(),
            Some("cursor-after-head")
        );
    }

    #[test]
    fn completed_sync_reconciles_catalog_identity_without_provider_fetch() {
        let directory = tempdir().expect("temporary directory should create");
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let mut original = environment_catalog();
        original.catalog_aliases.clear();
        original.title = "Environment and Planning A".to_string();
        original.title_aliases.clear();
        original.issn = None;
        original.all_issns = vec!["1472-3409".to_string()];
        original.area = Some("Legacy Area".to_string());
        write_content_batch(
            &content,
            &original,
            &canonical_batch_for_catalog(&original),
            "english:environment:seed",
            "2026-07-20T00:00:00Z",
        )
        .expect("original canonical content should write");
        let request = DirectIndexRequest {
            catalog_name: "english_journals".to_string(),
            provider_name: "provider-a".to_string(),
            batch_id: TEST_BATCH_ID.to_string(),
            run_id: "run-complete-reconcile".to_string(),
            timestamp: "2026-07-20T00:00:00Z".to_string(),
            worker_id: 0,
            resume: true,
            mode: IndexSyncMode::Bootstrap,
            entries: vec![environment_catalog()],
        };
        seed_completed_sync_for_batch(
            &control,
            &request.catalog_name,
            &request.provider_name,
            &request.entries[0].catalog_id,
            TEST_BATCH_ID,
            None,
            &request.timestamp,
        );
        acquire_lease(
            &control,
            &request.catalog_name,
            &request.provider_name,
            &request.run_id,
            LiveRunTime::now().epoch_seconds,
        )
        .expect("lease should acquire");

        prepare_catalog_identities(
            &content,
            &control,
            &content_path,
            &request.catalog_name,
            &request.entries,
        )
        .expect("catalog identity should reconcile before resume");
        let provider = StaticProvider::new();
        let metrics = index_entries_with_provider(&content, &control, &provider, &request)
            .expect("complete checkpoint should resume");

        assert_eq!(metrics.journals_resumed, 1);
        assert_eq!(metrics.pages_committed, 0);
        assert_eq!(*provider.calls.lock().expect("call count should lock"), 0);
        let reconciled = content
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM journal_identity_keys
                      WHERE canonical_catalog_id = 'issn-1472-3409'),
                     (SELECT title FROM journals WHERE catalog_id = 'issn-1472-3409'),
                     (SELECT area FROM article_listing),
                     (SELECT journal_title FROM article_search)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .expect("reconciled state should read");
        assert_eq!(
            reconciled,
            (
                4,
                "Environment and Planning A: Economy and Space".to_string(),
                "Regional, Environmental & Resource Studies".to_string(),
                "Environment and Planning A: Economy and Space".to_string(),
            )
        );
    }

    #[test]
    fn legacy_alias_sync_state_blocks_reconciliation_across_provider_namespaces() {
        let directory = tempdir().expect("temporary directory should create");
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let mut original = environment_catalog();
        original.catalog_aliases.clear();
        original.title = "Environment and Planning A".to_string();
        original.title_aliases.clear();
        original.issn = None;
        original.all_issns = vec!["1472-3409".to_string()];
        write_content_batch(
            &content,
            &original,
            &canonical_batch_for_catalog(&original),
            "english:environment:seed",
            "2026-07-20T00:00:00Z",
        )
        .expect("original canonical content should write");
        let alias_run = prepared_run(
            prepare_journal_sync(
                &control,
                "english_journals",
                "provider-b",
                "issn-0308-518x",
                "legacy-alias-run",
                IndexSyncMode::Bootstrap,
                false,
                "2026-07-20T00:00:00Z",
            )
            .expect("legacy alias run should prepare"),
        );
        advance_run_checkpoint(
            &control,
            "english_journals",
            "provider-b",
            "issn-0308-518x",
            &alias_run.run_id,
            alias_run.mode,
            alias_run.base_anchor.as_deref(),
            "legacy-cursor",
            "2026-07-20T00:00:00Z",
        )
        .expect("legacy checkpoint should write");
        let owners_before = content
            .query_row("SELECT COUNT(*) FROM journal_identity_keys", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("identity count should read");
        let provider = StaticProvider::new();

        let error = prepare_catalog_identities(
            &content,
            &control,
            &content_path,
            "english_journals",
            &[environment_catalog()],
        )
        .expect_err("legacy checkpoint must fail closed");

        let LiveIndexError::InvalidConfig(message) = error else {
            panic!("legacy checkpoint returned unexpected error: {error:?}");
        };
        assert_eq!(
            message,
            "legacy catalog alias has provider synchronization state"
        );
        assert_eq!(*provider.calls.lock().expect("call count should lock"), 0);
        assert_eq!(
            content
                .query_row("SELECT COUNT(*) FROM journal_identity_keys", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("identity count should read"),
            owners_before
        );
        assert_eq!(
            content
                .query_row(
                    "SELECT title FROM journals WHERE catalog_id = 'issn-1472-3409'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("journal title should read"),
            "Environment and Planning A"
        );
    }

    #[test]
    fn nonempty_legacy_journal_blocks_before_provider_fetch() {
        let directory = tempdir().expect("temporary directory should create");
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let legacy = legacy_environment_catalog();
        write_content_batch(
            &content,
            &legacy,
            &canonical_batch_for_catalog(&legacy),
            "english:legacy:seed",
            "2026-07-20T00:00:00Z",
        )
        .expect("legacy content should write");
        let provider = StaticProvider::new();

        let error = prepare_catalog_identities(
            &content,
            &control,
            &content_path,
            "english_journals",
            &[environment_catalog()],
        )
        .expect_err("nonempty legacy journal must fail closed");

        let LiveIndexError::ContentDatabase { source, .. } = error else {
            panic!("nonempty legacy journal returned unexpected error: {error:?}");
        };
        assert_eq!(
            source.to_string(),
            "legacy journal entity owns content or durable history"
        );
        assert_eq!(*provider.calls.lock().expect("call count should lock"), 0);
        assert_eq!(
            content
                .query_row(
                    "SELECT COUNT(*) FROM journals WHERE catalog_id = 'issn-0308-518x'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("legacy journal count should read"),
            1
        );
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
                mode: IndexSyncMode::Bootstrap,
                committed_anchor: None,
                traversal_checkpoint: None,
            }],
        }
    }

    fn worker_test_config(
        provider_name: &str,
        cnki_captcha_token: Option<String>,
    ) -> LiveIndexConfig {
        LiveIndexConfig {
            application_executable: "litradar".into(),
            project_root: ".".into(),
            secret_key_file: "secret.key".into(),
            file: None,
            worker_count: 1,
            process_count: 1,
            issue_batch_size: 1,
            timeout_seconds: 10,
            resume: true,
            update: false,
            full_rescan: false,
            notify: false,
            notify_dry_run: true,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            cnki_captcha_token,
            provider_proxy_selection: ProviderProxySelection::default(),
            index_provider_routes: BTreeMap::from([(
                "chinese_journals".to_string(),
                provider_name.to_string(),
            )]),
        }
    }

    #[test]
    fn worker_boundary_redacts_provider_credentials() {
        let sentinel = "captcha-secret-sentinel";
        let proxy_sentinel = "socks5h://user:worker-proxy-sentinel@proxy.example:1080";
        let mut config = worker_test_config("cnki", Some(sentinel.to_string()));
        config.provider_proxy_selection =
            ProviderProxySelection::new(proxy_sentinel, r#"{"cnki":true}"#)
                .expect("worker proxy selection should validate");
        let cnki_request = fetch_worker_request("cnki", "run-cnki-bootstrap");
        let cnki_proxy_url = config
            .provider_proxy_selection
            .proxy_url_for_provider(&cnki_request.provider_name);
        let cnki_bootstrap =
            worker_bootstrap(&cnki_request, Some(sentinel), cnki_proxy_url.as_deref());
        let scholarly_request = fetch_worker_request("scholarly", "run-scholarly-bootstrap");
        let scholarly_proxy_url = config
            .provider_proxy_selection
            .proxy_url_for_provider(&scholarly_request.provider_name);
        let scholarly_bootstrap = worker_bootstrap(
            &scholarly_request,
            Some(sentinel),
            scholarly_proxy_url.as_deref(),
        );
        let overseas_request = fetch_worker_request("cnki_oversea", "run-overseas-bootstrap");
        let overseas_proxy_url = config
            .provider_proxy_selection
            .proxy_url_for_provider(&overseas_request.provider_name);
        let overseas_bootstrap = worker_bootstrap(
            &overseas_request,
            Some(sentinel),
            overseas_proxy_url.as_deref(),
        );

        let debug = format!("{config:?}");
        let bootstrap_debug = format!("{cnki_bootstrap:?}");

        assert!(!debug.contains(sentinel));
        assert!(!debug.contains(proxy_sentinel));
        assert!(debug.contains("[REDACTED]"));
        assert!(!bootstrap_debug.contains(sentinel));
        assert!(!bootstrap_debug.contains(proxy_sentinel));
        assert!(bootstrap_debug.contains("[REDACTED]"));
        assert_eq!(cnki_bootstrap.cnki_captcha_token.as_deref(), Some(sentinel));
        assert_eq!(
            cnki_bootstrap.provider_proxy_url.as_deref(),
            Some(proxy_sentinel)
        );
        assert!(scholarly_bootstrap.cnki_captcha_token.is_none());
        assert!(scholarly_bootstrap.provider_proxy_url.is_none());
        assert!(overseas_bootstrap.cnki_captcha_token.is_none());
        assert!(overseas_bootstrap.provider_proxy_url.is_none());
    }

    #[test]
    fn worker_protocol_proxy_selection_matches_direct_and_multiprocess_paths() {
        let proxy_sentinel = "socks5h://user:worker-equivalence-sentinel@proxy.example:1080";
        let selection =
            ProviderProxySelection::new(proxy_sentinel, r#"{"cnki":true,"scholarly":false}"#)
                .expect("worker proxy selection should validate");

        for provider_name in ["cnki", "scholarly", "cnki_oversea"] {
            let request = fetch_worker_request(provider_name, "run-proxy-equivalence");
            let direct_proxy = selection.for_provider(provider_name);
            let proxy_url = selection.proxy_url_for_provider(provider_name);
            let bootstrap = worker_bootstrap(&request, None, proxy_url.as_deref());
            let mut input = Vec::new();
            write_message(&mut input, &bootstrap).expect("worker bootstrap should serialize");
            let (_, multiprocess_proxy) = read_worker_bootstrap(&request, &mut Cursor::new(input))
                .expect("worker bootstrap should select a proxy");

            assert_eq!(multiprocess_proxy, direct_proxy);
            assert_eq!(multiprocess_proxy.url(), direct_proxy.url());
            assert!(!format!("{multiprocess_proxy:?}").contains(proxy_sentinel));
        }
    }

    #[test]
    fn worker_request_file_excludes_runtime_secrets() {
        let sentinel = "captcha-secret-sentinel";
        let proxy_sentinel = "socks5h://user:worker-request-proxy-sentinel@proxy.example:1080";
        let mut config = worker_test_config("cnki", Some(sentinel.to_string()));
        config.provider_proxy_selection =
            ProviderProxySelection::new(proxy_sentinel, r#"{"cnki":true}"#)
                .expect("worker proxy selection should validate");
        let directory = tempdir().expect("temporary control directory should create");
        let control = open_control_db(directory.path().join("control.sqlite"))
            .expect("control database should open");
        let context = ParentWriterContext {
            catalog_name: "chinese_journals".to_string(),
            provider_name: "cnki".to_string(),
            batch_id: TEST_BATCH_ID.to_string(),
            run_id: "run-secret-boundary".to_string(),
            timestamp: "time".to_string(),
        };

        let (requests, _) =
            prepare_worker_requests(&config, &control, &context, 0, &[catalog("journal-1")])
                .expect("worker request should prepare");
        let request_path = directory.path().join("worker-request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&requests[0]).expect("worker request should serialize"),
        )
        .expect("worker request should write");
        let request_json =
            std::fs::read_to_string(request_path).expect("worker request should read");

        assert!(!request_json.contains(sentinel));
        assert!(!request_json.contains(proxy_sentinel));
        assert!(!request_json.contains("cnki_captcha_token"));
        assert!(!request_json.contains("provider_proxy"));
    }

    #[test]
    fn provider_switch_uses_new_control_namespace_and_same_content_ids() {
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
        assert!(
            read_sync_anchor(&control, "chinese_journals", "provider-a", "journal-1")
                .expect("provider A anchor should read")
                .is_some()
        );
        assert!(
            read_sync_anchor(&control, "chinese_journals", "provider-b", "journal-1")
                .expect("provider B anchor should read")
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
    fn worker_partitioning_and_concurrency_validation_are_bounded() {
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
            full_rescan: false,
            notify: false,
            notify_dry_run: true,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            cnki_captcha_token: None,
            provider_proxy_selection: ProviderProxySelection::default(),
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
            batch_id: TEST_BATCH_ID.to_string(),
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
        excessive_workers.worker_count = SCHOLARLY_WORKER_COUNT_MAX + 1;
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
        let mut excessive_aggregate = config.clone();
        excessive_aggregate.worker_count = INDEX_AGGREGATE_CONCURRENCY_MAX / 2 + 1;
        excessive_aggregate.process_count = 2;
        excessive_aggregate.index_provider_routes =
            BTreeMap::from([("catalog".to_string(), CNKI_PROVIDER_NAME.to_string())]);
        assert!(matches!(
            validate_live_config(&excessive_aggregate),
            Err(LiveIndexError::InvalidConfig(message))
                if message == "process_count * worker_count must be at most 32"
        ));
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
    fn scholarly_credential_preflight_rejects_missing_values_before_mutation() {
        let cases = [
            (
                "",
                "semantic-scholar-secret",
                "contact-secret@example.invalid",
                "OpenAlex API key is required for scholarly indexing",
            ),
            (
                "openalex-secret",
                "",
                "contact-secret@example.invalid",
                "Semantic Scholar API key is required for scholarly indexing",
            ),
            (
                "openalex-secret",
                "semantic-scholar-secret",
                "",
                "Crossref mailto is required for scholarly indexing",
            ),
        ];

        for (openalex_keys, semantic_scholar_keys, crossref_mailtos, expected_message) in cases {
            let directory = tempdir().expect("temporary directory should create");
            let config = LiveIndexConfig {
                application_executable: "litradar".into(),
                project_root: directory.path().to_path_buf(),
                secret_key_file: "secret.key".into(),
                file: None,
                worker_count: 1,
                process_count: 1,
                issue_batch_size: 1,
                timeout_seconds: 10,
                resume: true,
                update: false,
                full_rescan: false,
                notify: false,
                notify_dry_run: true,
                scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                    10,
                    openalex_keys,
                    semantic_scholar_keys,
                    crossref_mailtos,
                ),
                cnki_captcha_token: None,
                provider_proxy_selection: ProviderProxySelection::default(),
                index_provider_routes: BTreeMap::from([(
                    "catalog".to_string(),
                    "scholarly".to_string(),
                )]),
            };

            let error = run_live_index(&config).expect_err("missing credential should fail");
            let LiveIndexError::InvalidConfig(message) = error else {
                panic!("missing credential returned unexpected error: {error:?}");
            };
            assert_eq!(message, expected_message);
            for secret in [openalex_keys, semantic_scholar_keys, crossref_mailtos] {
                assert!(secret.is_empty() || !message.contains(secret));
            }
            assert!(!directory.path().join("data").exists());
        }
    }

    #[test]
    fn single_writer_parent_preloads_successful_and_inflight_state() {
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
            full_rescan: false,
            notify: false,
            notify_dry_run: true,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            cnki_captcha_token: None,
            provider_proxy_selection: ProviderProxySelection::default(),
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
            batch_id: TEST_BATCH_ID.to_string(),
            run_id: "run".to_string(),
            timestamp: "2026-07-19T00:00:00Z".to_string(),
        };
        seed_completed_sync_for_batch(
            &control,
            &context.catalog_name,
            &context.provider_name,
            "complete",
            TEST_BATCH_ID,
            None,
            &context.timestamp,
        );
        let resumable = prepared_run(
            prepare_journal_sync(
                &control,
                &context.catalog_name,
                &context.provider_name,
                "resumable",
                "previous-run",
                IndexSyncMode::Bootstrap,
                false,
                &context.timestamp,
            )
            .expect("resumable run should prepare"),
        );
        advance_run_checkpoint(
            &control,
            &context.catalog_name,
            &context.provider_name,
            "resumable",
            &resumable.run_id,
            resumable.mode,
            resumable.base_anchor.as_deref(),
            "cursor-resume",
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
            requests[0].assignments[0].traversal_checkpoint.as_deref(),
            Some("cursor-resume")
        );
        assert_eq!(requests[0].assignments[0].mode, IndexSyncMode::Bootstrap);
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
            batch_id: TEST_BATCH_ID.to_string(),
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
            None,
            &ProviderProxySelection::default(),
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
        assert!(matches!(batch.progress, ProviderProgress::Continue { .. }));
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
        assert!(matches!(batch.progress, ProviderProgress::Complete { .. }));
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
    fn spawn_stdio_echo_process() -> SupervisedChild {
        let mut command = Command::new("cmd");
        command
            .args(["/D", "/S", "/C", "more"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        SupervisedChild::spawn(&mut command).expect("Windows stdio echo child should start")
    }

    #[cfg(not(target_os = "windows"))]
    fn spawn_stdio_echo_process() -> SupervisedChild {
        let mut command = Command::new("cat");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        SupervisedChild::spawn(&mut command).expect("stdio echo child should start")
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
        let mut input = Vec::new();
        write_message(&mut input, &worker_bootstrap(&request, None, None))
            .expect("worker bootstrap should serialize");
        tracing::subscriber::with_default(captured.subscriber(), || {
            run_live_index_worker_with_io(&request_path, Cursor::new(input), &mut output)
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
    fn worker_protocol_domestic_bootstrap_completes_handshake() {
        let sentinel = "captcha-secret-sentinel";
        let proxy_sentinel = "socks5h://user:domestic-worker-proxy-sentinel@proxy.example:1080";
        let directory = tempdir().expect("temporary directory should create");
        let mut request = fetch_worker_request("cnki", "run-domestic-bootstrap");
        request.assignments.clear();
        let request_path = directory.path().join("worker-request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&request).expect("worker request should serialize"),
        )
        .expect("worker request should write");
        let mut input = Vec::new();
        write_message(
            &mut input,
            &worker_bootstrap(&request, Some(sentinel), Some(proxy_sentinel)),
        )
        .expect("domestic bootstrap should serialize");
        let mut output = Vec::new();

        run_live_index_worker_with_io(&request_path, Cursor::new(input), &mut output)
            .expect("domestic worker handshake should complete");
        let output_text = String::from_utf8(output.clone()).expect("worker output should be UTF-8");
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
        assert!(!output_text.contains(sentinel));
        assert!(!output_text.contains(proxy_sentinel));
    }

    #[test]
    fn worker_protocol_rejects_non_domestic_secret_without_exposure() {
        let sentinel = "captcha-secret-sentinel";
        let directory = tempdir().expect("temporary directory should create");
        let mut request = fetch_worker_request("scholarly", "run-invalid-bootstrap");
        request.assignments.clear();
        let request_path = directory.path().join("worker-request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&request).expect("worker request should serialize"),
        )
        .expect("worker request should write");
        let bootstrap = LiveIndexWorkerBootstrap {
            protocol_version: PROTOCOL_VERSION,
            worker_id: 0,
            cnki_captcha_token: Some(sentinel.to_string()),
            provider_proxy_url: None,
        };
        let mut input = Vec::new();
        write_message(&mut input, &bootstrap).expect("invalid bootstrap should serialize");
        let mut output = Vec::new();

        run_live_index_worker_with_io(&request_path, Cursor::new(input), &mut output)
            .expect("worker should emit a redacted terminal failure");
        let output_text = String::from_utf8(output.clone()).expect("worker output should be UTF-8");
        let message: WorkerMessage = read_message(&mut Cursor::new(output))
            .expect("terminal worker message should deserialize");

        assert!(matches!(
            message,
            WorkerMessage::Failed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence: 0,
                failure: LiveIndexWorkerFailure {
                    class: LiveIndexWorkerFailureClass::InvalidConfig,
                    ..
                },
            }
        ));
        assert!(!output_text.contains(sentinel));
        assert!(!format!("{bootstrap:?}").contains(sentinel));
    }

    #[test]
    fn worker_protocol_rejects_mismatched_bootstrap_identity() {
        let directory = tempdir().expect("temporary directory should create");
        let mut request = fetch_worker_request("scholarly", "run-bootstrap-mismatch");
        request.assignments.clear();
        let request_path = directory.path().join("worker-request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&request).expect("worker request should serialize"),
        )
        .expect("worker request should write");
        let mismatches = [
            LiveIndexWorkerBootstrap {
                protocol_version: PROTOCOL_VERSION - 1,
                worker_id: 0,
                cnki_captcha_token: None,
                provider_proxy_url: None,
            },
            LiveIndexWorkerBootstrap {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 1,
                cnki_captcha_token: None,
                provider_proxy_url: None,
            },
        ];

        for bootstrap in mismatches {
            let mut input = Vec::new();
            write_message(&mut input, &bootstrap).expect("bootstrap should serialize");
            let mut output = Vec::new();
            run_live_index_worker_with_io(&request_path, Cursor::new(input), &mut output)
                .expect("worker should emit a terminal failure");
            let message: WorkerMessage = read_message(&mut Cursor::new(output))
                .expect("terminal worker message should deserialize");
            assert!(matches!(
                message,
                WorkerMessage::Failed {
                    failure: LiveIndexWorkerFailure {
                        class: LiveIndexWorkerFailureClass::InvalidConfig,
                        ..
                    },
                    ..
                }
            ));
        }
    }

    #[test]
    fn worker_protocol_bootstrap_failure_cleans_request_and_process() {
        let sentinel = "captcha-secret-sentinel";
        let directory = tempdir().expect("temporary writer directory should create");
        let content = open_content_db(directory.path().join("content.sqlite"))
            .expect("content database should open");
        let control = open_control_db(directory.path().join("control.sqlite"))
            .expect("control database should open");
        let context = ParentWriterContext {
            catalog_name: "chinese_journals".to_string(),
            provider_name: "cnki".to_string(),
            batch_id: TEST_BATCH_ID.to_string(),
            run_id: "run-bootstrap-write-failure".to_string(),
            timestamp: "2026-07-24T00:00:00Z".to_string(),
        };
        let request = fetch_worker_request(&context.provider_name, &context.run_id);
        let request_dir = directory.path().join("worker-requests");
        let captured = CapturedLogs::default();

        let error = tracing::subscriber::with_default(captured.subscriber(), || {
            run_worker_processes_with_launcher(
                &request_dir,
                &content,
                &control,
                &context,
                vec![request],
                Some(sentinel),
                &ProviderProxySelection::default(),
                IndexRunMetrics::default(),
                Duration::from_secs(1),
                |_, _| {
                    Ok(LaunchedWorkerProcess::from_test_streams(
                        spawn_stdio_echo_process(),
                        Cursor::new(Vec::<u8>::new()),
                        FailingWriter,
                    ))
                },
                |_| {},
            )
        })
        .expect_err("bootstrap write failure should stop supervision");

        assert!(matches!(error, LiveIndexError::Worker(_)));
        assert!(!format!("{error:?}").contains(sentinel));
        assert!(!captured.text().contains(sentinel));
        assert!(request_dir.exists());
        assert!(std::fs::read_dir(&request_dir)
            .expect("request directory should read")
            .next()
            .is_none());
    }

    #[test]
    fn worker_protocol_version_six_rejects_version_five_requests() {
        let directory = tempdir().expect("temporary directory should create");
        let mut request = fetch_worker_request("scholarly", "run-version-mismatch");
        assert_eq!(PROTOCOL_VERSION, 6);
        request.protocol_version = 5;
        request.assignments.clear();
        let request_path = directory.path().join("worker-request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&request).expect("worker request should serialize"),
        )
        .expect("worker request should write");

        let mut output = Vec::new();
        let mut input = Vec::new();
        write_message(&mut input, &worker_bootstrap(&request, None, None))
            .expect("worker bootstrap should serialize");
        run_live_index_worker_with_io(&request_path, Cursor::new(input), &mut output)
            .expect("worker entrypoint should emit a redacted terminal failure");
        let message: WorkerMessage = read_message(&mut Cursor::new(output))
            .expect("terminal worker message should deserialize");

        assert!(matches!(
            message,
            WorkerMessage::Failed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence: 0,
                failure: LiveIndexWorkerFailure {
                    class: LiveIndexWorkerFailureClass::InvalidConfig,
                    ..
                },
            }
        ));
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
            "commit_content_then_progress(",
            "prepare_journal_sync(",
            "heartbeat_lease(",
            "advance_run_checkpoint(",
            "complete_sync_run(",
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
    fn single_writer_control_failure_replays_committed_content_idempotently() {
        let directory = tempdir().expect("temporary directory should create");
        let content_path = directory.path().join("content.sqlite");
        let control_path = directory.path().join("control.sqlite");
        let content = open_content_db(&content_path).expect("content should open");
        let control = open_control_db(&control_path).expect("control should open");
        let catalog = catalog("journal-single-writer-replay");
        let batch = canonical_batch(&catalog);
        let run = prepared_run(
            prepare_journal_sync(
                &control,
                "chinese_journals",
                "provider-a",
                &catalog.catalog_id,
                "single-writer-run",
                IndexSyncMode::Bootstrap,
                false,
                "2026-07-18T00:00:00Z",
            )
            .expect("single-writer run should prepare"),
        );
        control
            .execute_batch(
                "CREATE TRIGGER fail_single_writer_control
                 BEFORE INSERT ON provider_sync_anchors
                 BEGIN SELECT RAISE(ABORT, 'forced control failure'); END;",
            )
            .expect("control failpoint should install");

        let checkpoint_error = commit_content_then_progress(
            &control,
            "chinese_journals",
            "provider-a",
            &catalog.catalog_id,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            &batch.progress,
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
            read_sync_anchor(
                &control,
                "chinese_journals",
                "provider-a",
                &catalog.catalog_id
            )
            .expect("anchor should read"),
            None,
        );
        assert!(read_run_checkpoint(
            &control,
            "chinese_journals",
            "provider-a",
            &catalog.catalog_id
        )
        .expect("run should read")
        .is_some());
        control
            .execute_batch("DROP TRIGGER fail_single_writer_control")
            .expect("control failpoint should drop");

        let replay = commit_content_then_progress(
            &control,
            "chinese_journals",
            "provider-a",
            &catalog.catalog_id,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            &batch.progress,
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
        .expect("single-writer replay should advance the anchor");
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(replay.change_events_emitted, 0);
        assert!(read_sync_anchor(
            &control,
            "chinese_journals",
            "provider-a",
            &catalog.catalog_id
        )
        .expect("anchor should read")
        .is_some());
    }

    #[test]
    fn worker_failure_parent_content_commit_kinds_are_fixed_and_redacted() {
        const SENTINELS: [&str; 8] = [
            "C:\\private\\catalog.sqlite",
            "openalex-key-sentinel",
            "operator-sentinel@example.test",
            "10.9999/sentinel-doi",
            "Sentinel Article Title",
            "cursor-sentinel-value",
            "response-body-sentinel",
            "Bearer sentinel-token-value",
        ];
        let contract_message = SENTINELS.join(" | ");
        let cases = vec![
            (
                ContentDatabaseError::Json(
                    serde_json::from_str::<serde_json::Value>("{")
                        .expect_err("invalid JSON should fail"),
                ),
                "json",
            ),
            (
                ContentDatabaseError::Contract(ContractViolation::new(contract_message)),
                "contract",
            ),
            (
                ContentDatabaseError::Identity(ArticleIdentityError::MissingIdentity),
                "identity_missing",
            ),
            (
                ContentDatabaseError::Identity(ArticleIdentityError::ConflictingAliases {
                    article_ids: vec![9_123_456_789, 9_876_543_210],
                }),
                "identity_conflicting_aliases",
            ),
            (
                ContentDatabaseError::Merge(ArticleMergeError::CatalogMismatch),
                "merge_catalog_mismatch",
            ),
            (
                ContentDatabaseError::Merge(ArticleMergeError::ConflictingIdentifier {
                    field: "DOI",
                }),
                "merge_conflicting_doi",
            ),
            (
                ContentDatabaseError::Merge(ArticleMergeError::ConflictingIdentifier {
                    field: "PMID",
                }),
                "merge_conflicting_pmid",
            ),
            (
                ContentDatabaseError::Merge(ArticleMergeError::ConflictingIdentifier {
                    field: "openalex-key-sentinel",
                }),
                "merge_conflicting_other",
            ),
            (
                ContentDatabaseError::RebuildRequired {
                    found_version: 9_001,
                },
                "rebuild_required",
            ),
            (
                ContentDatabaseError::InvalidCurrentSchema(
                    "C:\\private\\catalog.sqlite operator-sentinel@example.test".to_string(),
                ),
                "invalid_current_schema",
            ),
            (
                ContentDatabaseError::ArticleIdCollision {
                    article_id: 9_223_372_036_854_775_000,
                },
                "article_id_collision",
            ),
        ];

        for (error, expected) in &cases {
            assert_eq!(
                ContentCommitErrorKind::from_error(error).map(ContentCommitErrorKind::as_str),
                Some(*expected)
            );
        }

        let captured = CapturedLogs::default();
        tracing::subscriber::with_default(captured.subscriber(), || {
            for (error, _) in &cases {
                emit_parent_content_commit_failure(0, error);
            }
        });
        let events = captured
            .text()
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("event should parse")
            })
            .collect::<Vec<_>>();
        assert_eq!(events.len(), cases.len());
        for (event, (_, expected)) in events.iter().zip(&cases) {
            assert_eq!(event["event"], "index.worker.failed");
            assert_eq!(event["worker_id"], 0);
            assert_eq!(event["failure_class"], "content");
            assert_eq!(event["operation"], "content_commit");
            assert_eq!(event["has_sqlite_code"], false);
            assert_eq!(event["is_busy_or_locked"], false);
            assert_eq!(event["content_error_kind"], *expected);
            assert!(event.get("sqlite_code").is_none());
            assert!(event.get("sqlite_extended_code").is_none());
        }

        let mut combined = captured.text();
        for (error, _) in cases {
            let error = LiveIndexError::Commit(ContentCheckpointCommitError::Content(error));
            let failure = LiveIndexWorkerFailure::from_error(&error);
            let message = WorkerMessage::Failed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence: 0,
                failure: failure.clone(),
            };
            let payload = serde_json::to_value(&message).expect("worker message should serialize");
            assert_eq!(
                payload["failure"]
                    .as_object()
                    .expect("failure should be an object")
                    .len(),
                5
            );
            assert!(payload["failure"].get("content_error_kind").is_none());
            combined.push_str(
                &serde_json::to_string(&message).expect("worker message should serialize"),
            );
            combined.push_str(&worker_failure_error(0, &failure).to_string());
        }
        for sentinel in SENTINELS {
            assert!(
                !combined.contains(sentinel),
                "content failure boundary exposed sensitive sentinel"
            );
        }
        for identifier in ["9123456789", "9876543210", "9223372036854775000"] {
            assert!(
                !combined.contains(identifier),
                "content failure boundary exposed an internal identifier"
            );
        }
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
            let LiveIndexError::Commit(ContentCheckpointCommitError::Content(source)) = &error
            else {
                panic!("expected content commit error");
            };
            emit_parent_content_commit_failure(2, source);
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
        assert!(event.get("content_error_kind").is_none());
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
    fn parent_heartbeat_preserves_domestic_cnki_lease_and_run_checkpoint() {
        let directory = tempdir().expect("temporary directory should create");
        let control_path = directory.path().join("control.sqlite");
        let control = open_control_db(&control_path).expect("control should open");
        let now = LiveRunTime::now().epoch_seconds;
        acquire_lease(&control, "catalog", "cnki", "run", now).expect("lease should acquire");
        let run = prepared_run(
            prepare_journal_sync(
                &control,
                "catalog",
                "cnki",
                "journal",
                "run",
                IndexSyncMode::Bootstrap,
                false,
                "2026-07-24T00:00:00Z",
            )
            .expect("run checkpoint should prepare"),
        );
        advance_run_checkpoint(
            &control,
            "catalog",
            "cnki",
            "journal",
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            "domestic-cursor",
            "2026-07-24T00:00:00Z",
        )
        .expect("checkpoint should write");
        let mut heartbeat = LeaseHeartbeat::start(
            control_path.clone(),
            "catalog".to_string(),
            "cnki".to_string(),
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
        assert_eq!(
            read_run_checkpoint(&control, "catalog", "cnki", "journal")
                .expect("domestic checkpoint should read")
                .expect("domestic run should remain")
                .traversal_checkpoint
                .as_deref(),
            Some("domestic-cursor")
        );
    }
}
