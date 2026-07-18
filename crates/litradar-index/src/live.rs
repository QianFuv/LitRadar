//! Provider-neutral live catalog indexing orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use litradar_domain::{JournalCatalogEntry, ProviderBatch};
use litradar_provider::{
    IndexContentProvider, ProviderError, ProviderRegistration, ProviderRegistryError,
};
use litradar_sources::{
    cnki_index_registration, scholarly_index_registration, LiveCnkiConfig, LiveCnkiTransport,
    LiveScholarlyConfig, LiveScholarlyTransport, CNKI_PROVIDER_NAME,
    OPENALEX_MAX_WORKERS_PER_PROCESS, SCHOLARLY_PROVIDER_NAME,
};
use rusqlite::Connection;
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

const LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS: u64 = 30;
const MAX_PROVIDER_PAGES_PER_JOURNAL: usize = 100_000;
const SCHOLARLY_MAX_PROCESS_COUNT: usize = 3;

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

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LiveIndexWorkerRequest {
    control_path: PathBuf,
    content_path: PathBuf,
    catalog_name: String,
    provider_name: String,
    run_id: String,
    timestamp: String,
    worker_id: usize,
    process_count: usize,
    source_worker_count: usize,
    schedule_epoch_unix_millis: u64,
    timeout_seconds: u64,
    resume: bool,
    update: bool,
    scholarly_config: LiveScholarlyConfig,
    entries: Vec<JournalCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LiveIndexWorkerResponse {
    worker_id: usize,
    status: String,
    metrics: IndexRunMetrics,
    error: Option<String>,
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
                    Err(RecvTimeoutError::Timeout) => heartbeat_lease(
                        &connection,
                        &catalog_name,
                        &provider_name,
                        &run_id,
                        LiveRunTime::now().epoch_seconds,
                    )
                    .map_err(|error| error.to_string())?,
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

/// Run one serialized journal-worker request and return its machine-readable response.
///
/// # Arguments
///
/// * `request_path` - Disposable JSON request path created by the parent process.
///
/// # Returns
///
/// One JSON response suitable for the worker subprocess stdout boundary.
pub fn run_live_index_worker_from_file_path(
    request_path: impl AsRef<Path>,
) -> Result<String, LiveIndexError> {
    let request: LiveIndexWorkerRequest =
        serde_json::from_str(&std::fs::read_to_string(request_path)?)?;
    let worker_id = request.worker_id;
    let response = match run_worker_request(&request) {
        Ok(metrics) => LiveIndexWorkerResponse {
            worker_id,
            status: "succeeded".to_string(),
            metrics,
            error: None,
        },
        Err(error) => LiveIndexWorkerResponse {
            worker_id,
            status: "failed".to_string(),
            metrics: IndexRunMetrics::default(),
            error: Some(error.to_string()),
        },
    };
    Ok(serde_json::to_string(&response)?)
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
    let content =
        open_content_db(&content_path).map_err(|source| LiveIndexError::ContentDatabase {
            path: content_path.clone(),
            source,
        })?;
    drop(content);
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
    let mut heartbeat = LeaseHeartbeat::start(
        control_path.clone(),
        catalog_name.clone(),
        provider_name.clone(),
        run_id.clone(),
        Duration::from_secs(LIVE_INDEX_HEARTBEAT_INTERVAL_SECONDS),
    );

    let execution = if config.process_count > 1 && entries.len() > 1 {
        let requests = build_worker_requests(
            config,
            &control_path,
            &content_path,
            &catalog_name,
            &provider_name,
            &run_id,
            &timestamp,
            run_time.epoch_milliseconds,
            &entries,
        );
        run_worker_processes(config, requests)
    } else {
        let request = LiveIndexWorkerRequest {
            control_path: control_path.clone(),
            content_path: content_path.clone(),
            catalog_name: catalog_name.clone(),
            provider_name: provider_name.clone(),
            run_id: run_id.clone(),
            timestamp: timestamp.clone(),
            worker_id: 0,
            process_count: 1,
            source_worker_count: config.worker_count,
            schedule_epoch_unix_millis: run_time.epoch_milliseconds,
            timeout_seconds: config.timeout_seconds,
            resume: config.resume,
            update: config.update,
            scholarly_config: config.scholarly_config.clone(),
            entries: entries.clone(),
        };
        run_worker_request(&request)
    };

    let heartbeat_result = heartbeat.stop_and_check();
    let release_result = release_lease(&control, &catalog_name, &provider_name, &run_id);
    let metrics = match execution {
        Ok(metrics) => metrics,
        Err(error) => {
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
    heartbeat_result?;
    release_result?;

    let content =
        open_content_db(&content_path).map_err(|source| LiveIndexError::ContentDatabase {
            path: content_path.clone(),
            source,
        })?;
    optimize_content_db(&content).map_err(|source| LiveIndexError::ContentDatabase {
        path: content_path.clone(),
        source,
    })?;
    let db_name = content_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("index.sqlite");
    let manifest_path = config.update.then(|| {
        config
            .project_root
            .join("data")
            .join("push_state")
            .join(format!("{catalog_name}.changes.json"))
    });
    if let Some(path) = manifest_path.as_deref() {
        write_content_change_manifest(&content, db_name, &run_id, &timestamp, path)?;
    } else {
        discard_content_change_events(&content).map_err(|error| {
            LiveIndexError::Worker(format!("content outbox acknowledgement failed: {error}"))
        })?;
    }
    let notify_exit_code = if config.notify {
        Some(run_notify_for_manifest(
            config,
            db_name,
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

#[allow(clippy::too_many_arguments)]
fn build_worker_requests(
    config: &LiveIndexConfig,
    control_path: &Path,
    content_path: &Path,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
    timestamp: &str,
    schedule_epoch_unix_millis: u64,
    entries: &[JournalCatalogEntry],
) -> Vec<LiveIndexWorkerRequest> {
    let process_count = config.process_count.min(entries.len()).max(1);
    let mut partitions = vec![Vec::new(); process_count];
    for (index, entry) in entries.iter().cloned().enumerate() {
        partitions[index % process_count].push(entry);
    }
    partitions
        .into_iter()
        .enumerate()
        .map(|(worker_id, entries)| LiveIndexWorkerRequest {
            control_path: control_path.to_path_buf(),
            content_path: content_path.to_path_buf(),
            catalog_name: catalog_name.to_string(),
            provider_name: provider_name.to_string(),
            run_id: run_id.to_string(),
            timestamp: timestamp.to_string(),
            worker_id,
            process_count,
            source_worker_count: config.worker_count,
            schedule_epoch_unix_millis,
            timeout_seconds: config.timeout_seconds,
            resume: config.resume,
            update: config.update,
            scholarly_config: config.scholarly_config.clone(),
            entries,
        })
        .collect()
}

fn run_worker_processes(
    config: &LiveIndexConfig,
    requests: Vec<LiveIndexWorkerRequest>,
) -> Result<IndexRunMetrics, LiveIndexError> {
    let request_dir = config
        .project_root
        .join("data")
        .join("index-control")
        .join("worker-requests");
    std::fs::create_dir_all(&request_dir)?;
    let mut children = Vec::with_capacity(requests.len());
    for request in requests {
        let request_path = request_dir.join(format!(
            "{}-worker-{}.json",
            request.run_id, request.worker_id
        ));
        let request_bytes = match serde_json::to_vec(&request) {
            Ok(request_bytes) => request_bytes,
            Err(error) => {
                cancel_workers(&mut children, 0);
                return Err(LiveIndexError::Json(error));
            }
        };
        if let Err(error) = std::fs::write(&request_path, request_bytes) {
            cancel_workers(&mut children, 0);
            return Err(LiveIndexError::Io(error));
        }
        let child = match Command::new(&config.application_executable)
            .arg("index")
            .arg("--live-worker-request")
            .arg(&request_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let _ = std::fs::remove_file(&request_path);
                cancel_workers(&mut children, 0);
                return Err(LiveIndexError::Worker(format!(
                    "worker {} could not start: {error}",
                    request.worker_id
                )));
            }
        };
        children.push(SpawnedWorker {
            worker_id: request.worker_id,
            request_path,
            child: Some(child),
        });
    }

    let mut aggregate = IndexRunMetrics::default();
    for index in 0..children.len() {
        let child = children[index]
            .child
            .take()
            .expect("spawned worker should own its process");
        let output = child.wait_with_output().map_err(|error| {
            cancel_workers(&mut children, index + 1);
            LiveIndexError::Worker(format!(
                "worker {} wait failed: {error}",
                children[index].worker_id
            ))
        })?;
        let _ = std::fs::remove_file(&children[index].request_path);
        if !output.status.success() {
            cancel_workers(&mut children, index + 1);
            return Err(LiveIndexError::Worker(format!(
                "worker {} exited with status {}",
                children[index].worker_id, output.status
            )));
        }
        let response: LiveIndexWorkerResponse =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                cancel_workers(&mut children, index + 1);
                LiveIndexError::Worker(format!(
                    "worker {} returned invalid JSON: {error}",
                    children[index].worker_id
                ))
            })?;
        if response.status != "succeeded" {
            cancel_workers(&mut children, index + 1);
            return Err(LiveIndexError::Worker(format!(
                "worker {} failed: {}",
                response.worker_id,
                response
                    .error
                    .as_deref()
                    .unwrap_or("no safe error was returned")
            )));
        }
        aggregate.merge(&response.metrics);
    }
    Ok(aggregate)
}

struct SpawnedWorker {
    worker_id: usize,
    request_path: PathBuf,
    child: Option<Child>,
}

fn cancel_workers(children: &mut [SpawnedWorker], start: usize) {
    for worker in &mut children[start..] {
        if let Some(child) = worker.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        worker.child = None;
        let _ = std::fs::remove_file(&worker.request_path);
    }
}

fn run_worker_request(request: &LiveIndexWorkerRequest) -> Result<IndexRunMetrics, LiveIndexError> {
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
    let content = open_content_db(&request.content_path).map_err(|source| {
        LiveIndexError::ContentDatabase {
            path: request.content_path.clone(),
            source,
        }
    })?;
    let control = open_control_db(&request.control_path)?;
    index_entries_with_provider(&content, &control, provider.as_ref(), request)
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
    request: &LiveIndexWorkerRequest,
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
        )?;
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
            )?;
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
    use std::sync::Mutex;
    use std::time::Duration;

    use litradar_domain::{
        ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft,
        JournalRankings, ProviderBatch,
    };
    use litradar_provider::{IndexContentProvider, ProviderError};
    use tempfile::tempdir;

    use super::{
        build_worker_requests, index_entries_with_provider, run_live_index,
        run_live_index_worker_from_file_path, validate_live_config, LeaseHeartbeat,
        LiveIndexConfig, LiveIndexError, LiveIndexWorkerRequest, LiveRunTime,
        OPENALEX_MAX_WORKERS_PER_PROCESS,
    };
    use crate::control::{
        acquire_lease, open_control_db, read_checkpoint, release_lease, CheckpointScope,
    };
    use crate::schema::open_content_db;

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

    fn worker_request(
        root: &std::path::Path,
        provider_name: &str,
        run_id: &str,
    ) -> LiveIndexWorkerRequest {
        LiveIndexWorkerRequest {
            control_path: root.join("control.sqlite"),
            content_path: root.join("content.sqlite"),
            catalog_name: "chinese_journals".to_string(),
            provider_name: provider_name.to_string(),
            run_id: run_id.to_string(),
            timestamp: "2026-07-18T00:00:00Z".to_string(),
            worker_id: 0,
            process_count: 1,
            source_worker_count: 1,
            schedule_epoch_unix_millis: 0,
            timeout_seconds: 10,
            resume: true,
            update: false,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            entries: vec![catalog("journal-1")],
        }
    }

    #[test]
    fn provider_switch_uses_new_checkpoint_namespace_and_same_content_ids() {
        let directory = tempdir().expect("temporary directory should create");
        let provider = StaticProvider::new();
        let request_a = worker_request(directory.path(), "provider-a", "run-a");
        let content = open_content_db(&request_a.content_path).expect("content should open");
        let control = open_control_db(&request_a.control_path).expect("control should open");
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

        let request_b = worker_request(directory.path(), "provider-b", "run-b");
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
        let request = worker_request(directory.path(), "provider-a", "run-a");
        let content = open_content_db(&request.content_path).expect("content should open");
        let control = open_control_db(&request.control_path).expect("control should open");
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
        std::fs::remove_file(&request.control_path).expect("control database should delete");
        let replay_control =
            open_control_db(&request.control_path).expect("control should recreate");
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
        let requests = build_worker_requests(
            &config,
            std::path::Path::new("control.sqlite"),
            std::path::Path::new("content.sqlite"),
            "catalog",
            "scholarly",
            "run",
            "time",
            123_456,
            &entries,
        );
        assert_eq!(requests.len(), 3);
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
            .flat_map(|request| request.entries.iter())
            .map(|entry| entry.catalog_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), entries.len());
        assert!(requests.iter().all(|request| !request.entries.is_empty()));
    }

    #[test]
    fn worker_file_entrypoint_returns_one_json_value_on_stdout_boundary() {
        let directory = tempdir().expect("temporary directory should create");
        let mut request = worker_request(directory.path(), "scholarly", "run-worker");
        request.entries.clear();
        let request_path = directory.path().join("worker-request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&request).expect("worker request should serialize"),
        )
        .expect("worker request should write");

        let response = run_live_index_worker_from_file_path(&request_path)
            .expect("worker entrypoint should return JSON");
        let payload: serde_json::Value =
            serde_json::from_str(&response).expect("worker response should be one JSON value");

        assert_eq!(payload["worker_id"], 0);
        assert_eq!(payload["status"], "succeeded");
        assert_eq!(payload["metrics"]["journals_total"], 0);
        assert!(payload["error"].is_null());
        assert_eq!(response.lines().count(), 1);
    }

    #[test]
    fn parent_heartbeat_renews_provider_scoped_lease() {
        let directory = tempdir().expect("temporary directory should create");
        let control_path = directory.path().join("control.sqlite");
        let control = open_control_db(&control_path).expect("control should open");
        let now = LiveRunTime::now().epoch_seconds;
        acquire_lease(&control, "catalog", "provider", "run", now).expect("lease should acquire");
        let mut heartbeat = LeaseHeartbeat::start(
            control_path,
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
