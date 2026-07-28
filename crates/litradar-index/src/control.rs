//! Disposable provider-scoped indexing control storage.

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use litradar_domain::{IndexSyncMode, ProviderProgress};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Deserialize;

use crate::schema::ContentDatabaseError;

/// Current disposable control database schema version.
pub const CONTROL_SCHEMA_VERSION: i64 = 3;

const CONTROL_BUSY_TIMEOUT_SECONDS: u64 = 30;
const LEASE_DURATION_SECONDS: i64 = 300;
const MAX_OPAQUE_STATE_BYTES: usize = 65_536;

/// One successfully committed Provider boundary for a canonical journal.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderSyncAnchor {
    /// Opaque Provider boundary, or `None` when successful coverage has no reusable boundary.
    pub committed_anchor: Option<String>,
    /// Safe orchestration timestamp for the completed synchronization.
    pub completed_at: String,
}

impl fmt::Debug for ProviderSyncAnchor {
    /// Format successful state without exposing the opaque Provider anchor.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSyncAnchor")
            .field("has_committed_anchor", &self.committed_anchor.is_some())
            .field("completed_at", &self.completed_at)
            .finish()
    }
}

/// Frozen Provider traversal state for one in-flight canonical journal synchronization.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderRunCheckpoint {
    /// Current core run that owns this journal traversal.
    pub run_id: String,
    /// Synchronization mode frozen for this traversal.
    pub mode: IndexSyncMode,
    /// Opaque successful boundary copied when the traversal began.
    pub base_anchor: Option<String>,
    /// Opaque Provider position for the next fetch operation.
    pub traversal_checkpoint: Option<String>,
    /// Safe timestamp retained from the first traversal attempt.
    pub started_at: String,
    /// Safe timestamp for the latest ownership or progress update.
    pub updated_at: String,
}

impl fmt::Debug for ProviderRunCheckpoint {
    /// Format run metadata without exposing opaque Provider state.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRunCheckpoint")
            .field("run_id", &self.run_id)
            .field("mode", &self.mode)
            .field("has_base_anchor", &self.base_anchor.is_some())
            .field(
                "has_traversal_checkpoint",
                &self.traversal_checkpoint.is_some(),
            )
            .field("started_at", &self.started_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Core decision returned before any Provider request for one journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalSyncPreparation {
    /// A normal resumable run may skip an already successful journal.
    Skip,
    /// The Provider must execute or resume this frozen traversal.
    Run(ProviderRunCheckpoint),
}

/// Disposable control database operation failure.
#[derive(Debug)]
pub enum ControlDatabaseError {
    /// Filesystem setup failed.
    Io(std::io::Error),
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// A non-disposable newer control schema was opened by an older binary.
    UnsupportedVersion {
        /// Version stored by the control database.
        found: i64,
        /// Highest version supported by this binary.
        supported: i64,
    },
    /// Another run owns an unexpired provider-scoped lease.
    ActiveLease {
        /// Current owner run identifier.
        run_id: String,
        /// Lease expiry as Unix seconds.
        expires_at: i64,
    },
    /// The requested run no longer owns the provider-scoped lease.
    OwnershipLost {
        /// Run identifier that failed the ownership check.
        run_id: String,
    },
    /// Stored and requested journal synchronization modes differ.
    RunModeMismatch {
        /// Mode retained by the in-flight journal traversal.
        stored: IndexSyncMode,
        /// Mode requested by the current command.
        requested: IndexSyncMode,
    },
    /// A journal run no longer matches the expected owner or frozen base anchor.
    RunOwnershipLost {
        /// Run identifier that failed the journal-state ownership check.
        run_id: String,
    },
    /// Disposable synchronization state violated a bounded invariant.
    InvalidSyncState {
        /// Fixed safe reason that does not include opaque state.
        reason: &'static str,
    },
}

impl fmt::Display for ControlDatabaseError {
    /// Format a safe control database diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported index control schema version {found}; maximum supported is {supported}"
            ),
            Self::ActiveLease { run_id, expires_at } => write!(
                formatter,
                "index control scope is owned by active run {run_id} until {expires_at}"
            ),
            Self::OwnershipLost { run_id } => {
                write!(
                    formatter,
                    "index run {run_id} no longer owns its control lease"
                )
            }
            Self::RunModeMismatch { stored, requested } => write!(
                formatter,
                "in-flight journal mode {stored:?} does not match requested mode {requested:?}; retry the original mode or disable resume"
            ),
            Self::RunOwnershipLost { run_id } => write!(
                formatter,
                "index run {run_id} no longer owns the frozen journal synchronization state"
            ),
            Self::InvalidSyncState { reason } => {
                write!(formatter, "invalid disposable synchronization state: {reason}")
            }
        }
    }
}

impl Error for ControlDatabaseError {
    /// Return the underlying SQLite failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::UnsupportedVersion { .. }
            | Self::ActiveLease { .. }
            | Self::OwnershipLost { .. }
            | Self::RunModeMismatch { .. }
            | Self::RunOwnershipLost { .. }
            | Self::InvalidSyncState { .. } => None,
        }
    }
}

impl From<std::io::Error> for ControlDatabaseError {
    /// Convert filesystem failures into control database errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for ControlDatabaseError {
    /// Convert SQLite failures into control database errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Open or recreate one disposable control database.
///
/// # Arguments
///
/// * `path` - Control database path outside the content index directory.
///
/// # Returns
///
/// Initialized control connection.
pub fn open_control_db(path: impl AsRef<Path>) -> Result<Connection, ControlDatabaseError> {
    if let Some(parent) = path
        .as_ref()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(CONTROL_BUSY_TIMEOUT_SECONDS))?;
    init_control_db(&connection)?;
    Ok(connection)
}

/// Failure while committing content before its disposable Provider progress.
#[derive(Debug)]
pub enum ContentCheckpointCommitError {
    /// The provider-neutral content transaction failed, so control progress was not attempted.
    Content(ContentDatabaseError),
    /// Content committed, but the disposable control transaction failed.
    Control(ControlDatabaseError),
}

impl fmt::Display for ContentCheckpointCommitError {
    /// Format one ordered content/checkpoint commit failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(error) => write!(formatter, "content commit failed: {error}"),
            Self::Control(error) => write!(formatter, "sync progress commit failed: {error}"),
        }
    }
}

impl Error for ContentCheckpointCommitError {
    /// Return the failed content or control operation.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Content(error) => Some(error),
            Self::Control(error) => Some(error),
        }
    }
}

impl From<ContentDatabaseError> for ContentCheckpointCommitError {
    /// Convert a content transaction failure into an ordered commit failure.
    fn from(error: ContentDatabaseError) -> Self {
        Self::Content(error)
    }
}

impl From<ControlDatabaseError> for ContentCheckpointCommitError {
    /// Convert a checkpoint transaction failure into an ordered commit failure.
    fn from(error: ControlDatabaseError) -> Self {
        Self::Control(error)
    }
}

