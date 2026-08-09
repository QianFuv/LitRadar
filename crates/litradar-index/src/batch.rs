//! Disposable project-level index batch orchestration storage.

use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use litradar_domain::{IndexSyncMode, JournalCatalogEntry};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::transforms::{parse_catalog_csv, CatalogContractError};

/// Current disposable project batch database schema version.
pub(crate) const BATCH_SCHEMA_VERSION: i64 = 2;

/// Stable filename for the project-level batch database.
pub(crate) const BATCH_DATABASE_FILE_NAME: &str = "index-batches.sqlite";

const BATCH_BUSY_TIMEOUT_SECONDS: u64 = 30;
const BATCH_LEASE_DURATION_SECONDS: i64 = 300;
const MAX_IDENTIFIER_BYTES: usize = 512;
const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
const FINGERPRINT_VERSION: &str = "litradar-index-batch-v1";
static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// How the current command selected maintained catalog files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogSelection {
    /// Every CSV discovered under the managed metadata directory.
    All,
    /// One basename selected explicitly with `--file`.
    ExplicitFile,
}

impl CatalogSelection {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ExplicitFile => "explicit_file",
        }
    }

    fn parse(value: &str) -> Result<Self, BatchDatabaseError> {
        match value {
            "all" => Ok(Self::All),
            "explicit_file" => Ok(Self::ExplicitFile),
            _ => Err(BatchDatabaseError::InvalidState {
                reason: "stored catalog selection is invalid",
            }),
        }
    }
}

/// One maintained catalog frozen from a single exact file read.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CatalogInput {
    /// Original catalog path used only by the current invocation.
    pub(crate) path: PathBuf,
    /// Safe catalog basename persisted in batch state.
    pub(crate) file_name: String,
    /// Stable catalog stem.
    pub(crate) catalog_name: String,
    /// Exact source-byte SHA-256 retained only for compatibility checks.
    pub(crate) csv_sha256: String,
    /// Core-resolved Provider route.
    pub(crate) provider_name: String,
    /// Canonical entries parsed from the same bytes that were hashed.
    pub(crate) entries: Vec<JournalCatalogEntry>,
}

impl CatalogInput {
    /// Read, hash, and parse one maintained catalog exactly once.
    ///
    /// # Arguments
    ///
    /// * `path` - Selected maintained catalog path.
    /// * `provider_name` - Core-resolved Provider route for the catalog stem.
    ///
    /// # Returns
    ///
    /// Frozen input whose entries and digest came from the same byte snapshot.
    pub(crate) fn freeze(
        path: impl AsRef<Path>,
        provider_name: impl Into<String>,
    ) -> Result<Self, BatchDatabaseError> {
        let path = path.as_ref();
        let file_name = safe_file_name(path)?;
        let catalog_name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .ok_or(BatchDatabaseError::InvalidInput {
                reason: "catalog filename must have a non-empty UTF-8 stem",
            })?
            .to_string();
        let provider_name = provider_name.into();
        validate_identifier(
            &provider_name,
            "provider route must be non-empty and bounded",
        )?;
        let bytes = std::fs::read(path)?;
        let csv_sha256 = sha256_hex(&bytes);
        let text = std::str::from_utf8(&bytes).map_err(|source| {
            BatchDatabaseError::InvalidCatalogEncoding {
                file_name: file_name.clone(),
                source,
            }
        })?;
        let entries = parse_catalog_csv(text)?;
        Ok(Self {
            path: path.to_path_buf(),
            file_name,
            catalog_name,
            csv_sha256,
            provider_name,
            entries,
        })
    }
}

impl fmt::Debug for CatalogInput {
    /// Format a frozen catalog without exposing its full path or digest.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogInput")
            .field("file_name", &self.file_name)
            .field("catalog_name", &self.catalog_name)
            .field("provider_name", &self.provider_name)
            .field("journal_count", &self.entries.len())
            .finish()
    }
}

/// Correctness-sensitive inputs that define one resumable index batch.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct IndexBatchRequest {
    /// Frozen catalogs in deterministic execution order.
    pub(crate) catalogs: Vec<CatalogInput>,
    /// Whether discovery selected all CSVs or one explicit basename.
    pub(crate) selection: CatalogSelection,
    /// Core synchronization mode.
    pub(crate) mode: IndexSyncMode,
    /// Provider-side issue detail batch size.
    pub(crate) issue_batch_size: usize,
    /// Whether an update manifest must be published and handed to notify.
    pub(crate) notify: bool,
    /// Whether the notification handoff runs in dry-run mode.
    pub(crate) notify_dry_run: bool,
}

impl IndexBatchRequest {
    /// Build and fingerprint a validated ordered batch request.
    ///
    /// # Arguments
    ///
    /// * `catalogs` - Frozen catalogs in execution order.
    /// * `selection` - Catalog selection mechanism.
    /// * `mode` - Core synchronization mode.
    /// * `issue_batch_size` - Positive Provider issue batch size.
    /// * `notify` - Whether notification follows update publication.
    /// * `notify_dry_run` - Whether notification uses dry-run mode.
    ///
    /// # Returns
    ///
    /// A deterministic compatibility request.
    pub(crate) fn new(
        catalogs: Vec<CatalogInput>,
        selection: CatalogSelection,
        mode: IndexSyncMode,
        issue_batch_size: usize,
        notify: bool,
        notify_dry_run: bool,
    ) -> Result<Self, BatchDatabaseError> {
        if catalogs.is_empty() {
            return Err(BatchDatabaseError::InvalidInput {
                reason: "an index batch must contain at least one catalog",
            });
        }
        if selection == CatalogSelection::ExplicitFile && catalogs.len() != 1 {
            return Err(BatchDatabaseError::InvalidInput {
                reason: "an explicit-file batch must contain exactly one catalog",
            });
        }
        if issue_batch_size == 0 {
            return Err(BatchDatabaseError::InvalidInput {
                reason: "issue batch size must be greater than zero",
            });
        }
        let mut file_names = HashSet::new();
        let mut catalog_names = HashSet::new();
        for catalog in &catalogs {
            validate_identifier(
                &catalog.file_name,
                "catalog filename must be non-empty and bounded",
            )?;
            validate_identifier(
                &catalog.catalog_name,
                "catalog name must be non-empty and bounded",
            )?;
            validate_identifier(
                &catalog.provider_name,
                "provider route must be non-empty and bounded",
            )?;
            if catalog.csv_sha256.len() != 64
                || !catalog
                    .csv_sha256
                    .bytes()
                    .all(|value| value.is_ascii_hexdigit())
            {
                return Err(BatchDatabaseError::InvalidInput {
                    reason: "catalog digest must be a SHA-256 hexadecimal value",
                });
            }
            if !file_names.insert(catalog.file_name.clone()) {
                return Err(BatchDatabaseError::InvalidInput {
                    reason: "catalog filenames must be unique within one batch",
                });
            }
            if !catalog_names.insert(catalog.catalog_name.clone()) {
                return Err(BatchDatabaseError::InvalidInput {
                    reason: "catalog names must be unique within one batch",
                });
            }
        }
        Ok(Self {
            catalogs,
            selection,
            mode,
            issue_batch_size,
            notify,
            notify_dry_run,
        })
    }

    fn fingerprint(&self) -> String {
        batch_fingerprint(
            &self.catalogs,
            self.selection,
            self.mode,
            self.issue_batch_size,
            self.notify,
            self.notify_dry_run,
        )
    }
}

impl fmt::Debug for IndexBatchRequest {
    /// Format correctness inputs without exposing CSV digests or filesystem paths.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexBatchRequest")
            .field("catalogs", &self.catalogs)
            .field("selection", &self.selection)
            .field("mode", &self.mode)
            .field("issue_batch_size", &self.issue_batch_size)
            .field("notify", &self.notify)
            .field("notify_dry_run", &self.notify_dry_run)
            .finish()
    }
}

/// Persisted phase for one catalog in an active batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchCatalogPhase {
    /// No catalog-owned operation has started.
    Pending,
    /// Journal indexing or non-update outbox finalization is active.
    Indexing,
    /// Exact manifest bytes are durable in batch state.
    ManifestPrepared,
    /// Exact manifest bytes are published and the outbox cursor is acknowledged.
    ManifestPublished,
    /// Notification handoff is the only remaining catalog operation.
    Notifying,
    /// Every catalog operation completed successfully.
    Completed,
}

impl BatchCatalogPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Indexing => "indexing",
            Self::ManifestPrepared => "manifest_prepared",
            Self::ManifestPublished => "manifest_published",
            Self::Notifying => "notifying",
            Self::Completed => "completed",
        }
    }

    fn parse(value: &str) -> Result<Self, BatchDatabaseError> {
        match value {
            "pending" => Ok(Self::Pending),
            "indexing" => Ok(Self::Indexing),
            "manifest_prepared" => Ok(Self::ManifestPrepared),
            "manifest_published" => Ok(Self::ManifestPublished),
            "notifying" => Ok(Self::Notifying),
            "completed" => Ok(Self::Completed),
            _ => Err(BatchDatabaseError::InvalidState {
                reason: "stored catalog phase is invalid",
            }),
        }
    }
}

/// Typed result of one index-to-notify child attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotifyHandoffStatus {
    /// The attempt was persisted but has no trusted terminal child result yet.
    Running,
    /// The child found no delivery candidates.
    Idle,
    /// The child completed delivery successfully.
    Completed,
    /// The child intentionally skipped delivery.
    Skipped,
    /// The child failed with a known terminal outcome.
    Failed,
    /// The child was cancelled.
    Cancelled,
    /// The child exceeded its deadline.
    TimedOut,
    /// The child result or handoff protocol is ambiguous.
    Unknown,
}

impl NotifyHandoffStatus {
    /// Return the stable SQLite and handoff protocol representation.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Unknown => "unknown",
        }
    }

    /// Parse a stable SQLite and handoff protocol representation.
    pub(crate) fn parse(value: &str) -> Result<Self, BatchDatabaseError> {
        match value {
            "running" => Ok(Self::Running),
            "idle" => Ok(Self::Idle),
            "completed" => Ok(Self::Completed),
            "skipped" => Ok(Self::Skipped),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "unknown" => Ok(Self::Unknown),
            _ => Err(BatchDatabaseError::InvalidState {
                reason: "stored notification handoff status is invalid",
            }),
        }
    }

    /// Return whether the catalog may complete without another child process.
    pub(crate) fn is_success(self) -> bool {
        matches!(self, Self::Idle | Self::Completed | Self::Skipped)
    }

    fn can_start_new_attempt(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled | Self::TimedOut)
    }
}

/// Durable typed state for the latest notification handoff attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotifyHandoffState {
    /// Stable identifier passed to the child and delivery dedupe layer.
    pub(crate) attempt_id: String,
    /// Typed latest child or recovery state.
    pub(crate) status: NotifyHandoffStatus,
    /// Child process exit code when one was observed.
    pub(crate) exit_code: Option<i32>,
    /// Most recently acknowledged ambiguous attempt identifier.
    pub(crate) unknown_acknowledged_attempt_id: Option<String>,
    /// Unix timestamp of the most recent explicit Unknown acknowledgement.
    pub(crate) unknown_acknowledged_at: Option<i64>,
}

/// Policy result returned before an index invocation launches a notify child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotifyAttemptPreparation {
    /// Launch or resume the supplied stable attempt.
    Run(NotifyHandoffState),
    /// The prior attempt already reached a trusted success state.
    Succeeded(NotifyHandoffState),
    /// The prior attempt is ambiguous and requires explicit acknowledgement.
    BlockedUnknown(NotifyHandoffState),
}

/// Safe catalog outcome retained so a completed catalog can be returned without replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchCatalogOutcome {
    /// Core-owned catalog run identifier.
    pub(crate) run_id: String,
    /// Number of frozen journals selected for the catalog.
    pub(crate) journal_count: usize,
    /// New or changed canonical article count.
    pub(crate) written_article_count: i64,
    /// Canonical Provider page count.
    pub(crate) source_attempt_count: usize,
    /// Optional project-relative manifest path.
    pub(crate) manifest_path: Option<String>,
}