/// Commit provider-neutral content before advancing one disposable journal synchronization.
///
/// # Arguments
///
/// * `control_connection` - Open provider-scoped control database.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `catalog_id` - Immutable LitRadar journal identifier.
/// * `run_id` - Expected core run owner.
/// * `mode` - Expected frozen synchronization mode.
/// * `base_anchor` - Expected frozen successful boundary.
/// * `progress` - Provider traversal or completion progress to commit after content.
/// * `updated_at` - Safe control-progress timestamp.
/// * `write_content` - One atomic provider-neutral content operation.
///
/// # Returns
///
/// The content operation outcome after both ordered commits succeed. A control failure leaves
/// committed content for idempotent replay and never advances control state first.
#[allow(clippy::too_many_arguments)]
pub fn commit_content_then_progress<Outcome, WriteContent>(
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
    let outcome = write_content()?;
    match progress {
        ProviderProgress::Continue { checkpoint } => advance_run_checkpoint(
            control_connection,
            catalog_name,
            provider_name,
            catalog_id,
            run_id,
            mode,
            base_anchor,
            checkpoint,
            updated_at,
        )?,
        ProviderProgress::Complete { next_anchor } => complete_sync_run(
            control_connection,
            catalog_name,
            provider_name,
            catalog_id,
            run_id,
            mode,
            base_anchor,
            next_anchor.as_deref(),
            updated_at,
        )?,
    }
    Ok(outcome)
}