/// Exact recoverable intent for one update manifest.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ManifestIntent {
    /// Exact JSON bytes, including the terminal newline.
    pub(crate) payload: Vec<u8>,
    /// Exact payload SHA-256 retained for corruption checks.
    pub(crate) sha256: String,
    /// Inclusive content outbox cursor represented by the payload.
    pub(crate) through_event_id: Option<i64>,
    /// Project-relative final manifest path.
    pub(crate) path: String,
    /// Core-owned catalog run identifier serialized in the payload.
    pub(crate) run_id: String,
    /// Safe manifest generation timestamp serialized in the payload.
    pub(crate) generated_at: String,
}

impl ManifestIntent {
    /// Validate and construct an exact manifest publication intent.
    ///
    /// # Arguments
    ///
    /// * `payload` - Exact serialized manifest bytes.
    /// * `through_event_id` - Inclusive outbox cursor, when events were included.
    /// * `path` - Safe project-relative publication path.
    /// * `run_id` - Core-owned catalog run identifier.
    /// * `generated_at` - Safe generation timestamp.
    ///
    /// # Returns
    ///
    /// A bounded intent with a verified SHA-256 digest.
    pub(crate) fn new(
        payload: Vec<u8>,
        through_event_id: Option<i64>,
        path: impl Into<String>,
        run_id: impl Into<String>,
        generated_at: impl Into<String>,
    ) -> Result<Self, BatchDatabaseError> {
        if payload.is_empty() || payload.len() > MAX_MANIFEST_BYTES {
            return Err(BatchDatabaseError::InvalidInput {
                reason: "manifest payload must be non-empty and bounded",
            });
        }
        if through_event_id.is_some_and(|value| value <= 0) {
            return Err(BatchDatabaseError::InvalidInput {
                reason: "manifest outbox cursor must be positive",
            });
        }
        let path = path.into();
        validate_relative_path(&path)?;
        let run_id = run_id.into();
        let generated_at = generated_at.into();
        validate_identifier(
            &run_id,
            "manifest run identifier must be non-empty and bounded",
        )?;
        validate_identifier(
            &generated_at,
            "manifest timestamp must be non-empty and bounded",
        )?;
        Ok(Self {
            sha256: sha256_hex(&payload),
            payload,
            through_event_id,
            path,
            run_id,
            generated_at,
        })
    }
}

impl fmt::Debug for ManifestIntent {
    /// Format recoverable manifest metadata without exposing bytes, digest, or full path.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManifestIntent")
            .field("payload_bytes", &self.payload.len())
            .field("through_event_id", &self.through_event_id)
            .field("file_name", &Path::new(&self.path).file_name())
            .field("run_id", &self.run_id)
            .field("generated_at", &self.generated_at)
            .finish()
    }
}

/// Persisted state for one catalog in a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexBatchCatalog {
    /// Stable zero-based execution ordinal.
    pub(crate) ordinal: usize,
    /// Safe catalog basename.
    pub(crate) file_name: String,
    /// Stable catalog stem.
    pub(crate) catalog_name: String,
    /// Core-resolved Provider route.
    pub(crate) provider_name: String,
    /// Number of journals frozen from the exact CSV bytes.
    pub(crate) journal_count: usize,
    /// Current durable catalog phase.
    pub(crate) phase: BatchCatalogPhase,
    /// Safe outcome fields when journal execution completed.
    pub(crate) outcome: Option<BatchCatalogOutcome>,
    /// Exact manifest intent when update publication was prepared.
    pub(crate) manifest_intent: Option<ManifestIntent>,
    /// Latest typed notification handoff state.
    pub(crate) notify_handoff: Option<NotifyHandoffState>,
}

/// One active or abandoning project-level batch owned by an invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexBatch {
    /// Core-owned safe batch correlation identifier.
    pub(crate) batch_id: String,
    /// Unique invocation owner holding the global lease.
    pub(crate) owner_id: String,
    /// Safe Unix start timestamp.
    pub(crate) started_at: i64,
    /// Whether this invocation adopted an existing compatible batch.
    pub(crate) did_resume: bool,
    /// Persisted catalogs in deterministic order.
    pub(crate) catalogs: Vec<IndexBatchCatalog>,
}

/// Result of admitting one command against the project batch database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchAdmission {
    /// A new or compatible active batch is ready to execute.
    Ready(IndexBatch),
    /// Owned checkpoints must be cleaned before replacing an abandoning batch.
    Abandoning(IndexBatch),
}

/// Safe correctness category that differed from an active batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BatchCompatibilityField {
    /// All-files and explicit-file selection differ.
    CatalogSelection,
    /// Ordered catalog basenames or stems differ.
    CatalogOrder,
    /// Exact bytes of at least one selected CSV differ.
    CatalogContent,
    /// At least one catalog routes to a different Provider.
    ProviderRoute,
    /// Core synchronization mode differs.
    SyncMode,
    /// Provider issue batch size differs.
    IssueBatchSize,
    /// Notification enablement differs.
    Notify,
    /// Notification dry-run mode differs.
    NotifyDryRun,
}

impl BatchCompatibilityField {
    fn as_str(self) -> &'static str {
        match self {
            Self::CatalogSelection => "catalog_selection",
            Self::CatalogOrder => "catalog_order",
            Self::CatalogContent => "catalog_content",
            Self::ProviderRoute => "provider_route",
            Self::SyncMode => "sync_mode",
            Self::IssueBatchSize => "issue_batch_size",
            Self::Notify => "notify",
            Self::NotifyDryRun => "notify_dry_run",
        }
    }
}

/// Project batch storage or compatibility failure.
#[derive(Debug)]
pub(crate) enum BatchDatabaseError {
    /// Filesystem setup or catalog reading failed.
    Io(std::io::Error),
    /// Canonical maintained catalog parsing failed.
    Catalog(CatalogContractError),
    /// A catalog was not valid UTF-8.
    InvalidCatalogEncoding {
        /// Safe basename of the invalid catalog.
        file_name: String,
        /// UTF-8 decoding failure.
        source: std::str::Utf8Error,
    },
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// A newer batch schema was opened by an older binary.
    UnsupportedVersion {
        /// Version stored by the batch database.
        found: i64,
        /// Highest version supported by this binary.
        supported: i64,
    },
    /// Correctness-sensitive request fields differ from the active batch.
    CompatibilityMismatch {
        /// Safe differing field categories without values or digests.
        fields: Vec<BatchCompatibilityField>,
    },
    /// Another invocation owns the unexpired global batch lease.
    ActiveLease {
        /// Safe invocation owner identifier.
        owner_id: String,
        /// Lease expiry as Unix seconds.
        expires_at: i64,
    },
    /// The requested invocation no longer owns the global batch lease.
    OwnershipLost {
        /// Safe invocation owner identifier.
        owner_id: String,
    },
    /// Default resume encountered an incompletely abandoned batch.
    AbandonmentPending,
    /// Disabling resume would discard a published notification handoff.
    PublishedNotificationPending,
    /// A caller supplied invalid bounded input.
    InvalidInput {
        /// Fixed safe validation reason.
        reason: &'static str,
    },
    /// Durable disposable state violated an invariant.
    InvalidState {
        /// Fixed safe invariant reason.
        reason: &'static str,
    },
}

impl fmt::Display for BatchDatabaseError {
    /// Format a diagnostic without exposing digests, payloads, or runtime secrets.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Catalog(error) => write!(formatter, "{error}"),
            Self::InvalidCatalogEncoding { file_name, .. } => {
                write!(formatter, "catalog {file_name} is not valid UTF-8")
            }
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported index batch schema version {found}; maximum supported is {supported}"
            ),
            Self::CompatibilityMismatch { fields } => {
                let fields = fields
                    .iter()
                    .map(|field| field.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    formatter,
                    "active index batch is incompatible in: {fields}; restore the original inputs or disable resume"
                )
            }
            Self::ActiveLease {
                owner_id,
                expires_at,
            } => write!(
                formatter,
                "index batch is owned by active invocation {owner_id} until {expires_at}"
            ),
            Self::OwnershipLost { owner_id } => write!(
                formatter,
                "index invocation {owner_id} no longer owns the project batch lease"
            ),
            Self::AbandonmentPending => formatter
                .write_str("an index batch abandonment is incomplete; retry with resume disabled"),
            Self::PublishedNotificationPending => formatter.write_str(
                "active index batch has a published notification handoff; resume it and acknowledge Unknown if required before disabling resume",
            ),
            Self::InvalidInput { reason } => {
                write!(formatter, "invalid index batch input: {reason}")
            }
            Self::InvalidState { reason } => {
                write!(formatter, "invalid disposable index batch state: {reason}")
            }
        }
    }
}

impl Error for BatchDatabaseError {
    /// Return the underlying IO, catalog, UTF-8, or SQLite failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Catalog(error) => Some(error),
            Self::InvalidCatalogEncoding { source, .. } => Some(source),
            Self::Sqlite(error) => Some(error),
            Self::UnsupportedVersion { .. }
            | Self::CompatibilityMismatch { .. }
            | Self::ActiveLease { .. }
            | Self::OwnershipLost { .. }
            | Self::AbandonmentPending
            | Self::PublishedNotificationPending
            | Self::InvalidInput { .. }
            | Self::InvalidState { .. } => None,
        }
    }
}

impl From<std::io::Error> for BatchDatabaseError {
    /// Convert filesystem failures into batch database errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CatalogContractError> for BatchDatabaseError {
    /// Convert maintained catalog failures into batch database errors.
    fn from(error: CatalogContractError) -> Self {
        Self::Catalog(error)
    }
}

impl From<rusqlite::Error> for BatchDatabaseError {
    /// Convert SQLite failures into batch database errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchStatus {
    Active,
    Abandoning,
    Completed,
    Abandoned,
}

impl BatchStatus {
    fn parse(value: &str) -> Result<Self, BatchDatabaseError> {
        match value {
            "active" => Ok(Self::Active),
            "abandoning" => Ok(Self::Abandoning),
            "completed" => Ok(Self::Completed),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(BatchDatabaseError::InvalidState {
                reason: "stored batch status is invalid",
            }),
        }
    }
}

#[derive(Debug)]
struct StoredBatchHeader {
    batch_id: String,
    status: BatchStatus,
    fingerprint: String,
    selection: CatalogSelection,
    mode: IndexSyncMode,
    issue_batch_size: usize,
    notify: bool,
    notify_dry_run: bool,
}

#[derive(Debug)]
struct StoredCatalogDescriptor {
    file_name: String,
    catalog_name: String,
    csv_sha256: String,
    provider_name: String,
}

/// Generate one safe unique invocation owner identifier.
///
/// # Returns
///
/// A process-local and time-qualified correlation identifier.
pub(crate) fn new_batch_owner_id() -> String {
    unique_id("index-owner")
}

/// Create a process-unique notification attempt identifier.
pub(crate) fn new_notify_attempt_id() -> String {
    sha256_hex(unique_id("notify-attempt").as_bytes())[..32].to_string()
}

/// Open or initialize the disposable project batch database.
///
/// # Arguments
///
/// * `path` - Batch database path under `data/index-control`.
///
/// # Returns
///
/// Initialized SQLite connection.
pub(crate) fn open_batch_db(path: impl AsRef<Path>) -> Result<Connection, BatchDatabaseError> {
    if let Some(parent) = path
        .as_ref()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(BATCH_BUSY_TIMEOUT_SECONDS))?;
    init_batch_db(&connection)?;
    Ok(connection)
}

/// Initialize an empty batch database or validate its supported version.
///
/// # Arguments
///
/// * `connection` - Open disposable batch database.
///
/// # Returns
///
/// Success after schema initialization or version validation.
pub(crate) fn init_batch_db(connection: &Connection) -> Result<(), BatchDatabaseError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if version > BATCH_SCHEMA_VERSION {
        return Err(BatchDatabaseError::UnsupportedVersion {
            found: version,
            supported: BATCH_SCHEMA_VERSION,
        });
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    if version == BATCH_SCHEMA_VERSION {
        return Ok(());
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    if version == 0 {
        transaction.execute_batch(
            "CREATE TABLE index_batches (
             batch_id TEXT PRIMARY KEY CHECK (length(batch_id) BETWEEN 1 AND 512),
             status TEXT NOT NULL
                 CHECK (status IN ('active', 'abandoning', 'completed', 'abandoned')),
             fingerprint TEXT NOT NULL CHECK (length(fingerprint) = 64),
             selection_kind TEXT NOT NULL
                 CHECK (selection_kind IN ('all', 'explicit_file')),
             sync_mode TEXT NOT NULL
                 CHECK (sync_mode IN ('bootstrap', 'incremental', 'full_rescan')),
             issue_batch_size INTEGER NOT NULL CHECK (issue_batch_size > 0),
             notify INTEGER NOT NULL CHECK (notify IN (0, 1)),
             notify_dry_run INTEGER NOT NULL CHECK (notify_dry_run IN (0, 1)),
             started_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             completed_at INTEGER
         );

         CREATE UNIQUE INDEX index_batches_one_active
             ON index_batches ((1))
             WHERE status IN ('active', 'abandoning');

         CREATE TABLE index_batch_catalogs (
             batch_id TEXT NOT NULL,
             ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
             file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 512),
             catalog_name TEXT NOT NULL CHECK (length(catalog_name) BETWEEN 1 AND 512),
             csv_sha256 TEXT NOT NULL CHECK (length(csv_sha256) = 64),
             provider_name TEXT NOT NULL CHECK (length(provider_name) BETWEEN 1 AND 512),
             journal_count INTEGER NOT NULL CHECK (journal_count >= 0),
             phase TEXT NOT NULL CHECK (phase IN (
                 'pending', 'indexing', 'manifest_prepared', 'manifest_published',
                 'notifying', 'completed'
             )),
             run_id TEXT CHECK (run_id IS NULL OR length(run_id) BETWEEN 1 AND 512),
             written_article_count INTEGER CHECK (
                 written_article_count IS NULL OR written_article_count >= 0
             ),
             source_attempt_count INTEGER CHECK (
                 source_attempt_count IS NULL OR source_attempt_count >= 0
             ),
             outcome_manifest_path TEXT,
             notify_attempt_id TEXT CHECK (
                 notify_attempt_id IS NULL OR length(notify_attempt_id) BETWEEN 1 AND 512
             ),
             notify_status TEXT CHECK (notify_status IS NULL OR notify_status IN (
                 'running', 'idle', 'completed', 'skipped', 'failed', 'cancelled',
                 'timed_out', 'unknown'
             )),
             notify_exit_code INTEGER,
             notify_unknown_acknowledged_attempt_id TEXT CHECK (
                 notify_unknown_acknowledged_attempt_id IS NULL
                 OR length(notify_unknown_acknowledged_attempt_id) BETWEEN 1 AND 512
             ),
             notify_unknown_acknowledged_at INTEGER CHECK (
                 notify_unknown_acknowledged_at IS NULL OR notify_unknown_acknowledged_at >= 0
             ),
             manifest_payload BLOB CHECK (
                 manifest_payload IS NULL OR length(manifest_payload) BETWEEN 1 AND 67108864
             ),
             manifest_sha256 TEXT CHECK (
                 manifest_sha256 IS NULL OR length(manifest_sha256) = 64
             ),
             manifest_through_event_id INTEGER CHECK (
                 manifest_through_event_id IS NULL OR manifest_through_event_id > 0
             ),
             manifest_path TEXT,
             manifest_run_id TEXT CHECK (
                 manifest_run_id IS NULL OR length(manifest_run_id) BETWEEN 1 AND 512
             ),
             manifest_generated_at TEXT CHECK (
                 manifest_generated_at IS NULL OR length(manifest_generated_at) BETWEEN 1 AND 512
             ),
             updated_at INTEGER NOT NULL,
             completed_at INTEGER,
             PRIMARY KEY (batch_id, ordinal),
             UNIQUE (batch_id, file_name),
             UNIQUE (batch_id, catalog_name),
             FOREIGN KEY (batch_id) REFERENCES index_batches(batch_id) ON DELETE CASCADE
         );

         CREATE TABLE index_batch_lease (
             lease_key INTEGER PRIMARY KEY CHECK (lease_key = 1),
             batch_id TEXT NOT NULL,
             owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 512),
             heartbeat_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             FOREIGN KEY (batch_id) REFERENCES index_batches(batch_id) ON DELETE CASCADE
         );

         CREATE INDEX index_batch_catalogs_phase
             ON index_batch_catalogs(batch_id, phase);",
        )?;
    } else if version == 1 {
        transaction.execute_batch(
            "ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_attempt_id TEXT CHECK (
                     notify_attempt_id IS NULL OR length(notify_attempt_id) BETWEEN 1 AND 512
                 );
             ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_status TEXT CHECK (notify_status IS NULL OR notify_status IN (
                     'running', 'idle', 'completed', 'skipped', 'failed', 'cancelled',
                     'timed_out', 'unknown'
                 ));
             ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_unknown_acknowledged_attempt_id TEXT CHECK (
                     notify_unknown_acknowledged_attempt_id IS NULL
                     OR length(notify_unknown_acknowledged_attempt_id) BETWEEN 1 AND 512
                 );
             ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_unknown_acknowledged_at INTEGER CHECK (
                     notify_unknown_acknowledged_at IS NULL
                     OR notify_unknown_acknowledged_at >= 0
                 );
             UPDATE index_batch_catalogs
             SET notify_attempt_id = 'legacy-notify-' || lower(hex(randomblob(16))),
                 notify_status = CASE
                     WHEN phase = 'notifying' THEN 'unknown'
                     WHEN phase = 'completed' AND notify_exit_code = 0 THEN 'completed'
                     ELSE 'unknown'
                 END
             WHERE phase = 'notifying' OR notify_exit_code IS NOT NULL;",
        )?;
    } else {
        return Err(BatchDatabaseError::UnsupportedVersion {
            found: version,
            supported: BATCH_SCHEMA_VERSION,
        });
    }
    transaction.pragma_update(None, "user_version", BATCH_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Admit a command against a new or existing project batch.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `request` - Frozen correctness-sensitive request.
/// * `should_resume` - Whether a compatible active batch may be reused.
/// * `owner_id` - Unique invocation owner identifier.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// A ready batch or an abandonment-cleanup gate.
pub(crate) fn admit_batch(
    connection: &Connection,
    request: &IndexBatchRequest,
    should_resume: bool,
    owner_id: &str,
    now: i64,
) -> Result<BatchAdmission, BatchDatabaseError> {
    validate_identifier(
        owner_id,
        "batch owner identifier must be non-empty and bounded",
    )?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let active = read_active_batch_header(&transaction)?;
    let admission = match active {
        None => {
            let batch_id = unique_id("index-batch");
            insert_batch(&transaction, &batch_id, request, now)?;
            claim_batch_lease(&transaction, &batch_id, owner_id, now)?;
            BatchAdmission::Ready(load_batch(&transaction, &batch_id, owner_id, false)?)
        }
        Some(active) if active.status == BatchStatus::Abandoning => {
            if should_resume {
                return Err(BatchDatabaseError::AbandonmentPending);
            }
            claim_batch_lease(&transaction, &active.batch_id, owner_id, now)?;
            transaction.execute(
                "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
                params![active.batch_id, now],
            )?;
            BatchAdmission::Abandoning(load_batch(&transaction, &active.batch_id, owner_id, true)?)
        }
        Some(active) if should_resume => {
            let mismatches = compatibility_mismatches(&transaction, &active, request)?;
            if !mismatches.is_empty() {
                return Err(BatchDatabaseError::CompatibilityMismatch { fields: mismatches });
            }
            claim_batch_lease(&transaction, &active.batch_id, owner_id, now)?;
            transaction.execute(
                "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
                params![active.batch_id, now],
            )?;
            BatchAdmission::Ready(load_batch(&transaction, &active.batch_id, owner_id, true)?)
        }
        Some(active) => {
            claim_batch_lease(&transaction, &active.batch_id, owner_id, now)?;
            if active.notify && has_pending_published_notification(&transaction, &active.batch_id)?
            {
                return Err(BatchDatabaseError::PublishedNotificationPending);
            }
            let changed = transaction.execute(
                "UPDATE index_batches
                 SET status = 'abandoning', updated_at = ?2
                 WHERE batch_id = ?1 AND status = 'active'",
                params![active.batch_id, now],
            )?;
            if changed != 1 {
                return Err(BatchDatabaseError::InvalidState {
                    reason: "active batch could not enter abandonment",
                });
            }
            BatchAdmission::Abandoning(load_batch(&transaction, &active.batch_id, owner_id, false)?)
        }
    };
    transaction.commit()?;
    Ok(admission)
}

/// Finish checkpoint cleanup for an abandoning batch and create its replacement.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `abandoning_batch_id` - Batch whose owned checkpoints were cleaned.
/// * `request` - Frozen replacement request.
/// * `owner_id` - Invocation that owns the abandoning batch lease.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// A fresh active batch that retains the abandoned batch history.
pub(crate) fn replace_abandoning_batch(
    connection: &Connection,
    abandoning_batch_id: &str,
    request: &IndexBatchRequest,
    owner_id: &str,
    now: i64,
) -> Result<IndexBatch, BatchDatabaseError> {
    validate_identifier(
        owner_id,
        "batch owner identifier must be non-empty and bounded",
    )?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, abandoning_batch_id, owner_id, now)?;
    let changed = transaction.execute(
        "UPDATE index_batches
         SET status = 'abandoned', updated_at = ?2, completed_at = ?2
         WHERE batch_id = ?1 AND status = 'abandoning'",
        params![abandoning_batch_id, now],
    )?;
    if changed != 1 {
        return Err(BatchDatabaseError::InvalidState {
            reason: "batch replacement requires an abandoning batch",
        });
    }
    let batch_id = unique_id("index-batch");
    insert_batch(&transaction, &batch_id, request, now)?;
    let expires_at = lease_expiry(now);
    let changed = transaction.execute(
        "UPDATE index_batch_lease
         SET batch_id = ?1, heartbeat_at = ?3, expires_at = ?4
         WHERE lease_key = 1 AND batch_id = ?2 AND owner_id = ?5",
        params![batch_id, abandoning_batch_id, now, expires_at, owner_id],
    )?;
    if changed != 1 {
        return Err(BatchDatabaseError::OwnershipLost {
            owner_id: owner_id.to_string(),
        });
    }
    let batch = load_batch(&transaction, &batch_id, owner_id, false)?;
    transaction.commit()?;
    Ok(batch)
}

/// Renew the global project batch lease.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Success when the same unexpired owner remains fenced.
pub(crate) fn heartbeat_batch_lease(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let changed = connection.execute(
        "UPDATE index_batch_lease
         SET heartbeat_at = ?3, expires_at = ?4
         WHERE lease_key = 1 AND batch_id = ?1 AND owner_id = ?2 AND expires_at > ?3",
        params![batch_id, owner_id, now, lease_expiry(now)],
    )?;
    if changed != 1 {
        return Err(BatchDatabaseError::OwnershipLost {
            owner_id: owner_id.to_string(),
        });
    }
    Ok(())
}

/// Release the global project batch lease idempotently.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active or abandoning batch identifier.
/// * `owner_id` - Current invocation owner.
///
/// # Returns
///
/// Success when the caller released its lease or it was already absent.
pub(crate) fn release_batch_lease(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
) -> Result<(), BatchDatabaseError> {
    let changed = connection.execute(
        "DELETE FROM index_batch_lease
         WHERE lease_key = 1 AND batch_id = ?1 AND owner_id = ?2",
        params![batch_id, owner_id],
    )?;
    if changed == 1 {
        return Ok(());
    }
    let current = connection
        .query_row(
            "SELECT owner_id FROM index_batch_lease WHERE lease_key = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if current.is_none() {
        Ok(())
    } else {
        Err(BatchDatabaseError::OwnershipLost {
            owner_id: owner_id.to_string(),
        })
    }
}