/// Initialize one empty or current disposable control database.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
///
/// # Returns
///
/// Success after schema validation or initialization.
pub fn init_control_db(connection: &Connection) -> Result<(), ControlDatabaseError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if version > CONTROL_SCHEMA_VERSION {
        return Err(ControlDatabaseError::UnsupportedVersion {
            found: version,
            supported: CONTROL_SCHEMA_VERSION,
        });
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    if version == CONTROL_SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_leases (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             run_id TEXT NOT NULL,
             heartbeat_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             PRIMARY KEY (catalog_name, provider_name)
         );",
    )?;
    let has_legacy_checkpoints = table_exists(&transaction, "provider_checkpoints")?;
    if version <= 1 {
        rewrite_legacy_provider_names(&transaction, has_legacy_checkpoints)?;
    }
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_sync_anchors (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             catalog_id TEXT NOT NULL,
             committed_anchor TEXT
                 CHECK (committed_anchor IS NULL OR (
                     length(CAST(committed_anchor AS BLOB)) BETWEEN 1 AND 65536
                 )),
             completed_at TEXT NOT NULL CHECK (length(completed_at) > 0),
             PRIMARY KEY (catalog_name, provider_name, catalog_id)
         );

         CREATE TABLE IF NOT EXISTS provider_run_checkpoints (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             catalog_id TEXT NOT NULL,
             run_id TEXT NOT NULL CHECK (length(run_id) > 0),
             sync_mode TEXT NOT NULL
                 CHECK (sync_mode IN ('bootstrap', 'incremental', 'full_rescan')),
             base_anchor TEXT
                 CHECK (base_anchor IS NULL OR (
                     length(CAST(base_anchor AS BLOB)) BETWEEN 1 AND 65536
                 )),
             traversal_checkpoint TEXT
                 CHECK (traversal_checkpoint IS NULL OR (
                     length(CAST(traversal_checkpoint AS BLOB)) BETWEEN 1 AND 65536
                 )),
             started_at TEXT NOT NULL CHECK (length(started_at) > 0),
             updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
             PRIMARY KEY (catalog_name, provider_name, catalog_id)
         );

         CREATE INDEX IF NOT EXISTS idx_provider_sync_anchors_catalog
             ON provider_sync_anchors(catalog_name, catalog_id);
         CREATE INDEX IF NOT EXISTS idx_provider_run_checkpoints_catalog
             ON provider_run_checkpoints(catalog_name, catalog_id);",
    )?;
    if has_legacy_checkpoints {
        migrate_legacy_completed_anchors(&transaction)?;
        transaction.execute_batch("DROP TABLE provider_checkpoints;")?;
    }
    transaction.pragma_update(None, "user_version", CONTROL_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Rewrite disposable control keys from retired provider runtime names.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
///
/// # Returns
///
/// Success after legacy provider keys are rewritten or discarded on conflict.
fn rewrite_legacy_provider_names(
    connection: &Connection,
    has_legacy_checkpoints: bool,
) -> Result<(), ControlDatabaseError> {
    for (legacy_name, current_name) in [("cnki", "cnki_oversea"), ("zjlib_cnki", "zjlib")] {
        if has_legacy_checkpoints {
            connection.execute(
                "UPDATE provider_checkpoints
                 SET provider_name = ?1
                 WHERE provider_name = ?2
                   AND NOT EXISTS (
                       SELECT 1
                       FROM provider_checkpoints AS existing
                       WHERE existing.catalog_name = provider_checkpoints.catalog_name
                         AND existing.provider_name = ?1
                         AND existing.scope_kind = provider_checkpoints.scope_kind
                         AND existing.scope_key = provider_checkpoints.scope_key
                   )",
                params![current_name, legacy_name],
            )?;
            connection.execute(
                "DELETE FROM provider_checkpoints WHERE provider_name = ?1",
                params![legacy_name],
            )?;
        }
        connection.execute(
            "UPDATE provider_leases
             SET provider_name = ?1
             WHERE provider_name = ?2
               AND NOT EXISTS (
                   SELECT 1
                   FROM provider_leases AS existing
                   WHERE existing.catalog_name = provider_leases.catalog_name
                     AND existing.provider_name = ?1
               )",
            params![current_name, legacy_name],
        )?;
        connection.execute(
            "DELETE FROM provider_leases WHERE provider_name = ?1",
            params![legacy_name],
        )?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum LegacyCompletionMarker {
    Complete,
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool, ControlDatabaseError> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        params![table_name],
        |row| row.get(0),
    )?)
}

fn migrate_legacy_completed_anchors(connection: &Connection) -> Result<(), ControlDatabaseError> {
    let rows = {
        let mut statement = connection.prepare(
            "SELECT catalog_name, provider_name, scope_key, checkpoint, updated_at
             FROM provider_checkpoints
             WHERE scope_kind = 'journal'",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (catalog_name, provider_name, catalog_id, checkpoint, completed_at) in rows {
        if catalog_name.is_empty()
            || provider_name.is_empty()
            || catalog_id.is_empty()
            || completed_at.is_empty()
            || !matches!(
                serde_json::from_str::<LegacyCompletionMarker>(&checkpoint),
                Ok(LegacyCompletionMarker::Complete)
            )
        {
            continue;
        }
        connection.execute(
            "INSERT INTO provider_sync_anchors (
                 catalog_name, provider_name, catalog_id, committed_anchor, completed_at
             ) VALUES (?1, ?2, ?3, NULL, ?4)
             ON CONFLICT(catalog_name, provider_name, catalog_id) DO UPDATE SET
                 committed_anchor = NULL,
                 completed_at = excluded.completed_at",
            params![catalog_name, provider_name, catalog_id, completed_at],
        )?;
    }
    Ok(())
}

/// Read one successfully committed Provider boundary.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `catalog_id` - Immutable LitRadar journal identifier.
///
/// # Returns
///
/// Successful journal state when previously committed.
pub fn read_sync_anchor(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    catalog_id: &str,
) -> Result<Option<ProviderSyncAnchor>, ControlDatabaseError> {
    read_sync_anchor_row(connection, catalog_name, provider_name, catalog_id)
}

fn read_sync_anchor_row(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    catalog_id: &str,
) -> Result<Option<ProviderSyncAnchor>, ControlDatabaseError> {
    let anchor = connection
        .query_row(
            "SELECT committed_anchor, completed_at
             FROM provider_sync_anchors
             WHERE catalog_name = ?1 AND provider_name = ?2 AND catalog_id = ?3",
            params![catalog_name, provider_name, catalog_id],
            |row| {
                Ok(ProviderSyncAnchor {
                    committed_anchor: row.get(0)?,
                    completed_at: row.get(1)?,
                })
            },
        )
        .optional()?;
    if let Some(anchor) = &anchor {
        validate_optional_opaque(anchor.committed_anchor.as_deref(), "committed anchor")?;
    }
    Ok(anchor)
}

/// Read one frozen in-flight Provider traversal.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `catalog_id` - Immutable LitRadar journal identifier.
///
/// # Returns
///
/// In-flight journal state when one exists.
pub fn read_run_checkpoint(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    catalog_id: &str,
) -> Result<Option<ProviderRunCheckpoint>, ControlDatabaseError> {
    let checkpoint = connection
        .query_row(
            "SELECT run_id, sync_mode, base_anchor, traversal_checkpoint, started_at, updated_at
             FROM provider_run_checkpoints
             WHERE catalog_name = ?1 AND provider_name = ?2 AND catalog_id = ?3",
            params![catalog_name, provider_name, catalog_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    checkpoint
        .map(
            |(run_id, mode, base_anchor, traversal_checkpoint, started_at, updated_at)| {
                validate_optional_opaque(base_anchor.as_deref(), "base anchor")?;
                validate_optional_opaque(traversal_checkpoint.as_deref(), "traversal checkpoint")?;
                Ok(ProviderRunCheckpoint {
                    run_id,
                    mode: parse_sync_mode(&mode)?,
                    base_anchor,
                    traversal_checkpoint,
                    started_at,
                    updated_at,
                })
            },
        )
        .transpose()
}

/// Prepare, resume, replace, or skip one canonical journal synchronization.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `catalog_id` - Immutable LitRadar journal identifier.
/// * `run_id` - Current core run that will own the traversal.
/// * `mode` - Synchronization mode requested by the current command.
/// * `should_resume` - Whether matching in-flight state may be resumed.
/// * `updated_at` - Safe orchestration timestamp.
///
/// # Returns
///
/// A skip decision or the exact frozen state to pass to the Provider.
#[allow(clippy::too_many_arguments)]
pub fn prepare_journal_sync(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    catalog_id: &str,
    run_id: &str,
    mode: IndexSyncMode,
    should_resume: bool,
    updated_at: &str,
) -> Result<JournalSyncPreparation, ControlDatabaseError> {
    validate_run_metadata(run_id, updated_at)?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let anchor = read_sync_anchor_row(&transaction, catalog_name, provider_name, catalog_id)?;
    if mode == IndexSyncMode::Bootstrap && should_resume && anchor.is_some() {
        transaction.commit()?;
        return Ok(JournalSyncPreparation::Skip);
    }
    let desired_base_anchor = match mode {
        IndexSyncMode::Bootstrap => None,
        IndexSyncMode::Incremental | IndexSyncMode::FullRescan => anchor
            .as_ref()
            .and_then(|anchor| anchor.committed_anchor.clone()),
    };
    let existing = read_run_checkpoint(&transaction, catalog_name, provider_name, catalog_id)?;
    let prepared = if should_resume {
        if let Some(mut existing) = existing {
            if existing.mode != mode {
                return Err(ControlDatabaseError::RunModeMismatch {
                    stored: existing.mode,
                    requested: mode,
                });
            }
            if existing.base_anchor != desired_base_anchor {
                return Err(ControlDatabaseError::InvalidSyncState {
                    reason: "run base anchor does not match the committed anchor",
                });
            }
            let changed = transaction.execute(
                "UPDATE provider_run_checkpoints
                 SET run_id = ?4, updated_at = ?8
                 WHERE catalog_name = ?1 AND provider_name = ?2 AND catalog_id = ?3
                   AND run_id = ?5 AND sync_mode = ?6
                   AND base_anchor IS ?7",
                params![
                    catalog_name,
                    provider_name,
                    catalog_id,
                    run_id,
                    existing.run_id,
                    sync_mode_text(mode),
                    desired_base_anchor.as_deref(),
                    updated_at,
                ],
            )?;
            if changed != 1 {
                return Err(ControlDatabaseError::RunOwnershipLost {
                    run_id: run_id.to_string(),
                });
            }
            existing.run_id = run_id.to_string();
            existing.updated_at = updated_at.to_string();
            existing
        } else {
            insert_run_checkpoint(
                &transaction,
                catalog_name,
                provider_name,
                catalog_id,
                run_id,
                mode,
                desired_base_anchor.as_deref(),
                updated_at,
            )?;
            ProviderRunCheckpoint {
                run_id: run_id.to_string(),
                mode,
                base_anchor: desired_base_anchor,
                traversal_checkpoint: None,
                started_at: updated_at.to_string(),
                updated_at: updated_at.to_string(),
            }
        }
    } else {
        transaction.execute(
            "INSERT INTO provider_run_checkpoints (
                 catalog_name, provider_name, catalog_id, run_id, sync_mode,
                 base_anchor, traversal_checkpoint, started_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7)
             ON CONFLICT(catalog_name, provider_name, catalog_id) DO UPDATE SET
                 run_id = excluded.run_id,
                 sync_mode = excluded.sync_mode,
                 base_anchor = excluded.base_anchor,
                 traversal_checkpoint = NULL,
                 started_at = excluded.started_at,
                 updated_at = excluded.updated_at",
            params![
                catalog_name,
                provider_name,
                catalog_id,
                run_id,
                sync_mode_text(mode),
                desired_base_anchor.as_deref(),
                updated_at,
            ],
        )?;
        ProviderRunCheckpoint {
            run_id: run_id.to_string(),
            mode,
            base_anchor: desired_base_anchor,
            traversal_checkpoint: None,
            started_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
        }
    };
    transaction.commit()?;
    Ok(JournalSyncPreparation::Run(prepared))
}

#[allow(clippy::too_many_arguments)]
fn insert_run_checkpoint(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    catalog_id: &str,
    run_id: &str,
    mode: IndexSyncMode,
    base_anchor: Option<&str>,
    updated_at: &str,
) -> Result<(), ControlDatabaseError> {
    connection.execute(
        "INSERT INTO provider_run_checkpoints (
             catalog_name, provider_name, catalog_id, run_id, sync_mode,
             base_anchor, traversal_checkpoint, started_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7)",
        params![
            catalog_name,
            provider_name,
            catalog_id,
            run_id,
            sync_mode_text(mode),
            base_anchor,
            updated_at,
        ],
    )?;
    Ok(())
}

/// Check whether retired catalog aliases own any successful or in-flight Provider state.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `catalog_aliases` - Retired catalog identifiers claimed by canonical entries.
///
/// # Returns
///
/// Whether any Provider namespace retains synchronization state for an alias.
pub fn has_catalog_alias_sync_state(
    connection: &Connection,
    catalog_name: &str,
    catalog_aliases: &[String],
) -> Result<bool, ControlDatabaseError> {
    for alias in catalog_aliases {
        let has_state = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM provider_sync_anchors
                 WHERE catalog_name = ?1 AND catalog_id = ?2
                 UNION ALL
                 SELECT 1 FROM provider_run_checkpoints
                 WHERE catalog_name = ?1 AND catalog_id = ?2
             )",
            params![catalog_name, alias],
            |row| row.get::<_, bool>(0),
        )?;
        if has_state {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Advance only the traversal checkpoint for one owned frozen journal run.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `catalog_id` - Immutable LitRadar journal identifier.
/// * `run_id` - Expected core run owner.
/// * `mode` - Expected frozen synchronization mode.
/// * `base_anchor` - Expected frozen successful boundary.
/// * `checkpoint` - Opaque Provider traversal position.
/// * `updated_at` - Safe orchestration timestamp.
///
/// # Returns
///
/// Success after a fenced immediate control transaction commits without changing the anchor.
#[allow(clippy::too_many_arguments)]
pub fn advance_run_checkpoint(
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
    validate_run_metadata(run_id, updated_at)?;
    validate_optional_opaque(base_anchor, "base anchor")?;
    validate_optional_opaque(Some(checkpoint), "traversal checkpoint")?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE provider_run_checkpoints
         SET traversal_checkpoint = ?7, updated_at = ?8
         WHERE catalog_name = ?1 AND provider_name = ?2 AND catalog_id = ?3
           AND run_id = ?4 AND sync_mode = ?5 AND base_anchor IS ?6",
        params![
            catalog_name,
            provider_name,
            catalog_id,
            run_id,
            sync_mode_text(mode),
            base_anchor,
            checkpoint,
            updated_at
        ],
    )?;
    if changed != 1 {
        return Err(ControlDatabaseError::RunOwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    transaction.commit()?;
    Ok(())
}

/// Atomically replace one owned journal run with its newly committed successful anchor.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `catalog_id` - Immutable LitRadar journal identifier.
/// * `run_id` - Expected core run owner.
/// * `mode` - Expected frozen synchronization mode.
/// * `base_anchor` - Expected frozen successful boundary.
/// * `next_anchor` - Optional opaque boundary proven by complete Provider coverage.
/// * `completed_at` - Safe orchestration timestamp.
///
/// # Returns
///
/// Success after one immediate transaction advances the anchor and removes the run.
#[allow(clippy::too_many_arguments)]
pub fn complete_sync_run(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    catalog_id: &str,
    run_id: &str,
    mode: IndexSyncMode,
    base_anchor: Option<&str>,
    next_anchor: Option<&str>,
    completed_at: &str,
) -> Result<(), ControlDatabaseError> {
    validate_run_metadata(run_id, completed_at)?;
    validate_optional_opaque(base_anchor, "base anchor")?;
    validate_optional_opaque(next_anchor, "committed anchor")?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let owns_run = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM provider_run_checkpoints
             WHERE catalog_name = ?1 AND provider_name = ?2 AND catalog_id = ?3
               AND run_id = ?4 AND sync_mode = ?5 AND base_anchor IS ?6
         )",
        params![
            catalog_name,
            provider_name,
            catalog_id,
            run_id,
            sync_mode_text(mode),
            base_anchor,
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !owns_run {
        return Err(ControlDatabaseError::RunOwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    transaction.execute(
        "INSERT INTO provider_sync_anchors (
             catalog_name, provider_name, catalog_id, committed_anchor, completed_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(catalog_name, provider_name, catalog_id) DO UPDATE SET
             committed_anchor = excluded.committed_anchor,
             completed_at = excluded.completed_at",
        params![
            catalog_name,
            provider_name,
            catalog_id,
            next_anchor,
            completed_at,
        ],
    )?;
    let deleted = transaction.execute(
        "DELETE FROM provider_run_checkpoints
         WHERE catalog_name = ?1 AND provider_name = ?2 AND catalog_id = ?3
           AND run_id = ?4 AND sync_mode = ?5 AND base_anchor IS ?6",
        params![
            catalog_name,
            provider_name,
            catalog_id,
            run_id,
            sync_mode_text(mode),
            base_anchor,
        ],
    )?;
    if deleted != 1 {
        return Err(ControlDatabaseError::RunOwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    transaction.commit()?;
    Ok(())
}

fn sync_mode_text(mode: IndexSyncMode) -> &'static str {
    match mode {
        IndexSyncMode::Bootstrap => "bootstrap",
        IndexSyncMode::Incremental => "incremental",
        IndexSyncMode::FullRescan => "full_rescan",
    }
}

fn parse_sync_mode(value: &str) -> Result<IndexSyncMode, ControlDatabaseError> {
    match value {
        "bootstrap" => Ok(IndexSyncMode::Bootstrap),
        "incremental" => Ok(IndexSyncMode::Incremental),
        "full_rescan" => Ok(IndexSyncMode::FullRescan),
        _ => Err(ControlDatabaseError::InvalidSyncState {
            reason: "stored synchronization mode is invalid",
        }),
    }
}

fn validate_optional_opaque(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), ControlDatabaseError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty() {
        return Err(ControlDatabaseError::InvalidSyncState { reason: field });
    }
    if value.len() > MAX_OPAQUE_STATE_BYTES {
        return Err(ControlDatabaseError::InvalidSyncState {
            reason: "opaque synchronization state exceeds 65,536 bytes",
        });
    }
    Ok(())
}

fn validate_run_metadata(run_id: &str, timestamp: &str) -> Result<(), ControlDatabaseError> {
    if run_id.is_empty() || timestamp.is_empty() {
        return Err(ControlDatabaseError::InvalidSyncState {
            reason: "run id and timestamp must not be empty",
        });
    }
    Ok(())
}

/// Acquire or reclaim one provider-scoped control lease.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `run_id` - Unique orchestration run identifier.
/// * `now_epoch_seconds` - Current Unix timestamp.
///
/// # Returns
///
/// Success when this run owns the lease.
pub fn acquire_lease(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), ControlDatabaseError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let existing = transaction
        .query_row(
            "SELECT run_id, expires_at
             FROM provider_leases
             WHERE catalog_name = ?1 AND provider_name = ?2",
            params![catalog_name, provider_name],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    if let Some((owner, expires_at)) = existing {
        if owner != run_id && expires_at > now_epoch_seconds {
            return Err(ControlDatabaseError::ActiveLease {
                run_id: owner,
                expires_at,
            });
        }
    }
    transaction.execute(
        "INSERT INTO provider_leases (
             catalog_name, provider_name, run_id, heartbeat_at, expires_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(catalog_name, provider_name) DO UPDATE SET
             run_id = excluded.run_id,
             heartbeat_at = excluded.heartbeat_at,
             expires_at = excluded.expires_at",
        params![
            catalog_name,
            provider_name,
            run_id,
            now_epoch_seconds,
            now_epoch_seconds + LEASE_DURATION_SECONDS
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Renew a provider-scoped control lease owned by one run.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `run_id` - Expected lease owner.
/// * `now_epoch_seconds` - Current Unix timestamp.
///
/// # Returns
///
/// Success when the exact owner renewed an unexpired lease.
pub fn heartbeat_lease(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), ControlDatabaseError> {
    let changed = connection.execute(
        "UPDATE provider_leases
         SET heartbeat_at = ?4, expires_at = ?5
         WHERE catalog_name = ?1 AND provider_name = ?2 AND run_id = ?3
           AND expires_at > ?4",
        params![
            catalog_name,
            provider_name,
            run_id,
            now_epoch_seconds,
            now_epoch_seconds + LEASE_DURATION_SECONDS
        ],
    )?;
    if changed == 0 {
        return Err(ControlDatabaseError::OwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    Ok(())
}

/// Release a provider-scoped lease owned by one run.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `run_id` - Expected lease owner.
///
/// # Returns
///
/// Success when the exact owner removed its lease.
pub fn release_lease(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
) -> Result<(), ControlDatabaseError> {
    let changed = connection.execute(
        "DELETE FROM provider_leases
         WHERE catalog_name = ?1 AND provider_name = ?2 AND run_id = ?3",
        params![catalog_name, provider_name, run_id],
    )?;
    if changed == 0 {
        return Err(ControlDatabaseError::OwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use litradar_domain::{
        ArticleDraft, IndexSyncMode, IssueDraft, JournalCatalogEntry, JournalDraft,
        JournalRankings, ProviderBatch, ProviderProgress,
    };
    use rusqlite::{params, Connection};
    use tempfile::tempdir;

    use crate::schema::{init_content_db, write_content_batch};

    use super::{
        acquire_lease, advance_run_checkpoint, commit_content_then_progress, complete_sync_run,
        has_catalog_alias_sync_state, heartbeat_lease, init_control_db, open_control_db,
        prepare_journal_sync, read_run_checkpoint, read_sync_anchor, release_lease,
        ContentCheckpointCommitError, ControlDatabaseError, JournalSyncPreparation,
        ProviderRunCheckpoint, CONTROL_SCHEMA_VERSION,
    };

    const CATALOG_NAME: &str = "chinese_journals";
    const PROVIDER_NAME: &str = "provider-a";
    const CATALOG_ID: &str = "issn-1234-5679";
    const TIMESTAMP: &str = "2026-07-18T00:00:00Z";

    #[test]
    fn schema_v3_initializes_only_expected_control_tables() {
        let connection = Connection::open_in_memory().expect("control database should open");
        init_control_db(&connection).expect("control schema should initialize");
        init_control_db(&connection).expect("current control schema should reopen");

        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("control version should read"),
            CONTROL_SCHEMA_VERSION
        );
        let tables = {
            let mut statement = connection
                .prepare(
                    "SELECT name FROM sqlite_master
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                     ORDER BY name",
                )
                .expect("table query should prepare");
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .expect("table query should run")
                .collect::<Result<Vec<_>, _>>()
                .expect("table names should read");
            rows
        };
        assert_eq!(
            tables,
            vec![
                "provider_leases".to_string(),
                "provider_run_checkpoints".to_string(),
                "provider_sync_anchors".to_string(),
            ]
        );
    }

    #[test]
    fn v2_migration_keeps_only_complete_journal_facts() {
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("v2.control.sqlite");
        let connection = create_legacy_control_database(&path, 2);
        insert_legacy_checkpoint(
            &connection,
            PROVIDER_NAME,
            "journal",
            CATALOG_ID,
            r#"{"state":"complete"}"#,
        );
        insert_legacy_checkpoint(
            &connection,
            "provider-cursor",
            "journal",
            "cursor-journal",
            r#"{"state":"provider","value":"page-2"}"#,
        );
        insert_legacy_checkpoint(
            &connection,
            "provider-listing",
            "listing",
            "",
            r#"{"state":"complete"}"#,
        );
        insert_legacy_checkpoint(
            &connection,
            "provider-year",
            "year",
            "year-journal:2025",
            r#"{"state":"complete"}"#,
        );
        insert_legacy_checkpoint(
            &connection,
            "cnki",
            "journal",
            "domestic-journal",
            r#"{"state":"complete"}"#,
        );
        drop(connection);

        let migrated = open_control_db(&path).expect("v2 database should migrate");
        let anchor = read_sync_anchor(&migrated, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
            .expect("migrated anchor should read")
            .expect("complete journal should migrate");
        assert_eq!(anchor.committed_anchor, None);
        assert_eq!(anchor.completed_at, TIMESTAMP);
        assert!(
            read_sync_anchor(&migrated, CATALOG_NAME, "provider-cursor", "cursor-journal")
                .expect("cursor state should read")
                .is_none()
        );
        assert!(
            read_sync_anchor(&migrated, CATALOG_NAME, "cnki", "domestic-journal")
                .expect("domestic state should read")
                .is_some()
        );
        assert!(
            read_sync_anchor(&migrated, CATALOG_NAME, "cnki_oversea", "domestic-journal")
                .expect("overseas state should read")
                .is_none()
        );
        assert_eq!(table_count(&migrated, "provider_run_checkpoints"), 0);
        assert!(!table_exists(&migrated, "provider_checkpoints"));
    }

    #[test]
    fn v0_and_v1_migrations_rewrite_retired_provider_names() {
        for version in [0, 1] {
            let directory = tempdir().expect("temporary directory should create");
            let path = directory.path().join(format!("v{version}.control.sqlite"));
            let connection = create_legacy_control_database(&path, version);
            insert_legacy_checkpoint(
                &connection,
                "cnki",
                "journal",
                CATALOG_ID,
                r#"{"state":"complete"}"#,
            );
            acquire_lease(&connection, CATALOG_NAME, "cnki", "legacy-run", 100)
                .expect("legacy lease should write");
            drop(connection);

            let migrated = open_control_db(&path).expect("legacy database should migrate");
            assert!(
                read_sync_anchor(&migrated, CATALOG_NAME, "cnki", CATALOG_ID)
                    .expect("retired state should read")
                    .is_none()
            );
            assert!(
                read_sync_anchor(&migrated, CATALOG_NAME, "cnki_oversea", CATALOG_ID)
                    .expect("rewritten state should read")
                    .is_some()
            );
            heartbeat_lease(&migrated, CATALOG_NAME, "cnki_oversea", "legacy-run", 101)
                .expect("rewritten lease should remain owned");
        }
    }

    #[test]
    fn migration_is_transactional_and_newer_versions_fail_closed() {
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("failed-v1.control.sqlite");
        let connection = create_legacy_control_database(&path, 1);
        insert_legacy_checkpoint(
            &connection,
            "cnki",
            "journal",
            CATALOG_ID,
            r#"{"state":"complete"}"#,
        );
        connection
            .execute_batch(
                "CREATE TRIGGER fail_provider_rewrite
                 BEFORE UPDATE OF provider_name ON provider_checkpoints
                 BEGIN SELECT RAISE(ABORT, 'forced provider rewrite failure'); END;",
            )
            .expect("migration failpoint should install");
        drop(connection);

        open_control_db(&path).expect_err("provider rewrite should fail");
        let unchanged = Connection::open(&path).expect("failed database should reopen");
        assert_eq!(
            unchanged
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("legacy version should read"),
            1
        );
        assert!(!table_exists(&unchanged, "provider_sync_anchors"));

        let newer = Connection::open_in_memory().expect("newer database should open");
        newer
            .pragma_update(None, "user_version", CONTROL_SCHEMA_VERSION + 1)
            .expect("newer version should write");
        assert!(matches!(
            init_control_db(&newer),
            Err(ControlDatabaseError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn matching_runs_resume_frozen_state_and_no_resume_replaces_only_the_run() {
        let connection = current_control();
        seed_anchor(&connection, PROVIDER_NAME, Some("anchor-a"));

        let first = prepared_run(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "run-a",
                IndexSyncMode::Incremental,
                true,
                TIMESTAMP,
            )
            .expect("incremental run should prepare"),
        );
        assert_eq!(first.base_anchor.as_deref(), Some("anchor-a"));
        advance_run_checkpoint(
            &connection,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            "run-a",
            IndexSyncMode::Incremental,
            Some("anchor-a"),
            "cursor-a",
            "2026-07-18T00:01:00Z",
        )
        .expect("traversal should advance");

        let resumed = prepared_run(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "run-b",
                IndexSyncMode::Incremental,
                true,
                "2026-07-18T00:02:00Z",
            )
            .expect("matching traversal should resume"),
        );
        assert_eq!(resumed.run_id, "run-b");
        assert_eq!(resumed.base_anchor.as_deref(), Some("anchor-a"));
        assert_eq!(resumed.traversal_checkpoint.as_deref(), Some("cursor-a"));
        assert_eq!(resumed.started_at, TIMESTAMP);

        assert!(matches!(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "normal-run",
                IndexSyncMode::Bootstrap,
                true,
                "2026-07-18T00:03:00Z",
            )
            .expect("completed normal journal should prepare"),
            JournalSyncPreparation::Skip
        ));

        let replacement = prepared_run(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "full-run",
                IndexSyncMode::FullRescan,
                false,
                "2026-07-18T00:04:00Z",
            )
            .expect("no-resume should replace traversal"),
        );
        assert_eq!(replacement.mode, IndexSyncMode::FullRescan);
        assert_eq!(replacement.base_anchor.as_deref(), Some("anchor-a"));
        assert_eq!(replacement.traversal_checkpoint, None);
        assert_eq!(
            read_sync_anchor(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("anchor should read")
                .expect("anchor should remain")
                .committed_anchor
                .as_deref(),
            Some("anchor-a")
        );
        assert!(matches!(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "incremental-run",
                IndexSyncMode::Incremental,
                true,
                "2026-07-18T00:05:00Z",
            ),
            Err(ControlDatabaseError::RunModeMismatch { .. })
        ));
    }

    #[test]
    fn continue_preserves_anchor_and_complete_atomically_replaces_the_run() {
        let connection = current_control();
        seed_anchor(&connection, PROVIDER_NAME, Some("anchor-old"));
        let run = prepared_run(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "run-current",
                IndexSyncMode::Incremental,
                true,
                TIMESTAMP,
            )
            .expect("incremental run should prepare"),
        );

        advance_run_checkpoint(
            &connection,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            "cursor-next",
            "2026-07-18T00:01:00Z",
        )
        .expect("continue should commit");
        assert_eq!(
            read_sync_anchor(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("anchor should read")
                .expect("anchor should exist")
                .committed_anchor
                .as_deref(),
            Some("anchor-old")
        );
        assert_eq!(
            read_run_checkpoint(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("run should read")
                .expect("run should exist")
                .traversal_checkpoint
                .as_deref(),
            Some("cursor-next")
        );

        connection
            .execute_batch(
                "CREATE TRIGGER fail_run_completion
                 BEFORE DELETE ON provider_run_checkpoints
                 BEGIN SELECT RAISE(ABORT, 'forced completion failure'); END;",
            )
            .expect("completion failpoint should install");
        assert!(complete_sync_run(
            &connection,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            Some("anchor-new"),
            "2026-07-18T00:02:00Z",
        )
        .is_err());
        assert_eq!(
            read_sync_anchor(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("old anchor should read")
                .expect("old anchor should remain")
                .committed_anchor
                .as_deref(),
            Some("anchor-old")
        );
        assert!(
            read_run_checkpoint(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("run should read")
                .is_some()
        );

        connection
            .execute_batch("DROP TRIGGER fail_run_completion")
            .expect("completion failpoint should drop");
        complete_sync_run(
            &connection,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            Some("anchor-new"),
            "2026-07-18T00:03:00Z",
        )
        .expect("completion should commit");
        assert_eq!(
            read_sync_anchor(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("new anchor should read")
                .expect("new anchor should exist")
                .committed_anchor
                .as_deref(),
            Some("anchor-new")
        );
        assert!(
            read_run_checkpoint(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("run should read")
                .is_none()
        );
    }

    #[test]
    fn content_precedes_control_and_both_failure_sides_are_replay_safe() {
        let content = Connection::open_in_memory().expect("content database should open");
        init_content_db(&content).expect("content schema should initialize");
        let control = current_control();
        let run = prepared_run(
            prepare_journal_sync(
                &control,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "ordered-run",
                IndexSyncMode::Bootstrap,
                false,
                TIMESTAMP,
            )
            .expect("run should prepare"),
        );
        let catalog = canonical_catalog();
        let batch = canonical_batch(ProviderProgress::Continue {
            checkpoint: "page-2".to_string(),
        });

        content
            .execute_batch(
                "CREATE TRIGGER fail_content_event
                 BEFORE INSERT ON article_change_events
                 BEGIN SELECT RAISE(ABORT, 'forced content failure'); END;",
            )
            .expect("content failpoint should install");
        let content_error = commit_content_then_progress(
            &control,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            &batch.progress,
            TIMESTAMP,
            || write_content_batch(&content, &catalog, &batch, "revision-a", TIMESTAMP),
        )
        .expect_err("content failure should stop control progress");
        assert!(matches!(
            content_error,
            ContentCheckpointCommitError::Content(_)
        ));
        assert_eq!(table_count(&content, "articles"), 0);
        assert_eq!(
            read_run_checkpoint(&control, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("run should read")
                .expect("run should remain")
                .traversal_checkpoint,
            None
        );
        content
            .execute_batch("DROP TRIGGER fail_content_event")
            .expect("content failpoint should drop");

        control
            .execute_batch(
                "CREATE TRIGGER fail_traversal_update
                 BEFORE UPDATE OF traversal_checkpoint ON provider_run_checkpoints
                 BEGIN SELECT RAISE(ABORT, 'forced control failure'); END;",
            )
            .expect("control failpoint should install");
        let control_error = commit_content_then_progress(
            &control,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            &batch.progress,
            TIMESTAMP,
            || write_content_batch(&content, &catalog, &batch, "revision-a", TIMESTAMP),
        )
        .expect_err("control failure should surface after content commits");
        assert!(matches!(
            control_error,
            ContentCheckpointCommitError::Control(_)
        ));
        assert_eq!(table_count(&content, "articles"), 1);
        assert_eq!(
            read_run_checkpoint(&control, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("run should read")
                .expect("run should remain")
                .traversal_checkpoint,
            None
        );
        control
            .execute_batch("DROP TRIGGER fail_traversal_update")
            .expect("control failpoint should drop");

        let replay = commit_content_then_progress(
            &control,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            &batch.progress,
            "2026-07-18T00:01:00Z",
            || write_content_batch(&content, &catalog, &batch, "revision-a", TIMESTAMP),
        )
        .expect("idempotent replay should advance traversal");
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(replay.change_events_emitted, 0);
        assert_eq!(
            read_run_checkpoint(&control, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
                .expect("run should read")
                .expect("run should remain")
                .traversal_checkpoint
                .as_deref(),
            Some("page-2")
        );
    }

    #[test]
    fn alias_detection_covers_anchor_and_run_state_across_providers() {
        let connection = current_control();
        let aliases = vec!["legacy-journal".to_string()];
        seed_anchor_for(
            &connection,
            "provider-anchor",
            "canonical-journal",
            Some("anchor"),
        );
        assert!(
            !has_catalog_alias_sync_state(&connection, "english_journals", &aliases)
                .expect("canonical state should not block alias")
        );

        let run = prepared_run(
            prepare_journal_sync(
                &connection,
                "english_journals",
                "provider-run",
                &aliases[0],
                "alias-run",
                IndexSyncMode::Bootstrap,
                false,
                TIMESTAMP,
            )
            .expect("alias run should prepare"),
        );
        assert!(
            has_catalog_alias_sync_state(&connection, "english_journals", &aliases)
                .expect("alias run should be detected")
        );
        complete_sync_run(
            &connection,
            "english_journals",
            "provider-run",
            &aliases[0],
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            None,
            TIMESTAMP,
        )
        .expect("alias run should complete");
        assert!(
            has_catalog_alias_sync_state(&connection, "english_journals", &aliases)
                .expect("alias anchor should be detected")
        );
    }

    #[test]
    fn provider_switch_and_control_loss_start_without_foreign_state() {
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("provider-scoped.control.sqlite");
        let connection = open_control_db(&path).expect("control database should open");
        seed_anchor(&connection, "provider-a", Some("anchor-a"));
        assert!(
            read_sync_anchor(&connection, CATALOG_NAME, "provider-b", CATALOG_ID)
                .expect("replacement provider state should read")
                .is_none()
        );
        drop(connection);
        std::fs::remove_file(&path).expect("disposable control database should delete");

        let recreated = open_control_db(&path).expect("control database should recreate");
        assert!(
            read_sync_anchor(&recreated, CATALOG_NAME, "provider-a", CATALOG_ID)
                .expect("recreated state should read")
                .is_none()
        );
        let run = prepared_run(
            prepare_journal_sync(
                &recreated,
                CATALOG_NAME,
                "provider-a",
                CATALOG_ID,
                "bootstrap-after-loss",
                IndexSyncMode::Bootstrap,
                true,
                TIMESTAMP,
            )
            .expect("control loss should bootstrap"),
        );
        assert_eq!(run.mode, IndexSyncMode::Bootstrap);
        assert_eq!(run.base_anchor, None);
    }

    #[test]
    fn opaque_state_and_run_ownership_fail_closed() {
        let connection = current_control();
        let run = prepared_run(
            prepare_journal_sync(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "owned-run",
                IndexSyncMode::Incremental,
                false,
                TIMESTAMP,
            )
            .expect("run should prepare"),
        );
        assert!(matches!(
            advance_run_checkpoint(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                &run.run_id,
                run.mode,
                run.base_anchor.as_deref(),
                "",
                TIMESTAMP,
            ),
            Err(ControlDatabaseError::InvalidSyncState { .. })
        ));
        let oversized = "x".repeat(65_537);
        assert!(matches!(
            advance_run_checkpoint(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                &run.run_id,
                run.mode,
                run.base_anchor.as_deref(),
                &oversized,
                TIMESTAMP,
            ),
            Err(ControlDatabaseError::InvalidSyncState { .. })
        ));
        assert!(matches!(
            complete_sync_run(
                &connection,
                CATALOG_NAME,
                PROVIDER_NAME,
                CATALOG_ID,
                "wrong-run",
                run.mode,
                run.base_anchor.as_deref(),
                Some("anchor"),
                TIMESTAMP,
            ),
            Err(ControlDatabaseError::RunOwnershipLost { .. })
        ));
        let checkpoint_secret = "checkpoint-secret-sentinel";
        advance_run_checkpoint(
            &connection,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            checkpoint_secret,
            TIMESTAMP,
        )
        .expect("valid checkpoint should advance");
        let stored_run = read_run_checkpoint(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
            .expect("stored run should read")
            .expect("stored run should exist");
        assert!(!format!("{stored_run:?}").contains(checkpoint_secret));

        let anchor_secret = "anchor-secret-sentinel";
        complete_sync_run(
            &connection,
            CATALOG_NAME,
            PROVIDER_NAME,
            CATALOG_ID,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            Some(anchor_secret),
            TIMESTAMP,
        )
        .expect("valid anchor should complete");
        let stored_anchor = read_sync_anchor(&connection, CATALOG_NAME, PROVIDER_NAME, CATALOG_ID)
            .expect("stored anchor should read")
            .expect("stored anchor should exist");
        assert!(!format!("{stored_anchor:?}").contains(anchor_secret));
    }

    #[test]
    fn leases_are_fenced_by_provider_scope_owner_and_expiry() {
        let connection = current_control();
        acquire_lease(&connection, "catalog", "provider", "run-a", 100)
            .expect("first owner should acquire");
        assert!(matches!(
            acquire_lease(&connection, "catalog", "provider", "run-b", 101),
            Err(ControlDatabaseError::ActiveLease { .. })
        ));
        heartbeat_lease(&connection, "catalog", "provider", "run-a", 102)
            .expect("owner should renew");
        assert!(matches!(
            heartbeat_lease(&connection, "catalog", "provider", "run-b", 103),
            Err(ControlDatabaseError::OwnershipLost { .. })
        ));
        release_lease(&connection, "catalog", "provider", "run-a").expect("owner should release");
        acquire_lease(&connection, "catalog", "provider", "run-b", 104)
            .expect("next owner should acquire");

        acquire_lease(&connection, "catalog", "expired", "run-old", 100)
            .expect("expiring owner should acquire");
        acquire_lease(&connection, "catalog", "expired", "run-new", 400)
            .expect("expired lease should be reclaimed");
    }

    fn current_control() -> Connection {
        let connection = Connection::open_in_memory().expect("control database should open");
        init_control_db(&connection).expect("control schema should initialize");
        connection
    }

    fn prepared_run(preparation: JournalSyncPreparation) -> ProviderRunCheckpoint {
        match preparation {
            JournalSyncPreparation::Run(run) => run,
            JournalSyncPreparation::Skip => panic!("fixture journal should not skip"),
        }
    }

    fn seed_anchor(connection: &Connection, provider_name: &str, anchor: Option<&str>) {
        seed_anchor_for(connection, provider_name, CATALOG_ID, anchor);
    }

    fn seed_anchor_for(
        connection: &Connection,
        provider_name: &str,
        catalog_id: &str,
        anchor: Option<&str>,
    ) {
        let run = prepared_run(
            prepare_journal_sync(
                connection,
                if catalog_id == "canonical-journal" || catalog_id == "legacy-journal" {
                    "english_journals"
                } else {
                    CATALOG_NAME
                },
                provider_name,
                catalog_id,
                "anchor-seed-run",
                IndexSyncMode::Incremental,
                false,
                TIMESTAMP,
            )
            .expect("anchor seed run should prepare"),
        );
        complete_sync_run(
            connection,
            if catalog_id == "canonical-journal" || catalog_id == "legacy-journal" {
                "english_journals"
            } else {
                CATALOG_NAME
            },
            provider_name,
            catalog_id,
            &run.run_id,
            run.mode,
            run.base_anchor.as_deref(),
            anchor,
            TIMESTAMP,
        )
        .expect("anchor seed should complete");
    }

    fn create_legacy_control_database(path: &std::path::Path, version: i64) -> Connection {
        let connection = Connection::open(path).expect("legacy control database should open");
        connection
            .execute_batch(
                "CREATE TABLE provider_leases (
                     catalog_name TEXT NOT NULL,
                     provider_name TEXT NOT NULL,
                     run_id TEXT NOT NULL,
                     heartbeat_at INTEGER NOT NULL,
                     expires_at INTEGER NOT NULL,
                     PRIMARY KEY (catalog_name, provider_name)
                 );
                 CREATE TABLE provider_checkpoints (
                     catalog_name TEXT NOT NULL,
                     provider_name TEXT NOT NULL,
                     scope_kind TEXT NOT NULL
                         CHECK (scope_kind IN ('listing', 'journal', 'year')),
                     scope_key TEXT NOT NULL,
                     checkpoint TEXT NOT NULL,
                     updated_at TEXT NOT NULL,
                     PRIMARY KEY (catalog_name, provider_name, scope_kind, scope_key)
                 );
                 CREATE INDEX idx_provider_checkpoints_catalog_provider
                     ON provider_checkpoints(catalog_name, provider_name);",
            )
            .expect("legacy control schema should initialize");
        connection
            .pragma_update(None, "user_version", version)
            .expect("legacy version should write");
        connection
    }

    fn insert_legacy_checkpoint(
        connection: &Connection,
        provider_name: &str,
        scope_kind: &str,
        scope_key: &str,
        checkpoint: &str,
    ) {
        connection
            .execute(
                "INSERT INTO provider_checkpoints (
                     catalog_name, provider_name, scope_kind, scope_key, checkpoint, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    CATALOG_NAME,
                    provider_name,
                    scope_kind,
                    scope_key,
                    checkpoint,
                    TIMESTAMP,
                ],
            )
            .expect("legacy checkpoint should write");
    }

    fn canonical_catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: CATALOG_ID.to_string(),
            catalog_aliases: Vec::new(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: Some("Systems".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn canonical_batch(progress: ProviderProgress) -> ProviderBatch {
        ProviderBatch {
            catalog_id: CATALOG_ID.to_string(),
            journal: JournalDraft {
                catalog_id: CATALOG_ID.to_string(),
                observed_title: Some("Canonical Journal".to_string()),
                observed_issns: vec!["1234-5679".to_string()],
                observed_title_aliases: Vec::new(),
            },
            issues: vec![IssueDraft {
                catalog_id: CATALOG_ID.to_string(),
                publication_year: Some(2026),
                title: None,
                volume: Some("1".to_string()),
                number: Some("2".to_string()),
                date: Some("2026-07".to_string()),
            }],
            articles: vec![ArticleDraft {
                catalog_id: CATALOG_ID.to_string(),
                title: "Canonical Article".to_string(),
                publication_year: Some(2026),
                date: Some("2026-07-18".to_string()),
                issue_title: None,
                volume: Some("1".to_string()),
                issue_number: Some("2".to_string()),
                authors: Vec::new(),
                start_page: Some("1".to_string()),
                end_page: Some("8".to_string()),
                abstract_text: Some("Canonical abstract".to_string()),
                doi: Some("10.1000/canonical".to_string()),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_dois: Vec::new(),
            }],
            progress,
        }
    }

    fn table_count(connection: &Connection, table_name: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
                row.get(0)
            })
            .expect("table count should load")
    }

    fn table_exists(connection: &Connection, table_name: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                 )",
                params![table_name],
                |row| row.get(0),
            )
            .expect("table existence should load")
    }
}