/// Reload persisted catalogs for one batch.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Batch identifier.
///
/// # Returns
///
/// Catalog states in stable ordinal order.
pub(crate) fn read_batch_catalogs(
    connection: &Connection,
    batch_id: &str,
) -> Result<Vec<IndexBatchCatalog>, BatchDatabaseError> {
    load_batch_catalogs(connection, batch_id)
}

/// Move one catalog through a valid durable phase transition.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `ordinal` - Stable catalog ordinal.
/// * `next_phase` - Requested next phase.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Success after an idempotent or forward-only transition.
pub(crate) fn transition_catalog_phase(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    ordinal: usize,
    next_phase: BatchCatalogPhase,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    let current = read_catalog_phase(&transaction, batch_id, ordinal)?;
    if current != next_phase && !is_valid_phase_transition(current, next_phase) {
        return Err(BatchDatabaseError::InvalidState {
            reason: "catalog phase transition is not allowed",
        });
    }
    transaction.execute(
        "UPDATE index_batch_catalogs
         SET phase = ?3, updated_at = ?4
         WHERE batch_id = ?1 AND ordinal = ?2",
        params![batch_id, usize_to_i64(ordinal)?, next_phase.as_str(), now],
    )?;
    transaction.execute(
        "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
        params![batch_id, now],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Atomically persist exact manifest intent and enter the prepared phase.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `ordinal` - Stable catalog ordinal.
/// * `intent` - Exact bounded publication intent.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Success when the same intent is durable in `manifest_prepared` or a later phase.
pub(crate) fn store_manifest_intent(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    ordinal: usize,
    intent: &ManifestIntent,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    validate_manifest_intent(intent)?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    let current_phase = read_catalog_phase(&transaction, batch_id, ordinal)?;
    let current_intent = read_manifest_intent(&transaction, batch_id, ordinal)?;
    match current_intent {
        Some(current) if current == *intent => {
            if current_phase == BatchCatalogPhase::Indexing {
                return Err(BatchDatabaseError::InvalidState {
                    reason: "stored manifest intent has no prepared catalog phase",
                });
            }
        }
        Some(_) => {
            return Err(BatchDatabaseError::InvalidState {
                reason: "catalog already has a different manifest intent",
            });
        }
        None if current_phase == BatchCatalogPhase::Indexing => {
            transaction.execute(
                "UPDATE index_batch_catalogs
                 SET phase = 'manifest_prepared', manifest_payload = ?3,
                     manifest_sha256 = ?4, manifest_through_event_id = ?5,
                     manifest_path = ?6, manifest_run_id = ?7,
                     manifest_generated_at = ?8, updated_at = ?9
                 WHERE batch_id = ?1 AND ordinal = ?2",
                params![
                    batch_id,
                    usize_to_i64(ordinal)?,
                    intent.payload,
                    intent.sha256,
                    intent.through_event_id,
                    intent.path,
                    intent.run_id,
                    intent.generated_at,
                    now,
                ],
            )?;
        }
        None => {
            return Err(BatchDatabaseError::InvalidState {
                reason: "manifest intent requires an indexing catalog",
            });
        }
    }
    transaction.execute(
        "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
        params![batch_id, now],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Persist safe catalog execution counters without advancing finalization.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `ordinal` - Stable catalog ordinal.
/// * `outcome` - Safe execution outcome.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Success after the outcome is inserted or monotonically enriched.
pub(crate) fn store_catalog_outcome(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    ordinal: usize,
    outcome: &BatchCatalogOutcome,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    save_catalog_outcome(&transaction, batch_id, ordinal, outcome, now)?;
    transaction.execute(
        "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
        params![batch_id, now],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Prepare the only notification attempt that the current invocation may launch.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `ordinal` - Stable catalog ordinal.
/// * `new_attempt_id` - Fresh identifier used only when policy permits a new attempt.
/// * `should_acknowledge_unknown` - Whether the operator explicitly acknowledged Unknown.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// A runnable stable attempt, prior success, or blocked ambiguous outcome.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_notify_attempt(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    ordinal: usize,
    new_attempt_id: &str,
    should_acknowledge_unknown: bool,
    now: i64,
) -> Result<NotifyAttemptPreparation, BatchDatabaseError> {
    validate_identifier(
        new_attempt_id,
        "notification attempt identifier must be non-empty and bounded",
    )?;
    if now < 0 {
        return Err(BatchDatabaseError::InvalidInput {
            reason: "notification attempt timestamp must not be negative",
        });
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    if read_catalog_phase(&transaction, batch_id, ordinal)? != BatchCatalogPhase::Notifying {
        return Err(BatchDatabaseError::InvalidState {
            reason: "notification attempt requires the notifying phase",
        });
    }
    let current = read_notify_handoff(&transaction, batch_id, ordinal)?;
    let (state, preparation, should_write) = match current {
        None => {
            let state = NotifyHandoffState {
                attempt_id: new_attempt_id.to_string(),
                status: NotifyHandoffStatus::Running,
                exit_code: None,
                unknown_acknowledged_attempt_id: None,
                unknown_acknowledged_at: None,
            };
            (state.clone(), NotifyAttemptPreparation::Run(state), true)
        }
        Some(current) if current.status == NotifyHandoffStatus::Running => (
            current.clone(),
            NotifyAttemptPreparation::Run(current),
            false,
        ),
        Some(current) if current.status.is_success() => (
            current.clone(),
            NotifyAttemptPreparation::Succeeded(current),
            false,
        ),
        Some(current) if current.status.can_start_new_attempt() => {
            if current.attempt_id == new_attempt_id {
                return Err(BatchDatabaseError::InvalidInput {
                    reason: "notification retry attempt identifier must be new",
                });
            }
            let state = NotifyHandoffState {
                attempt_id: new_attempt_id.to_string(),
                status: NotifyHandoffStatus::Running,
                exit_code: None,
                unknown_acknowledged_attempt_id: current.unknown_acknowledged_attempt_id,
                unknown_acknowledged_at: current.unknown_acknowledged_at,
            };
            (state.clone(), NotifyAttemptPreparation::Run(state), true)
        }
        Some(current) if !should_acknowledge_unknown => (
            current.clone(),
            NotifyAttemptPreparation::BlockedUnknown(current),
            false,
        ),
        Some(current) => {
            if current.attempt_id == new_attempt_id {
                return Err(BatchDatabaseError::InvalidInput {
                    reason: "notification acknowledged attempt identifier must be new",
                });
            }
            let state = NotifyHandoffState {
                attempt_id: new_attempt_id.to_string(),
                status: NotifyHandoffStatus::Running,
                exit_code: None,
                unknown_acknowledged_attempt_id: Some(current.attempt_id),
                unknown_acknowledged_at: Some(now),
            };
            (state.clone(), NotifyAttemptPreparation::Run(state), true)
        }
    };
    if should_write {
        save_notify_handoff(&transaction, batch_id, ordinal, &state, now)?;
        transaction.execute(
            "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
            params![batch_id, now],
        )?;
    }
    transaction.commit()?;
    Ok(preparation)
}

/// Persist the typed observation for the current stable notification attempt.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `ordinal` - Stable catalog ordinal.
/// * `attempt_id` - Attempt that produced the observation.
/// * `status` - Parsed or conservative typed result.
/// * `exit_code` - Child exit code when available.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Persisted latest handoff state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_notify_attempt_result(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    ordinal: usize,
    attempt_id: &str,
    status: NotifyHandoffStatus,
    exit_code: Option<i32>,
    now: i64,
) -> Result<NotifyHandoffState, BatchDatabaseError> {
    validate_identifier(
        attempt_id,
        "notification attempt identifier must be non-empty and bounded",
    )?;
    if now < 0 {
        return Err(BatchDatabaseError::InvalidInput {
            reason: "notification attempt timestamp must not be negative",
        });
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    if read_catalog_phase(&transaction, batch_id, ordinal)? != BatchCatalogPhase::Notifying {
        return Err(BatchDatabaseError::InvalidState {
            reason: "notification result requires the notifying phase",
        });
    }
    let current = read_notify_handoff(&transaction, batch_id, ordinal)?.ok_or(
        BatchDatabaseError::InvalidState {
            reason: "notification result has no prepared attempt",
        },
    )?;
    if current.attempt_id != attempt_id {
        return Err(BatchDatabaseError::InvalidState {
            reason: "notification result attempt identifier is stale",
        });
    }
    if current.status != NotifyHandoffStatus::Running {
        if current.status == status && current.exit_code == exit_code {
            transaction.commit()?;
            return Ok(current);
        }
        return Err(BatchDatabaseError::InvalidState {
            reason: "notification attempt already has a different terminal result",
        });
    }
    let state = NotifyHandoffState {
        status,
        exit_code,
        ..current
    };
    save_notify_handoff(&transaction, batch_id, ordinal, &state, now)?;
    transaction.execute(
        "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
        params![batch_id, now],
    )?;
    transaction.commit()?;
    Ok(state)
}

/// Persist a catalog outcome and enter the completed phase atomically.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `ordinal` - Stable catalog ordinal.
/// * `outcome` - Final safe catalog outcome.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Success after an idempotent catalog completion.
pub(crate) fn complete_catalog(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    ordinal: usize,
    outcome: &BatchCatalogOutcome,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    let phase = read_catalog_phase(&transaction, batch_id, ordinal)?;
    if phase != BatchCatalogPhase::Completed
        && !is_valid_phase_transition(phase, BatchCatalogPhase::Completed)
    {
        return Err(BatchDatabaseError::InvalidState {
            reason: "catalog cannot complete from its current phase",
        });
    }
    if phase == BatchCatalogPhase::Notifying {
        let handoff = read_notify_handoff(&transaction, batch_id, ordinal)?.ok_or(
            BatchDatabaseError::InvalidState {
                reason: "notifying catalog has no notification handoff",
            },
        )?;
        if !handoff.status.is_success() {
            return Err(BatchDatabaseError::InvalidState {
                reason: "catalog cannot complete without a trusted notification result",
            });
        }
    }
    save_catalog_outcome(&transaction, batch_id, ordinal, outcome, now)?;
    transaction.execute(
        "UPDATE index_batch_catalogs
         SET phase = 'completed', completed_at = COALESCE(completed_at, ?3), updated_at = ?3
         WHERE batch_id = ?1 AND ordinal = ?2",
        params![batch_id, usize_to_i64(ordinal)?, now],
    )?;
    transaction.execute(
        "UPDATE index_batches SET updated_at = ?2 WHERE batch_id = ?1",
        params![batch_id, now],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Mark an all-catalog-success batch terminal and release its lease.
///
/// # Arguments
///
/// * `connection` - Open batch database.
/// * `batch_id` - Active batch identifier.
/// * `owner_id` - Current invocation owner.
/// * `now` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Success when every catalog was complete and the active marker was removed.
pub(crate) fn complete_batch(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    verify_batch_ownership(&transaction, batch_id, owner_id, now)?;
    let incomplete = transaction.query_row(
        "SELECT COUNT(*) FROM index_batch_catalogs
         WHERE batch_id = ?1 AND phase != 'completed'",
        params![batch_id],
        |row| row.get::<_, i64>(0),
    )?;
    if incomplete != 0 {
        return Err(BatchDatabaseError::InvalidState {
            reason: "batch cannot complete while a catalog is incomplete",
        });
    }
    let changed = transaction.execute(
        "UPDATE index_batches
         SET status = 'completed', updated_at = ?2, completed_at = ?2
         WHERE batch_id = ?1 AND status = 'active'",
        params![batch_id, now],
    )?;
    if changed != 1 {
        return Err(BatchDatabaseError::InvalidState {
            reason: "only an active batch can complete",
        });
    }
    let released = transaction.execute(
        "DELETE FROM index_batch_lease
         WHERE lease_key = 1 AND batch_id = ?1 AND owner_id = ?2",
        params![batch_id, owner_id],
    )?;
    if released != 1 {
        return Err(BatchDatabaseError::OwnershipLost {
            owner_id: owner_id.to_string(),
        });
    }
    transaction.commit()?;
    Ok(())
}

fn insert_batch(
    connection: &Connection,
    batch_id: &str,
    request: &IndexBatchRequest,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    connection.execute(
        "INSERT INTO index_batches (
             batch_id, status, fingerprint, selection_kind, sync_mode,
             issue_batch_size, notify, notify_dry_run, started_at, updated_at
         ) VALUES (?1, 'active', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            batch_id,
            request.fingerprint(),
            request.selection.as_str(),
            sync_mode_text(request.mode),
            usize_to_i64(request.issue_batch_size)?,
            request.notify,
            request.notify_dry_run,
            now,
        ],
    )?;
    for (ordinal, catalog) in request.catalogs.iter().enumerate() {
        connection.execute(
            "INSERT INTO index_batch_catalogs (
                 batch_id, ordinal, file_name, catalog_name, csv_sha256,
                 provider_name, journal_count, phase, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)",
            params![
                batch_id,
                usize_to_i64(ordinal)?,
                catalog.file_name,
                catalog.catalog_name,
                catalog.csv_sha256,
                catalog.provider_name,
                usize_to_i64(catalog.entries.len())?,
                now,
            ],
        )?;
    }
    Ok(())
}

fn claim_batch_lease(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let current = connection
        .query_row(
            "SELECT owner_id, expires_at FROM index_batch_lease WHERE lease_key = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    if let Some((current_owner, expires_at)) = current {
        if current_owner != owner_id && expires_at > now {
            return Err(BatchDatabaseError::ActiveLease {
                owner_id: current_owner,
                expires_at,
            });
        }
    }
    connection.execute(
        "INSERT INTO index_batch_lease (
             lease_key, batch_id, owner_id, heartbeat_at, expires_at
         ) VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(lease_key) DO UPDATE SET
             batch_id = excluded.batch_id,
             owner_id = excluded.owner_id,
             heartbeat_at = excluded.heartbeat_at,
             expires_at = excluded.expires_at",
        params![batch_id, owner_id, now, lease_expiry(now)],
    )?;
    Ok(())
}

fn verify_batch_ownership(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    let owns = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM index_batch_lease
             WHERE lease_key = 1 AND batch_id = ?1 AND owner_id = ?2 AND expires_at > ?3
         )",
        params![batch_id, owner_id, now],
        |row| row.get::<_, bool>(0),
    )?;
    if !owns {
        return Err(BatchDatabaseError::OwnershipLost {
            owner_id: owner_id.to_string(),
        });
    }
    Ok(())
}

fn read_active_batch_header(
    connection: &Connection,
) -> Result<Option<StoredBatchHeader>, BatchDatabaseError> {
    let row = connection
        .query_row(
            "SELECT
                 batch_id, status, fingerprint, selection_kind, sync_mode,
                 issue_batch_size, notify, notify_dry_run, started_at
             FROM index_batches
             WHERE status IN ('active', 'abandoning')",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            batch_id,
            status,
            fingerprint,
            selection,
            mode,
            issue_batch_size,
            notify,
            notify_dry_run,
            _started_at,
        )| {
            Ok(StoredBatchHeader {
                batch_id,
                status: BatchStatus::parse(&status)?,
                fingerprint,
                selection: CatalogSelection::parse(&selection)?,
                mode: parse_sync_mode(&mode)?,
                issue_batch_size: i64_to_usize(issue_batch_size)?,
                notify,
                notify_dry_run,
            })
        },
    )
    .transpose()
}

fn has_pending_published_notification(
    connection: &Connection,
    batch_id: &str,
) -> Result<bool, BatchDatabaseError> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM index_batch_catalogs
                 WHERE batch_id = ?1 AND phase != 'completed'
                   AND outcome_manifest_path IS NOT NULL
             )",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn compatibility_mismatches(
    connection: &Connection,
    active: &StoredBatchHeader,
    request: &IndexBatchRequest,
) -> Result<Vec<BatchCompatibilityField>, BatchDatabaseError> {
    if active.fingerprint == request.fingerprint() {
        return Ok(Vec::new());
    }
    let mut fields = BTreeSet::new();
    if active.selection != request.selection {
        fields.insert(BatchCompatibilityField::CatalogSelection);
    }
    if active.mode != request.mode {
        fields.insert(BatchCompatibilityField::SyncMode);
    }
    if active.issue_batch_size != request.issue_batch_size {
        fields.insert(BatchCompatibilityField::IssueBatchSize);
    }
    if active.notify != request.notify {
        fields.insert(BatchCompatibilityField::Notify);
    }
    if active.notify_dry_run != request.notify_dry_run {
        fields.insert(BatchCompatibilityField::NotifyDryRun);
    }
    let stored = read_stored_catalog_descriptors(connection, &active.batch_id)?;
    let same_order = stored.len() == request.catalogs.len()
        && stored
            .iter()
            .zip(&request.catalogs)
            .all(|(stored, requested)| {
                stored.file_name == requested.file_name
                    && stored.catalog_name == requested.catalog_name
            });
    if !same_order {
        fields.insert(BatchCompatibilityField::CatalogOrder);
    } else {
        for (stored, requested) in stored.iter().zip(&request.catalogs) {
            if stored.csv_sha256 != requested.csv_sha256 {
                fields.insert(BatchCompatibilityField::CatalogContent);
            }
            if stored.provider_name != requested.provider_name {
                fields.insert(BatchCompatibilityField::ProviderRoute);
            }
        }
    }
    if fields.is_empty() {
        return Err(BatchDatabaseError::InvalidState {
            reason: "batch fingerprint differs without a recognized compatibility field",
        });
    }
    Ok(fields.into_iter().collect())
}

fn read_stored_catalog_descriptors(
    connection: &Connection,
    batch_id: &str,
) -> Result<Vec<StoredCatalogDescriptor>, BatchDatabaseError> {
    let mut statement = connection.prepare(
        "SELECT file_name, catalog_name, csv_sha256, provider_name
         FROM index_batch_catalogs WHERE batch_id = ?1 ORDER BY ordinal",
    )?;
    let rows = statement
        .query_map(params![batch_id], |row| {
            Ok(StoredCatalogDescriptor {
                file_name: row.get(0)?,
                catalog_name: row.get(1)?,
                csv_sha256: row.get(2)?,
                provider_name: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn load_batch(
    connection: &Connection,
    batch_id: &str,
    owner_id: &str,
    did_resume: bool,
) -> Result<IndexBatch, BatchDatabaseError> {
    let started_at = connection
        .query_row(
            "SELECT started_at FROM index_batches WHERE batch_id = ?1",
            params![batch_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(BatchDatabaseError::InvalidState {
            reason: "batch row is missing",
        })?;
    Ok(IndexBatch {
        batch_id: batch_id.to_string(),
        owner_id: owner_id.to_string(),
        started_at,
        did_resume,
        catalogs: load_batch_catalogs(connection, batch_id)?,
    })
}

#[allow(clippy::type_complexity)]
fn load_batch_catalogs(
    connection: &Connection,
    batch_id: &str,
) -> Result<Vec<IndexBatchCatalog>, BatchDatabaseError> {
    let raw = {
        let mut statement = connection.prepare(
            "SELECT ordinal, file_name, catalog_name, provider_name, journal_count, phase
             FROM index_batch_catalogs WHERE batch_id = ?1 ORDER BY ordinal",
        )?;
        let rows = statement
            .query_map(params![batch_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    raw.into_iter()
        .map(
            |(ordinal, file_name, catalog_name, provider_name, journal_count, phase)| {
                let ordinal = i64_to_usize(ordinal)?;
                Ok(IndexBatchCatalog {
                    ordinal,
                    file_name,
                    catalog_name,
                    provider_name,
                    journal_count: i64_to_usize(journal_count)?,
                    phase: BatchCatalogPhase::parse(&phase)?,
                    outcome: read_catalog_outcome(connection, batch_id, ordinal)?,
                    manifest_intent: read_manifest_intent(connection, batch_id, ordinal)?,
                    notify_handoff: read_notify_handoff(connection, batch_id, ordinal)?,
                })
            },
        )
        .collect()
}

fn read_catalog_phase(
    connection: &Connection,
    batch_id: &str,
    ordinal: usize,
) -> Result<BatchCatalogPhase, BatchDatabaseError> {
    let phase = connection
        .query_row(
            "SELECT phase FROM index_batch_catalogs WHERE batch_id = ?1 AND ordinal = ?2",
            params![batch_id, usize_to_i64(ordinal)?],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(BatchDatabaseError::InvalidState {
            reason: "batch catalog row is missing",
        })?;
    BatchCatalogPhase::parse(&phase)
}

fn read_manifest_intent(
    connection: &Connection,
    batch_id: &str,
    ordinal: usize,
) -> Result<Option<ManifestIntent>, BatchDatabaseError> {
    let row = connection.query_row(
        "SELECT
             manifest_payload, manifest_sha256, manifest_through_event_id,
             manifest_path, manifest_run_id, manifest_generated_at
         FROM index_batch_catalogs WHERE batch_id = ?1 AND ordinal = ?2",
        params![batch_id, usize_to_i64(ordinal)?],
        |row| {
            Ok((
                row.get::<_, Option<Vec<u8>>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        },
    )?;
    let (payload, sha256, through_event_id, path, run_id, generated_at) = row;
    if payload.is_none()
        && sha256.is_none()
        && through_event_id.is_none()
        && path.is_none()
        && run_id.is_none()
        && generated_at.is_none()
    {
        return Ok(None);
    }
    let intent = ManifestIntent {
        payload: payload.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored manifest intent is incomplete",
        })?,
        sha256: sha256.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored manifest intent is incomplete",
        })?,
        through_event_id,
        path: path.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored manifest intent is incomplete",
        })?,
        run_id: run_id.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored manifest intent is incomplete",
        })?,
        generated_at: generated_at.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored manifest intent is incomplete",
        })?,
    };
    validate_manifest_intent(&intent)?;
    Ok(Some(intent))
}

fn read_notify_handoff(
    connection: &Connection,
    batch_id: &str,
    ordinal: usize,
) -> Result<Option<NotifyHandoffState>, BatchDatabaseError> {
    let row = connection.query_row(
        "SELECT
             notify_attempt_id, notify_status, notify_exit_code,
             notify_unknown_acknowledged_attempt_id, notify_unknown_acknowledged_at
         FROM index_batch_catalogs WHERE batch_id = ?1 AND ordinal = ?2",
        params![batch_id, usize_to_i64(ordinal)?],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i32>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        },
    )?;
    let (attempt_id, status, exit_code, acknowledged_attempt_id, acknowledged_at) = row;
    if attempt_id.is_none()
        && status.is_none()
        && exit_code.is_none()
        && acknowledged_attempt_id.is_none()
        && acknowledged_at.is_none()
    {
        return Ok(None);
    }
    let state = NotifyHandoffState {
        attempt_id: attempt_id.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored notification handoff is incomplete",
        })?,
        status: NotifyHandoffStatus::parse(status.as_deref().ok_or(
            BatchDatabaseError::InvalidState {
                reason: "stored notification handoff is incomplete",
            },
        )?)?,
        exit_code,
        unknown_acknowledged_attempt_id: acknowledged_attempt_id,
        unknown_acknowledged_at: acknowledged_at,
    };
    validate_notify_handoff(&state)?;
    Ok(Some(state))
}

fn save_notify_handoff(
    connection: &Connection,
    batch_id: &str,
    ordinal: usize,
    state: &NotifyHandoffState,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    validate_notify_handoff(state)?;
    let changed = connection.execute(
        "UPDATE index_batch_catalogs
         SET notify_attempt_id = ?3, notify_status = ?4, notify_exit_code = ?5,
             notify_unknown_acknowledged_attempt_id = ?6,
             notify_unknown_acknowledged_at = ?7, updated_at = ?8
         WHERE batch_id = ?1 AND ordinal = ?2",
        params![
            batch_id,
            usize_to_i64(ordinal)?,
            state.attempt_id,
            state.status.as_str(),
            state.exit_code,
            state.unknown_acknowledged_attempt_id,
            state.unknown_acknowledged_at,
            now,
        ],
    )?;
    if changed != 1 {
        return Err(BatchDatabaseError::InvalidState {
            reason: "batch catalog row is missing",
        });
    }
    Ok(())
}

fn read_catalog_outcome(
    connection: &Connection,
    batch_id: &str,
    ordinal: usize,
) -> Result<Option<BatchCatalogOutcome>, BatchDatabaseError> {
    let row = connection.query_row(
        "SELECT
             run_id, journal_count, written_article_count, source_attempt_count,
             outcome_manifest_path
         FROM index_batch_catalogs WHERE batch_id = ?1 AND ordinal = ?2",
        params![batch_id, usize_to_i64(ordinal)?],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )?;
    let (run_id, journal_count, written_article_count, source_attempt_count, manifest_path) = row;
    let Some(run_id) = run_id else {
        if written_article_count.is_some()
            || source_attempt_count.is_some()
            || manifest_path.is_some()
        {
            return Err(BatchDatabaseError::InvalidState {
                reason: "stored catalog outcome is incomplete",
            });
        }
        return Ok(None);
    };
    let outcome = BatchCatalogOutcome {
        run_id,
        journal_count: i64_to_usize(journal_count)?,
        written_article_count: written_article_count.ok_or(BatchDatabaseError::InvalidState {
            reason: "stored catalog outcome is incomplete",
        })?,
        source_attempt_count: i64_to_usize(source_attempt_count.ok_or(
            BatchDatabaseError::InvalidState {
                reason: "stored catalog outcome is incomplete",
            },
        )?)?,
        manifest_path,
    };
    validate_catalog_outcome(&outcome)?;
    Ok(Some(outcome))
}

fn save_catalog_outcome(
    connection: &Connection,
    batch_id: &str,
    ordinal: usize,
    outcome: &BatchCatalogOutcome,
    now: i64,
) -> Result<(), BatchDatabaseError> {
    validate_catalog_outcome(outcome)?;
    let phase = read_catalog_phase(connection, batch_id, ordinal)?;
    if phase == BatchCatalogPhase::Pending {
        return Err(BatchDatabaseError::InvalidState {
            reason: "catalog outcome cannot be stored before indexing starts",
        });
    }
    let merged = match read_catalog_outcome(connection, batch_id, ordinal)? {
        None => outcome.clone(),
        Some(existing) => merge_catalog_outcome(existing, outcome)?,
    };
    let changed = connection.execute(
        "UPDATE index_batch_catalogs
         SET run_id = ?3, written_article_count = ?4, source_attempt_count = ?5,
             outcome_manifest_path = ?6, updated_at = ?7
         WHERE batch_id = ?1 AND ordinal = ?2 AND journal_count = ?8",
        params![
            batch_id,
            usize_to_i64(ordinal)?,
            merged.run_id,
            merged.written_article_count,
            usize_to_i64(merged.source_attempt_count)?,
            merged.manifest_path,
            now,
            usize_to_i64(merged.journal_count)?,
        ],
    )?;
    if changed != 1 {
        return Err(BatchDatabaseError::InvalidState {
            reason: "catalog outcome journal count does not match the frozen CSV",
        });
    }
    Ok(())
}

fn merge_catalog_outcome(
    existing: BatchCatalogOutcome,
    requested: &BatchCatalogOutcome,
) -> Result<BatchCatalogOutcome, BatchDatabaseError> {
    if existing.run_id != requested.run_id
        || existing.journal_count != requested.journal_count
        || existing.written_article_count != requested.written_article_count
        || existing.source_attempt_count != requested.source_attempt_count
    {
        return Err(BatchDatabaseError::InvalidState {
            reason: "catalog outcome immutable counters changed during recovery",
        });
    }
    let manifest_path = merge_optional_value(
        existing.manifest_path,
        requested.manifest_path.clone(),
        "catalog manifest path changed during recovery",
    )?;
    Ok(BatchCatalogOutcome {
        manifest_path,
        ..existing
    })
}

fn merge_optional_value<Value: PartialEq>(
    existing: Option<Value>,
    requested: Option<Value>,
    reason: &'static str,
) -> Result<Option<Value>, BatchDatabaseError> {
    match (existing, requested) {
        (Some(existing), Some(requested)) if existing != requested => {
            Err(BatchDatabaseError::InvalidState { reason })
        }
        (Some(existing), _) => Ok(Some(existing)),
        (None, requested) => Ok(requested),
    }
}

fn validate_catalog_outcome(outcome: &BatchCatalogOutcome) -> Result<(), BatchDatabaseError> {
    validate_identifier(
        &outcome.run_id,
        "catalog run identifier must be non-empty and bounded",
    )?;
    if outcome.written_article_count < 0 {
        return Err(BatchDatabaseError::InvalidInput {
            reason: "written article count must not be negative",
        });
    }
    if let Some(path) = outcome.manifest_path.as_deref() {
        validate_relative_path(path)?;
    }
    Ok(())
}

fn validate_notify_handoff(state: &NotifyHandoffState) -> Result<(), BatchDatabaseError> {
    validate_identifier(
        &state.attempt_id,
        "notification attempt identifier must be non-empty and bounded",
    )?;
    if state.status.is_success() && state.exit_code != Some(0) {
        return Err(BatchDatabaseError::InvalidState {
            reason: "successful notification handoff must have a zero exit code",
        });
    }
    if state.status != NotifyHandoffStatus::Unknown
        && state.exit_code.is_some_and(|exit_code| exit_code == 0)
        && !state.status.is_success()
    {
        return Err(BatchDatabaseError::InvalidState {
            reason: "unsuccessful notification handoff cannot have a zero exit code",
        });
    }
    match (
        state.unknown_acknowledged_attempt_id.as_deref(),
        state.unknown_acknowledged_at,
    ) {
        (None, None) => {}
        (Some(attempt_id), Some(acknowledged_at)) => {
            validate_identifier(
                attempt_id,
                "acknowledged notification attempt identifier must be non-empty and bounded",
            )?;
            if acknowledged_at < 0 {
                return Err(BatchDatabaseError::InvalidState {
                    reason: "notification acknowledgement timestamp must not be negative",
                });
            }
        }
        _ => {
            return Err(BatchDatabaseError::InvalidState {
                reason: "stored notification acknowledgement is incomplete",
            });
        }
    }
    Ok(())
}

fn validate_manifest_intent(intent: &ManifestIntent) -> Result<(), BatchDatabaseError> {
    if intent.payload.is_empty() || intent.payload.len() > MAX_MANIFEST_BYTES {
        return Err(BatchDatabaseError::InvalidState {
            reason: "stored manifest payload is empty or too large",
        });
    }
    if intent.sha256.len() != 64 || sha256_hex(&intent.payload) != intent.sha256 {
        return Err(BatchDatabaseError::InvalidState {
            reason: "stored manifest payload digest is invalid",
        });
    }
    if intent.through_event_id.is_some_and(|value| value <= 0) {
        return Err(BatchDatabaseError::InvalidState {
            reason: "stored manifest outbox cursor is invalid",
        });
    }
    validate_relative_path(&intent.path)?;
    validate_identifier(
        &intent.run_id,
        "manifest run identifier must be non-empty and bounded",
    )?;
    validate_identifier(
        &intent.generated_at,
        "manifest timestamp must be non-empty and bounded",
    )?;
    Ok(())
}

fn is_valid_phase_transition(current: BatchCatalogPhase, next: BatchCatalogPhase) -> bool {
    matches!(
        (current, next),
        (BatchCatalogPhase::Pending, BatchCatalogPhase::Indexing)
            | (
                BatchCatalogPhase::Indexing,
                BatchCatalogPhase::ManifestPrepared
            )
            | (BatchCatalogPhase::Indexing, BatchCatalogPhase::Completed)
            | (
                BatchCatalogPhase::ManifestPrepared,
                BatchCatalogPhase::ManifestPublished
            )
            | (
                BatchCatalogPhase::ManifestPublished,
                BatchCatalogPhase::Notifying
            )
            | (
                BatchCatalogPhase::ManifestPublished,
                BatchCatalogPhase::Completed
            )
            | (BatchCatalogPhase::Notifying, BatchCatalogPhase::Completed)
    )
}

fn batch_fingerprint(
    catalogs: &[CatalogInput],
    selection: CatalogSelection,
    mode: IndexSyncMode,
    issue_batch_size: usize,
    notify: bool,
    notify_dry_run: bool,
) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, FINGERPRINT_VERSION.as_bytes());
    hash_field(&mut hasher, selection.as_str().as_bytes());
    hash_field(&mut hasher, sync_mode_text(mode).as_bytes());
    hash_field(&mut hasher, issue_batch_size.to_string().as_bytes());
    hash_field(&mut hasher, if notify { b"1" } else { b"0" });
    hash_field(&mut hasher, if notify_dry_run { b"1" } else { b"0" });
    for (ordinal, catalog) in catalogs.iter().enumerate() {
        hash_field(&mut hasher, ordinal.to_string().as_bytes());
        hash_field(&mut hasher, catalog.file_name.as_bytes());
        hash_field(&mut hasher, catalog.catalog_name.as_bytes());
        hash_field(&mut hasher, catalog.csv_sha256.as_bytes());
        hash_field(&mut hasher, catalog.provider_name.as_bytes());
    }
    hex_digest(hasher.finalize())
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(value);
}

fn sha256_hex(value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value);
    hex_digest(hasher.finalize())
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn safe_file_name(path: &Path) -> Result<String, BatchDatabaseError> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or(BatchDatabaseError::InvalidInput {
            reason: "catalog path must have a non-empty UTF-8 basename",
        })?;
    validate_identifier(file_name, "catalog filename must be non-empty and bounded")?;
    Ok(file_name.to_string())
}

fn validate_identifier(value: &str, reason: &'static str) -> Result<(), BatchDatabaseError> {
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES || value.chars().any(char::is_control)
    {
        return Err(BatchDatabaseError::InvalidInput { reason });
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), BatchDatabaseError> {
    validate_identifier(value, "manifest path must be non-empty and bounded")?;
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(BatchDatabaseError::InvalidInput {
            reason: "manifest path must remain project-relative",
        });
    }
    Ok(())
}

fn sync_mode_text(mode: IndexSyncMode) -> &'static str {
    match mode {
        IndexSyncMode::Bootstrap => "bootstrap",
        IndexSyncMode::Incremental => "incremental",
        IndexSyncMode::FullRescan => "full_rescan",
    }
}

fn parse_sync_mode(value: &str) -> Result<IndexSyncMode, BatchDatabaseError> {
    match value {
        "bootstrap" => Ok(IndexSyncMode::Bootstrap),
        "incremental" => Ok(IndexSyncMode::Incremental),
        "full_rescan" => Ok(IndexSyncMode::FullRescan),
        _ => Err(BatchDatabaseError::InvalidState {
            reason: "stored synchronization mode is invalid",
        }),
    }
}

fn usize_to_i64(value: usize) -> Result<i64, BatchDatabaseError> {
    i64::try_from(value).map_err(|_| BatchDatabaseError::InvalidInput {
        reason: "batch count exceeds SQLite integer capacity",
    })
}

fn i64_to_usize(value: i64) -> Result<usize, BatchDatabaseError> {
    usize::try_from(value).map_err(|_| BatchDatabaseError::InvalidState {
        reason: "stored batch count is invalid",
    })
}

fn lease_expiry(now: i64) -> i64 {
    now.saturating_add(BATCH_LEASE_DURATION_SECONDS)
}

fn unique_id(prefix: &str) -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let sequence = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}-{:x}-{}-{sequence}",
        elapsed.as_nanos(),
        std::process::id()
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use litradar_domain::{IndexSyncMode, JournalCatalogEntry, JournalRankings};
    use rusqlite::{Connection, OptionalExtension};
    use tempfile::tempdir;

    use super::{
        admit_batch, complete_batch, complete_catalog, heartbeat_batch_lease, init_batch_db,
        new_batch_owner_id, new_notify_attempt_id, open_batch_db, prepare_notify_attempt,
        read_batch_catalogs, record_notify_attempt_result, release_batch_lease,
        replace_abandoning_batch, store_catalog_outcome, store_manifest_intent,
        transition_catalog_phase, BatchAdmission, BatchCatalogOutcome, BatchCatalogPhase,
        BatchCompatibilityField, BatchDatabaseError, CatalogInput, CatalogSelection,
        IndexBatchRequest, ManifestIntent, NotifyAttemptPreparation, NotifyHandoffStatus,
        BATCH_DATABASE_FILE_NAME, BATCH_SCHEMA_VERSION,
    };
    use crate::transforms::CATALOG_CSV_V3_COLUMNS;

    fn catalog_entry(catalog_id: &str) -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: catalog_id.to_string(),
            catalog_aliases: Vec::new(),
            title: format!("Journal {catalog_id}"),
            issn: None,
            eissn: None,
            all_issns: Vec::new(),
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
    }

    fn input(file_name: &str, provider_name: &str, digest_byte: u8) -> CatalogInput {
        let catalog_name = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("fixture should have a catalog stem")
            .to_string();
        CatalogInput {
            path: Path::new("meta").join(file_name),
            file_name: file_name.to_string(),
            catalog_name: catalog_name.clone(),
            csv_sha256: format!("{digest_byte:02x}").repeat(32),
            provider_name: provider_name.to_string(),
            entries: vec![catalog_entry(&format!("{catalog_name}-journal"))],
        }
    }

    fn request(inputs: Vec<CatalogInput>) -> IndexBatchRequest {
        IndexBatchRequest::new(
            inputs,
            CatalogSelection::All,
            IndexSyncMode::Incremental,
            20,
            true,
            false,
        )
        .expect("batch request should build")
    }

    fn ready(admission: BatchAdmission) -> super::IndexBatch {
        match admission {
            BatchAdmission::Ready(batch) => batch,
            BatchAdmission::Abandoning(_) => panic!("batch should be ready"),
        }
    }

    fn outcome(run_id: &str) -> BatchCatalogOutcome {
        BatchCatalogOutcome {
            run_id: run_id.to_string(),
            journal_count: 1,
            written_article_count: 2,
            source_attempt_count: 3,
            manifest_path: None,
        }
    }

    #[test]
    fn frozen_catalog_hash_and_entries_share_one_exact_read() {
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("english.csv");
        let text = format!(
            "{}\nenglish-journal,,English Journal,1234-5679,,1234-5679,,,,,,,,,,\n",
            CATALOG_CSV_V3_COLUMNS.join(",")
        );
        std::fs::write(&path, &text).expect("catalog should write");

        let frozen = CatalogInput::freeze(&path, "scholarly")
            .expect("catalog should freeze from one byte snapshot");

        assert_eq!(frozen.file_name, "english.csv");
        assert_eq!(frozen.catalog_name, "english");
        assert_eq!(frozen.entries.len(), 1);
        assert_eq!(frozen.csv_sha256.len(), 64);
        assert!(!format!("{frozen:?}").contains(directory.path().to_string_lossy().as_ref()));
        assert!(!format!("{frozen:?}").contains(&frozen.csv_sha256));
    }

    #[test]
    fn schema_initializes_once_and_rejects_newer_versions() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        init_batch_db(&connection).expect("current schema should reopen");
        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("schema version should read");
        assert_eq!(version, BATCH_SCHEMA_VERSION);

        let newer = Connection::open_in_memory().expect("newer database should open");
        newer
            .pragma_update(None, "user_version", BATCH_SCHEMA_VERSION + 1)
            .expect("newer schema version should set");
        assert!(matches!(
            init_batch_db(&newer),
            Err(BatchDatabaseError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn notification_attempt_identifier_matches_the_child_protocol_contract() {
        let first = new_notify_attempt_id();
        let second = new_notify_attempt_id();

        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn v1_notifying_handoff_migrates_to_a_conservative_unknown_state() {
        let connection = Connection::open_in_memory().expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE index_batch_catalogs (
                     batch_id TEXT NOT NULL,
                     ordinal INTEGER NOT NULL,
                     phase TEXT NOT NULL,
                     notify_exit_code INTEGER
                 );
                 INSERT INTO index_batch_catalogs
                     (batch_id, ordinal, phase, notify_exit_code)
                 VALUES
                     ('active-batch', 0, 'notifying', 7),
                     ('completed-batch', 0, 'completed', 0);
                 PRAGMA user_version = 1;",
            )
            .expect("v1 handoff fixture should initialize");

        init_batch_db(&connection).expect("v1 batch schema should migrate");

        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("schema version should read");
        let active = connection
            .query_row(
                "SELECT notify_attempt_id, notify_status, notify_exit_code,
                        notify_unknown_acknowledged_attempt_id,
                        notify_unknown_acknowledged_at
                 FROM index_batch_catalogs
                 WHERE batch_id = 'active-batch'",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i32>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .expect("migrated active handoff should read");
        let completed_status = connection
            .query_row(
                "SELECT notify_status FROM index_batch_catalogs
                 WHERE batch_id = 'completed-batch'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .expect("migrated completed handoff should read");

        assert_eq!(version, 2);
        assert!(active
            .0
            .is_some_and(|attempt| attempt.starts_with("legacy-notify-")));
        assert_eq!(active.1.as_deref(), Some("unknown"));
        assert_eq!(active.2, Some(7));
        assert_eq!(active.3, None);
        assert_eq!(active.4, None);
        assert_eq!(completed_status.as_deref(), Some("completed"));
    }

    #[test]
    fn notify_attempt_state_reuses_running_retries_safe_failures_and_fences_unknown() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        let request = request(vec![input("english.csv", "scholarly", 1)]);
        let owner = "notify-owner";
        let batch = ready(
            admit_batch(&connection, &request, true, owner, 100).expect("batch should create"),
        );
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            owner,
            0,
            BatchCatalogPhase::Indexing,
            101,
        )
        .expect("catalog should enter indexing");
        let outcome = outcome("catalog-run");
        store_catalog_outcome(&connection, &batch.batch_id, owner, 0, &outcome, 102)
            .expect("outcome should persist");
        let intent = ManifestIntent::new(
            b"{}\n".to_vec(),
            None,
            "data/push_state/english.changes.json",
            "catalog-run",
            "102",
        )
        .expect("manifest intent should build");
        store_manifest_intent(&connection, &batch.batch_id, owner, 0, &intent, 103)
            .expect("manifest should prepare");
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            owner,
            0,
            BatchCatalogPhase::ManifestPublished,
            104,
        )
        .expect("manifest should publish");
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            owner,
            0,
            BatchCatalogPhase::Notifying,
            105,
        )
        .expect("catalog should enter notifying");

        let first = prepare_notify_attempt(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-one",
            false,
            106,
        )
        .expect("first attempt should prepare");
        assert!(matches!(
            first,
            NotifyAttemptPreparation::Run(ref state)
                if state.attempt_id == "attempt-one"
                    && state.status == NotifyHandoffStatus::Running
        ));
        let reused = prepare_notify_attempt(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "unused-attempt",
            false,
            107,
        )
        .expect("running attempt should reuse");
        assert!(matches!(
            reused,
            NotifyAttemptPreparation::Run(ref state) if state.attempt_id == "attempt-one"
        ));
        record_notify_attempt_result(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-one",
            NotifyHandoffStatus::Failed,
            Some(1),
            108,
        )
        .expect("known failure should persist");
        assert!(matches!(
            complete_catalog(&connection, &batch.batch_id, owner, 0, &outcome, 108),
            Err(BatchDatabaseError::InvalidState { reason })
                if reason == "catalog cannot complete without a trusted notification result"
        ));

        let second = prepare_notify_attempt(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-two",
            false,
            109,
        )
        .expect("known failure should create a new attempt");
        assert!(matches!(
            second,
            NotifyAttemptPreparation::Run(ref state) if state.attempt_id == "attempt-two"
        ));
        record_notify_attempt_result(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-two",
            NotifyHandoffStatus::Cancelled,
            Some(1),
            110,
        )
        .expect("cancelled result should persist");
        let third = prepare_notify_attempt(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-three",
            false,
            111,
        )
        .expect("cancelled result should create a new attempt");
        assert!(matches!(
            third,
            NotifyAttemptPreparation::Run(ref state) if state.attempt_id == "attempt-three"
        ));
        record_notify_attempt_result(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-three",
            NotifyHandoffStatus::TimedOut,
            Some(1),
            112,
        )
        .expect("timed-out result should persist");
        let fourth = prepare_notify_attempt(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-four",
            false,
            113,
        )
        .expect("timed-out result should create a new attempt");
        assert!(matches!(
            fourth,
            NotifyAttemptPreparation::Run(ref state) if state.attempt_id == "attempt-four"
        ));
        record_notify_attempt_result(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-four",
            NotifyHandoffStatus::Unknown,
            Some(1),
            114,
        )
        .expect("unknown result should persist");
        assert!(matches!(
            prepare_notify_attempt(
                &connection,
                &batch.batch_id,
                owner,
                0,
                "blocked-attempt",
                false,
                115,
            )
            .expect("unknown attempt should return a policy outcome"),
            NotifyAttemptPreparation::BlockedUnknown(ref state)
                if state.attempt_id == "attempt-four"
        ));

        let acknowledged = prepare_notify_attempt(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-five",
            true,
            116,
        )
        .expect("explicit acknowledgement should create a new attempt");
        assert!(matches!(
            acknowledged,
            NotifyAttemptPreparation::Run(ref state)
                if state.attempt_id == "attempt-five"
                    && state.unknown_acknowledged_attempt_id.as_deref() == Some("attempt-four")
                    && state.unknown_acknowledged_at == Some(116)
        ));
        record_notify_attempt_result(
            &connection,
            &batch.batch_id,
            owner,
            0,
            "attempt-five",
            NotifyHandoffStatus::Completed,
            Some(0),
            117,
        )
        .expect("successful result should persist");
        assert!(matches!(
            prepare_notify_attempt(
                &connection,
                &batch.batch_id,
                owner,
                0,
                "unused-success-attempt",
                false,
                118,
            )
            .expect("successful handoff should return a policy outcome"),
            NotifyAttemptPreparation::Succeeded(ref state)
                if state.attempt_id == "attempt-five"
        ));
        complete_catalog(&connection, &batch.batch_id, owner, 0, &outcome, 119)
            .expect("successful handoff should allow catalog completion");
        let stored = read_batch_catalogs(&connection, &batch.batch_id)
            .expect("catalog should read")
            .remove(0);
        assert_eq!(stored.phase, BatchCatalogPhase::Completed);
        assert_eq!(
            stored
                .notify_handoff
                .expect("handoff should persist")
                .unknown_acknowledged_attempt_id
                .as_deref(),
            Some("attempt-four")
        );
    }

    #[test]
    fn one_active_batch_and_one_unexpired_owner_win_contention() {
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join(BATCH_DATABASE_FILE_NAME);
        let first = open_batch_db(&path).expect("first connection should open");
        let second = open_batch_db(&path).expect("second connection should open");
        let request = request(vec![input("english.csv", "scholarly", 1)]);
        let first_owner = new_batch_owner_id();
        let second_owner = new_batch_owner_id();
        let first_batch = ready(
            admit_batch(&first, &request, true, &first_owner, 100)
                .expect("first owner should acquire the batch"),
        );
        heartbeat_batch_lease(&first, &first_batch.batch_id, &first_owner, 101)
            .expect("first owner should renew the batch lease");

        let error = admit_batch(&second, &request, true, &second_owner, 101)
            .expect_err("second owner should lose lease contention");

        assert!(matches!(error, BatchDatabaseError::ActiveLease { .. }));
        let active_count = first
            .query_row(
                "SELECT COUNT(*) FROM index_batches WHERE status IN ('active', 'abandoning')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("active count should read");
        assert_eq!(active_count, 1);
        assert_eq!(first_batch.catalogs.len(), 1);

        let recovered = ready(
            admit_batch(&second, &request, true, &second_owner, 401)
                .expect("expired lease should permit one new owner"),
        );
        assert_eq!(recovered.batch_id, first_batch.batch_id);
        assert!(recovered.did_resume);
    }

    #[test]
    fn compatibility_is_exact_for_correctness_fields_only() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        let original = request(vec![
            input("ccf.csv", "scholarly", 1),
            input("english.csv", "scholarly", 2),
        ]);
        let owner = new_batch_owner_id();
        let batch = ready(
            admit_batch(&connection, &original, true, &owner, 100).expect("batch should create"),
        );
        release_batch_lease(&connection, &batch.batch_id, &owner).expect("lease should release");
        let resumed_owner = new_batch_owner_id();
        let resumed = ready(
            admit_batch(&connection, &original, true, &resumed_owner, 101)
                .expect("identical correctness inputs should resume"),
        );
        assert!(resumed.did_resume);
        release_batch_lease(&connection, &resumed.batch_id, &resumed_owner)
            .expect("resumed lease should release");

        let mut changed_content = original.clone();
        changed_content.catalogs[1].csv_sha256 = "ff".repeat(32);
        let error = admit_batch(
            &connection,
            &changed_content,
            true,
            &new_batch_owner_id(),
            102,
        )
        .expect_err("changed CSV bytes should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::CatalogContent]
        ));

        let mut changed_route = original.clone();
        changed_route.catalogs[0].provider_name = "openalex".to_string();
        let error = admit_batch(
            &connection,
            &changed_route,
            true,
            &new_batch_owner_id(),
            103,
        )
        .expect_err("changed Provider route should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::ProviderRoute]
        ));
        assert!(!format!("{error}").contains(&original.catalogs[0].csv_sha256));

        let mut changed_order = original.clone();
        changed_order.catalogs.swap(0, 1);
        let error = admit_batch(
            &connection,
            &changed_order,
            true,
            &new_batch_owner_id(),
            104,
        )
        .expect_err("changed CSV order should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::CatalogOrder]
        ));

        let mut changed_selection = original.clone();
        changed_selection.selection = CatalogSelection::ExplicitFile;
        let error = admit_batch(
            &connection,
            &changed_selection,
            true,
            &new_batch_owner_id(),
            105,
        )
        .expect_err("changed selection mechanism should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::CatalogSelection]
        ));

        let mut changed_mode = original.clone();
        changed_mode.mode = IndexSyncMode::FullRescan;
        let error = admit_batch(&connection, &changed_mode, true, &new_batch_owner_id(), 106)
            .expect_err("changed sync mode should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::SyncMode]
        ));

        let mut changed_issue_batch = original.clone();
        changed_issue_batch.issue_batch_size += 1;
        let error = admit_batch(
            &connection,
            &changed_issue_batch,
            true,
            &new_batch_owner_id(),
            107,
        )
        .expect_err("changed issue batch should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::IssueBatchSize]
        ));

        let mut changed_notify = original.clone();
        changed_notify.notify = false;
        let error = admit_batch(
            &connection,
            &changed_notify,
            true,
            &new_batch_owner_id(),
            108,
        )
        .expect_err("changed notify mode should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::Notify]
        ));

        let mut changed_notify_dry_run = original.clone();
        changed_notify_dry_run.notify_dry_run = true;
        let error = admit_batch(
            &connection,
            &changed_notify_dry_run,
            true,
            &new_batch_owner_id(),
            109,
        )
        .expect_err("changed notify dry-run mode should reject resume");
        assert!(matches!(
            error,
            BatchDatabaseError::CompatibilityMismatch { ref fields }
                if fields == &[BatchCompatibilityField::NotifyDryRun]
        ));
    }

    #[test]
    fn completed_batch_is_not_reused_by_next_update() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        let request = request(vec![input("english.csv", "scholarly", 1)]);
        let owner = new_batch_owner_id();
        let batch = ready(
            admit_batch(&connection, &request, true, &owner, 100).expect("batch should create"),
        );
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            BatchCatalogPhase::Indexing,
            101,
        )
        .expect("indexing should begin");
        complete_catalog(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            &outcome("run-one"),
            102,
        )
        .expect("catalog should complete");
        complete_batch(&connection, &batch.batch_id, &owner, 103).expect("batch should complete");

        let next_owner = new_batch_owner_id();
        let next = ready(
            admit_batch(&connection, &request, true, &next_owner, 104)
                .expect("next update should create a fresh batch"),
        );
        assert_ne!(next.batch_id, batch.batch_id);
        assert!(!next.did_resume);
        assert_eq!(next.catalogs[0].phase, BatchCatalogPhase::Pending);
    }

    #[test]
    fn phase_and_manifest_intent_survive_reopen_without_secret_debug_values() {
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("index-batches.sqlite");
        let connection = open_batch_db(&path).expect("batch database should open");
        let request = request(vec![input("english.csv", "scholarly", 1)]);
        let owner = new_batch_owner_id();
        let batch = ready(
            admit_batch(&connection, &request, true, &owner, 100).expect("batch should create"),
        );
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            BatchCatalogPhase::Indexing,
            101,
        )
        .expect("indexing should begin");
        store_catalog_outcome(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            &outcome("run-one"),
            102,
        )
        .expect("catalog counters should persist");
        let payload = br#"{"run_id":"secret-payload-value"}\n"#.to_vec();
        let intent = ManifestIntent::new(
            payload.clone(),
            Some(9),
            "data/push_state/english.changes.json",
            "run-one",
            "100",
        )
        .expect("manifest intent should build");
        store_manifest_intent(&connection, &batch.batch_id, &owner, 0, &intent, 103)
            .expect("manifest intent should persist");
        release_batch_lease(&connection, &batch.batch_id, &owner).expect("lease should release");
        drop(connection);

        let reopened = open_batch_db(&path).expect("batch database should reopen");
        let resumed_owner = new_batch_owner_id();
        let resumed = ready(
            admit_batch(&reopened, &request, true, &resumed_owner, 104)
                .expect("batch should resume after reopen"),
        );
        let catalog = &resumed.catalogs[0];
        assert_eq!(catalog.phase, BatchCatalogPhase::ManifestPrepared);
        assert_eq!(catalog.manifest_intent.as_ref(), Some(&intent));
        let debug = format!("{:?}", catalog.manifest_intent.as_ref().expect("intent"));
        assert!(!debug.contains("secret-payload-value"));
        assert!(!debug.contains(&intent.sha256));
        assert!(!debug.contains("data/push_state"));
    }

    #[test]
    fn invalid_phase_transition_fails_without_mutation() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        let request = request(vec![input("english.csv", "scholarly", 1)]);
        let owner = new_batch_owner_id();
        let batch = ready(
            admit_batch(&connection, &request, true, &owner, 100).expect("batch should create"),
        );

        let error = transition_catalog_phase(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            BatchCatalogPhase::ManifestPublished,
            101,
        )
        .expect_err("pending catalog cannot publish a manifest");

        assert!(matches!(error, BatchDatabaseError::InvalidState { .. }));
        let catalogs =
            read_batch_catalogs(&connection, &batch.batch_id).expect("catalog state should read");
        assert_eq!(catalogs[0].phase, BatchCatalogPhase::Pending);
    }

    #[test]
    fn no_resume_preserves_abandoned_manifest_state_before_replacement() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        let original = request(vec![input("english.csv", "scholarly", 1)]);
        let owner = new_batch_owner_id();
        let batch = ready(
            admit_batch(&connection, &original, true, &owner, 100).expect("batch should create"),
        );
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            BatchCatalogPhase::Indexing,
            101,
        )
        .expect("indexing should begin");
        let intent = ManifestIntent::new(
            b"{}\n".to_vec(),
            None,
            "data/push_state/english.changes.json",
            "run-one",
            "100",
        )
        .expect("manifest intent should build");
        store_manifest_intent(&connection, &batch.batch_id, &owner, 0, &intent, 102)
            .expect("manifest intent should persist");
        release_batch_lease(&connection, &batch.batch_id, &owner).expect("lease should release");

        let replacement_request = request(vec![input("english.csv", "scholarly", 2)]);
        let replacement_owner = new_batch_owner_id();
        let abandoning = match admit_batch(
            &connection,
            &replacement_request,
            false,
            &replacement_owner,
            103,
        )
        .expect("no-resume should stage abandonment")
        {
            BatchAdmission::Abandoning(batch) => batch,
            BatchAdmission::Ready(_) => panic!("existing batch should require cleanup"),
        };
        let replacement = replace_abandoning_batch(
            &connection,
            &abandoning.batch_id,
            &replacement_request,
            &replacement_owner,
            104,
        )
        .expect("abandonment should finish after checkpoint cleanup");

        assert_ne!(replacement.batch_id, batch.batch_id);
        let old_status = connection
            .query_row(
                "SELECT status FROM index_batches WHERE batch_id = ?1",
                [&batch.batch_id],
                |row| row.get::<_, String>(0),
            )
            .expect("old status should read");
        let old_payload = connection
            .query_row(
                "SELECT manifest_payload FROM index_batch_catalogs
                 WHERE batch_id = ?1 AND ordinal = 0",
                [&batch.batch_id],
                |row| row.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()
            .expect("old payload should query")
            .flatten();
        assert_eq!(old_status, "abandoned");
        assert_eq!(old_payload, Some(b"{}\n".to_vec()));
        assert_eq!(replacement.catalogs[0].phase, BatchCatalogPhase::Pending);
    }

    #[test]
    fn no_resume_cannot_discard_a_published_notification_handoff() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_batch_db(&connection).expect("schema should initialize");
        let original = request(vec![input("english.csv", "scholarly", 1)]);
        let owner = new_batch_owner_id();
        let batch = ready(
            admit_batch(&connection, &original, true, &owner, 100).expect("batch should create"),
        );
        transition_catalog_phase(
            &connection,
            &batch.batch_id,
            &owner,
            0,
            BatchCatalogPhase::Indexing,
            101,
        )
        .expect("indexing should begin");
        let outcome = BatchCatalogOutcome {
            manifest_path: Some("data/push_state/english.changes.json".to_string()),
            ..outcome("run-one")
        };
        store_catalog_outcome(&connection, &batch.batch_id, &owner, 0, &outcome, 102)
            .expect("published outcome intent should persist");
        let intent = ManifestIntent::new(
            b"{}\n".to_vec(),
            None,
            "data/push_state/english.changes.json",
            "run-one",
            "100",
        )
        .expect("manifest intent should build");
        store_manifest_intent(&connection, &batch.batch_id, &owner, 0, &intent, 103)
            .expect("manifest intent should persist");
        release_batch_lease(&connection, &batch.batch_id, &owner).expect("lease should release");

        let replacement_request = request(vec![input("english.csv", "scholarly", 2)]);
        let replacement_owner = new_batch_owner_id();
        let error = admit_batch(
            &connection,
            &replacement_request,
            false,
            &replacement_owner,
            104,
        )
        .expect_err("published notification handoff must not be abandoned");

        assert!(matches!(
            error,
            BatchDatabaseError::PublishedNotificationPending
        ));
        let status = connection
            .query_row(
                "SELECT status FROM index_batches WHERE batch_id = ?1",
                [&batch.batch_id],
                |row| row.get::<_, String>(0),
            )
            .expect("active batch status should read");
        let lease_count = connection
            .query_row("SELECT COUNT(*) FROM index_batch_lease", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("rolled-back lease count should read");
        assert_eq!(status, "active");
        assert_eq!(lease_count, 0);
    }
}
