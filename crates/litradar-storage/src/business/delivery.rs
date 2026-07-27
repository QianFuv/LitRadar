//! Durable delivery run, progress, dedupe, lease, and legacy-import repository.

use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{open_sqlite_connection, StorageConfig};

const MAX_DB_NAME_BYTES: usize = 255;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_ITEM_KEY_BYTES: usize = 512;
const MAX_RESULT_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_LEGACY_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MANUAL_DISPATCH_BATCH: usize = 64;

/// Delivery workflow persisted in the authentication database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryWorkflow {
    /// PushPlus notification delivery.
    Notify,
    /// Tracking-folder synchronization.
    Push,
}

impl DeliveryWorkflow {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notify => "notify",
            Self::Push => "push",
        }
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "notify" => Ok(Self::Notify),
            "push" => Ok(Self::Push),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Source that admitted a delivery run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryTriggerKind {
    /// Durable scheduler admission.
    Scheduled,
    /// Authenticated manual admission.
    Manual,
    /// Read-only legacy JSON import.
    Legacy,
}

impl DeliveryTriggerKind {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
            Self::Legacy => "legacy",
        }
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "scheduled" => Ok(Self::Scheduled),
            "manual" => Ok(Self::Manual),
            "legacy" => Ok(Self::Legacy),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Whether a delivery run may perform external side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryRunMode {
    /// Build a plan without side effects.
    DryRun,
    /// Execute approved side effects.
    Execute,
}

impl DeliveryRunMode {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry_run",
            Self::Execute => "execute",
        }
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "dry_run" => Ok(Self::DryRun),
            "execute" => Ok(Self::Execute),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Application-owned delivery run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryRunStatus {
    /// Waiting for a worker.
    Queued,
    /// Claimed but not yet executing side effects.
    Claimed,
    /// Actively executing.
    Running,
    /// Cancellation was requested and is being drained.
    Cancelling,
    /// Completed successfully.
    Completed,
    /// Failed with a known terminal error.
    Failed,
    /// Cancelled before or during execution.
    Cancelled,
    /// Exceeded its total deadline.
    TimedOut,
    /// Finished without applicable work.
    Skipped,
    /// Terminal state whose external outcome is not known.
    Unknown,
}

impl DeliveryRunStatus {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Skipped => "skipped",
            Self::Unknown => "unknown",
        }
    }

    /// Return whether the status owns an execution lease.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Claimed | Self::Running | Self::Cancelling)
    }

    /// Return whether the status is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Failed
                | Self::Cancelled
                | Self::TimedOut
                | Self::Skipped
                | Self::Unknown
        )
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "queued" => Ok(Self::Queued),
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "cancelling" => Ok(Self::Cancelling),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "skipped" => Ok(Self::Skipped),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Application-owned checkpoint status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryCheckpointStatus {
    /// No run is active and no completed state is available.
    Idle,
    /// A legacy checkpoint observed an active run.
    Running,
    /// Latest run completed.
    Completed,
    /// Latest run failed.
    Failed,
    /// Latest run had no applicable work.
    Skipped,
    /// Legacy or external state could not be classified.
    Unknown,
}

impl DeliveryCheckpointStatus {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "idle" => Ok(Self::Idle),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Kind of progress item owned by a delivery run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryItemKind {
    /// Issue checkpoint key.
    Issue,
    /// In-press journal checkpoint key.
    InPress,
    /// Candidate or delivered article.
    Article,
    /// Per-subscriber delivery result.
    Subscriber,
}

impl DeliveryItemKind {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::InPress => "inpress",
            Self::Article => "article",
            Self::Subscriber => "subscriber",
        }
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "issue" => Ok(Self::Issue),
            "inpress" => Ok(Self::InPress),
            "article" => Ok(Self::Article),
            "subscriber" => Ok(Self::Subscriber),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Application-owned delivery item status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryItemStatus {
    /// Waiting to be claimed.
    Pending,
    /// Claimed before an external side effect starts.
    Claimed,
    /// External delivery has started.
    Sending,
    /// Completed successfully.
    Succeeded,
    /// Failed before an ambiguous external outcome.
    Failed,
    /// Deliberately skipped.
    Skipped,
    /// Cancelled before completion.
    Cancelled,
    /// External outcome cannot be determined and must not be replayed automatically.
    Unknown,
}

impl DeliveryItemStatus {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Sending => "sending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    /// Return whether this item is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Skipped | Self::Cancelled | Self::Unknown
        )
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "sending" => Ok(Self::Sending),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            "cancelled" => Ok(Self::Cancelled),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Durable dedupe reservation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryDedupeStatus {
    /// Reserved before delivery begins.
    Reserved,
    /// Delivery completed with a known response.
    Confirmed,
    /// Delivery started but the response is ambiguous.
    Unknown,
}

impl DeliveryDedupeStatus {
    /// Return the canonical SQLite value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Confirmed => "confirmed",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Result<Self, DeliveryRepositoryError> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "confirmed" => Ok(Self::Confirmed),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DeliveryRepositoryError::InvalidStoredState),
        }
    }
}

/// Durable delivery repository error without payload or file content.
#[derive(Debug)]
pub enum DeliveryRepositoryError {
    /// SQLite rejected a repository operation.
    Sqlite(rusqlite::Error),
    /// Filesystem access failed.
    Io(std::io::Error),
    /// Legacy or result JSON was invalid.
    Json(serde_json::Error),
    /// A caller supplied an invalid bounded field or transition.
    InvalidInput(&'static str),
    /// A stored enum or invariant was not recognized.
    InvalidStoredState,
    /// A requested durable record does not exist.
    NotFound,
    /// A revision, owner, status, or import hash no longer matches.
    Conflict,
    /// A legacy file does not match its workflow or database identity.
    InvalidLegacyState,
    /// A legacy file changed after a prior successful import.
    LegacyImportConflict,
    /// A legacy file exceeds the bounded import size.
    LegacyStateTooLarge,
}

impl fmt::Display for DeliveryRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(_) => formatter.write_str("Delivery storage operation failed"),
            Self::Io(_) => formatter.write_str("Delivery state filesystem access failed"),
            Self::Json(_) | Self::InvalidLegacyState => {
                formatter.write_str("Legacy delivery state is invalid")
            }
            Self::InvalidInput(detail) => formatter.write_str(detail),
            Self::InvalidStoredState => formatter.write_str("Stored delivery state is invalid"),
            Self::NotFound => formatter.write_str("Delivery record not found"),
            Self::Conflict => formatter.write_str("Delivery state changed concurrently"),
            Self::LegacyImportConflict => {
                formatter.write_str("Legacy delivery state changed after import")
            }
            Self::LegacyStateTooLarge => formatter.write_str("Legacy delivery state is too large"),
        }
    }
}

impl Error for DeliveryRepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for DeliveryRepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for DeliveryRepositoryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for DeliveryRepositoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Durable workflow checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryCheckpointRecord {
    /// SQLite row identifier.
    pub id: i64,
    /// Delivery workflow.
    pub workflow: DeliveryWorkflow,
    /// Canonical index database filename.
    pub db_name: String,
    /// Latest checkpoint state.
    pub status: DeliveryCheckpointStatus,
    /// Unrecognized legacy status preserved during import.
    pub legacy_status: Option<String>,
    /// Canonical snapshot JSON.
    pub snapshot_json: String,
    /// Legacy or current completion timestamp.
    pub last_completed_run_at: Option<String>,
    /// Compare-and-swap revision.
    pub revision: i64,
    /// Imported legacy content hash.
    pub legacy_source_hash: Option<String>,
    /// Imported legacy filename without directory content.
    pub legacy_source_name: Option<String>,
    /// Legacy import Unix timestamp.
    pub legacy_imported_at: Option<f64>,
    /// Creation Unix timestamp.
    pub created_at: f64,
    /// Last update Unix timestamp.
    pub updated_at: f64,
}

/// Mutable checkpoint fields used by compare-and-swap updates.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryCheckpointUpdate {
    /// New checkpoint status.
    pub status: DeliveryCheckpointStatus,
    /// Canonical snapshot JSON.
    pub snapshot_json: String,
    /// Latest completed run timestamp.
    pub last_completed_run_at: Option<String>,
    /// Update Unix timestamp.
    pub updated_at: f64,
}

/// Input used to enqueue one delivery run.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRunCreate {
    /// External job or manifest run identifier.
    pub external_id: String,
    /// Delivery workflow.
    pub workflow: DeliveryWorkflow,
    /// Stable admission scope.
    pub scope_key: String,
    /// Optional index database filename.
    pub db_name: Option<String>,
    /// Admission source.
    pub trigger_kind: DeliveryTriggerKind,
    /// Side-effect mode.
    pub mode: DeliveryRunMode,
    /// Optional authenticated user identifier.
    pub user_id: Option<i64>,
    /// Optional total deadline Unix timestamp.
    pub deadline_at: Option<f64>,
    /// Creation Unix timestamp.
    pub created_at: f64,
}

/// Durable delivery run row.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRunRecord {
    /// Internal SQLite run identifier.
    pub id: i64,
    /// External job or manifest run identifier.
    pub external_id: String,
    /// Delivery workflow.
    pub workflow: DeliveryWorkflow,
    /// Stable admission scope.
    pub scope_key: String,
    /// Optional index database filename.
    pub db_name: Option<String>,
    /// Admission source.
    pub trigger_kind: DeliveryTriggerKind,
    /// Side-effect mode.
    pub mode: DeliveryRunMode,
    /// Optional authenticated user identifier.
    pub user_id: Option<i64>,
    /// Application-owned run status.
    pub status: DeliveryRunStatus,
    /// Unrecognized legacy status preserved during import.
    pub legacy_status: Option<String>,
    /// Current owner identifier.
    pub owner_id: Option<String>,
    /// Current owner lease expiration.
    pub lease_expires_at: Option<f64>,
    /// Total run deadline.
    pub deadline_at: Option<f64>,
    /// Whether cancellation was requested.
    pub cancellation_requested: bool,
    /// Bounded JSON result or imported legacy run payload.
    pub result_json: Option<String>,
    /// Fixed terminal error classification.
    pub error_code: Option<String>,
    /// Compare-and-swap revision.
    pub revision: i64,
    /// Creation Unix timestamp.
    pub created_at: f64,
    /// First claim Unix timestamp.
    pub started_at: Option<f64>,
    /// Last update Unix timestamp.
    pub updated_at: f64,
    /// Terminal Unix timestamp.
    pub finished_at: Option<f64>,
}

/// Outcome of claiming a durable run.
#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryRunClaimOutcome {
    /// The requested run was claimed or taken over after expiration.
    Claimed(DeliveryRunRecord),
    /// Another active run owns the same workflow/database scope.
    Busy(DeliveryRunRecord),
    /// The requested run is terminal or otherwise unavailable.
    Unavailable(DeliveryRunRecord),
}

/// Outcome of idempotently admitting one durable delivery run.
#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryRunAdmissionOutcome {
    /// A new queued run was inserted.
    Enqueued(DeliveryRunRecord),
    /// The same workflow, scope, and external identifier already exists.
    Existing(DeliveryRunRecord),
    /// Another manual run is already queued or active for the same user.
    Busy(DeliveryRunRecord),
}

/// Input used to create one run item.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRunItemCreate {
    /// Item kind.
    pub item_kind: DeliveryItemKind,
    /// Stable key unique within its run and kind.
    pub item_key: String,
    /// Optional user identifier.
    pub user_id: Option<i64>,
    /// Optional article identifier.
    pub article_id: Option<i64>,
}

/// Durable delivery run item.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRunItemRecord {
    /// SQLite item identifier.
    pub id: i64,
    /// Parent delivery run identifier.
    pub delivery_run_id: i64,
    /// Item kind.
    pub item_kind: DeliveryItemKind,
    /// Stable item key.
    pub item_key: String,
    /// Optional user identifier.
    pub user_id: Option<i64>,
    /// Optional article identifier.
    pub article_id: Option<i64>,
    /// Application-owned item status.
    pub status: DeliveryItemStatus,
    /// Unrecognized legacy status preserved during import.
    pub legacy_status: Option<String>,
    /// Current owner identifier.
    pub owner_id: Option<String>,
    /// Current owner lease expiration.
    pub lease_expires_at: Option<f64>,
    /// Number of claims.
    pub attempt_count: i64,
    /// Bounded result JSON.
    pub result_json: Option<String>,
    /// Fixed error classification.
    pub error_code: Option<String>,
    /// Compare-and-swap revision.
    pub revision: i64,
    /// Creation Unix timestamp.
    pub created_at: f64,
    /// First claim Unix timestamp.
    pub started_at: Option<f64>,
    /// Last update Unix timestamp.
    pub updated_at: f64,
    /// Terminal Unix timestamp.
    pub finished_at: Option<f64>,
}

/// Durable dedupe row.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryDedupeRecord {
    /// SQLite dedupe identifier.
    pub id: i64,
    /// Delivery workflow.
    pub workflow: DeliveryWorkflow,
    /// Index database filename.
    pub db_name: String,
    /// User identifier.
    pub user_id: i64,
    /// Article identifier.
    pub article_id: i64,
    /// Reserving or completing run identifier.
    pub delivery_run_id: Option<i64>,
    /// Dedupe status.
    pub status: DeliveryDedupeStatus,
    /// Optional upstream message identifier.
    pub message_id: Option<String>,
    /// Current reservation owner.
    pub reservation_owner: Option<String>,
    /// Original legacy timestamp text.
    pub legacy_delivered_at: Option<String>,
    /// Compare-and-swap revision.
    pub revision: i64,
    /// Reservation Unix timestamp.
    pub reserved_at: f64,
    /// Confirmed or unknown delivery Unix timestamp.
    pub delivered_at: Option<f64>,
    /// Last update Unix timestamp.
    pub updated_at: f64,
}

/// Outcome of reserving one dedupe identity.
#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryDedupeReserveOutcome {
    /// This caller inserted the reservation.
    Reserved(DeliveryDedupeRecord),
    /// A durable reservation or terminal row already exists.
    Existing(DeliveryDedupeRecord),
}

/// One dedupe row and revision expected by an atomic delivery transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryDedupeResolution {
    /// Durable dedupe row identifier.
    pub id: i64,
    /// Exact reservation revision.
    pub expected_revision: i64,
}

/// Durable workflow/database lease.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryLeaseRecord {
    /// SQLite lease identifier.
    pub id: i64,
    /// Delivery workflow.
    pub workflow: DeliveryWorkflow,
    /// Index database filename.
    pub db_name: String,
    /// Owning delivery run identifier.
    pub delivery_run_id: Option<i64>,
    /// Owning worker identifier.
    pub owner_id: Option<String>,
    /// Compare-and-swap revision.
    pub revision: i64,
    /// Acquisition Unix timestamp.
    pub acquired_at: Option<f64>,
    /// Last heartbeat Unix timestamp.
    pub heartbeat_at: Option<f64>,
    /// Lease expiration Unix timestamp.
    pub expires_at: Option<f64>,
    /// Last update Unix timestamp.
    pub updated_at: f64,
}

/// Outcome of acquiring a workflow/database lease.
#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryLeaseAcquireOutcome {
    /// This caller acquired a free or expired lease.
    Acquired(DeliveryLeaseRecord),
    /// Another unexpired owner holds the lease.
    Busy(DeliveryLeaseRecord),
}

/// Aggregate result of one all-or-nothing legacy import scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeliveryImportResult {
    /// Legacy files discovered before parsing.
    pub discovered_count: usize,
    /// Files imported during this call.
    pub imported_count: usize,
    /// Files skipped because the same content hash was already imported.
    pub skipped_count: usize,
    /// Run item rows imported.
    pub item_count: usize,
    /// Dedupe rows imported.
    pub dedupe_count: usize,
}

/// Aggregate rows reconciled after an expired run owner is replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryRecoveryResult {
    /// Pre-send item claims returned to pending.
    pub reset_item_count: usize,
    /// Sending items finalized with an ambiguous outcome.
    pub unknown_item_count: usize,
    /// Pre-send dedupe reservations released for safe retry.
    pub released_dedupe_count: usize,
    /// Sending dedupe reservations finalized as ambiguous.
    pub unknown_dedupe_count: usize,
}

/// Rows committed by one atomic run/checkpoint/lease finalization.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRunFinalization {
    /// Terminal delivery run.
    pub run: DeliveryRunRecord,
    /// Updated workflow checkpoint.
    pub checkpoint: DeliveryCheckpointRecord,
    /// Released workflow lease with its incremented revision.
    pub lease: DeliveryLeaseRecord,
}

/// Load a workflow checkpoint by its stable scope.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `workflow` - Delivery workflow.
/// * `db_name` - Canonical index database filename.
///
/// # Returns
///
/// Existing checkpoint, or `None` before the first committed checkpoint.
pub fn load_delivery_checkpoint(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
) -> Result<Option<DeliveryCheckpointRecord>, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    let connection = open_delivery_connection(auth_db_path)?;
    load_delivery_checkpoint_from_connection(&connection, workflow, db_name)
}

/// Insert or compare-and-swap one workflow checkpoint.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `workflow` - Delivery workflow.
/// * `db_name` - Canonical index database filename.
/// * `expected_revision` - `None` for first insert, or the exact observed revision.
/// * `update` - Validated checkpoint payload.
///
/// # Returns
///
/// Inserted or updated checkpoint with its current revision.
pub fn compare_and_swap_delivery_checkpoint(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    expected_revision: Option<i64>,
    update: &DeliveryCheckpointUpdate,
) -> Result<DeliveryCheckpointRecord, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_json(&update.snapshot_json)?;
    validate_time(update.updated_at, "Checkpoint update time is invalid")?;
    if expected_revision.is_some_and(|revision| revision < 0) {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Checkpoint revision is invalid",
        ));
    }
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match expected_revision {
        None => {
            let inserted = transaction.execute(
                "INSERT INTO delivery_checkpoints
                 (workflow, db_name, status, legacy_status, snapshot_json,
                  last_completed_run_at, revision, legacy_source_hash, legacy_source_name,
                  legacy_imported_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, 0, NULL, NULL, NULL, ?6, ?7)
                 ON CONFLICT(workflow, db_name) DO NOTHING",
                params![
                    workflow.as_str(),
                    db_name,
                    update.status.as_str(),
                    update.snapshot_json,
                    update.last_completed_run_at,
                    update.updated_at,
                    update.updated_at,
                ],
            )?;
            if inserted != 1 {
                return Err(DeliveryRepositoryError::Conflict);
            }
        }
        Some(revision) => {
            let updated = transaction.execute(
                "UPDATE delivery_checkpoints
                 SET status = ?1, legacy_status = NULL, snapshot_json = ?2,
                     last_completed_run_at = ?3, revision = revision + 1, updated_at = ?4
                 WHERE workflow = ?5 AND db_name = ?6 AND revision = ?7",
                params![
                    update.status.as_str(),
                    update.snapshot_json,
                    update.last_completed_run_at,
                    update.updated_at,
                    workflow.as_str(),
                    db_name,
                    revision,
                ],
            )?;
            if updated != 1 {
                return Err(DeliveryRepositoryError::Conflict);
            }
        }
    }
    let record = load_delivery_checkpoint_from_connection(&transaction, workflow, db_name)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// Enqueue one durable delivery run.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `run` - Bounded run identity, scope, mode, and deadline.
///
/// # Returns
///
/// Newly inserted queued run.
pub fn enqueue_delivery_run(
    auth_db_path: impl AsRef<Path>,
    run: &DeliveryRunCreate,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError> {
    match admit_delivery_run(auth_db_path, run)? {
        DeliveryRunAdmissionOutcome::Enqueued(record) => Ok(record),
        DeliveryRunAdmissionOutcome::Existing(_) | DeliveryRunAdmissionOutcome::Busy(_) => {
            Err(DeliveryRepositoryError::Conflict)
        }
    }
}

/// Idempotently enqueue or load one delivery run identity.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `run` - Bounded run identity, scope, mode, and deadline.
///
/// # Returns
///
/// A newly queued run, the exact existing run, or the conflicting active manual run.
pub fn admit_delivery_run(
    auth_db_path: impl AsRef<Path>,
    run: &DeliveryRunCreate,
) -> Result<DeliveryRunAdmissionOutcome, DeliveryRepositoryError> {
    validate_run_create(run)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let inserted = transaction.execute(
        "INSERT INTO delivery_runs
         (external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
          legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
          result_json, error_code, revision, created_at, started_at, updated_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', NULL, NULL, NULL, ?8, 0,
                 NULL, NULL, 0, ?9, NULL, ?10, NULL)
         ON CONFLICT DO NOTHING",
        params![
            run.external_id,
            run.workflow.as_str(),
            run.scope_key,
            run.db_name,
            run.trigger_kind.as_str(),
            run.mode.as_str(),
            run.user_id,
            run.deadline_at,
            run.created_at,
            run.created_at,
        ],
    )?;
    let outcome = if inserted == 1 {
        let id = transaction.last_insert_rowid();
        DeliveryRunAdmissionOutcome::Enqueued(
            load_delivery_run_from_connection(&transaction, id)?
                .ok_or(DeliveryRepositoryError::NotFound)?,
        )
    } else if let Some(existing) = load_delivery_run_by_external_id_from_connection(
        &transaction,
        run.workflow,
        &run.scope_key,
        &run.external_id,
    )? {
        DeliveryRunAdmissionOutcome::Existing(existing)
    } else if run.trigger_kind == DeliveryTriggerKind::Manual {
        let user_id = run.user_id.ok_or(DeliveryRepositoryError::InvalidInput(
            "Manual delivery runs require a user",
        ))?;
        let existing = load_active_manual_delivery_run_from_connection(&transaction, user_id)?
            .ok_or(DeliveryRepositoryError::Conflict)?;
        DeliveryRunAdmissionOutcome::Busy(existing)
    } else {
        return Err(DeliveryRepositoryError::Conflict);
    };
    transaction.commit()?;
    Ok(outcome)
}

/// Load one delivery run by its internal identifier.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `delivery_run_id` - Internal run identifier.
///
/// # Returns
///
/// Existing run, or `None`.
pub fn load_delivery_run(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    let connection = open_delivery_connection(auth_db_path)?;
    load_delivery_run_from_connection(&connection, delivery_run_id)
}

/// Load the most recent manual delivery run for one authenticated user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `user_id` - Authenticated user identifier.
///
/// # Returns
///
/// Latest queued, active, or terminal manual run for the user.
pub fn load_latest_manual_delivery_run(
    auth_db_path: impl AsRef<Path>,
    user_id: i64,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    validate_positive_id(user_id, "Manual delivery user id is invalid")?;
    let connection = open_delivery_connection(auth_db_path)?;
    connection
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM delivery_runs
                 WHERE trigger_kind = 'manual' AND user_id = ?1
                 ORDER BY id DESC LIMIT 1"
            ),
            [user_id],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

/// Load one user-owned manual delivery run by its public external identifier.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `user_id` - Authenticated user identifier.
/// * `external_id` - Public opaque job identifier.
///
/// # Returns
///
/// Matching user-owned manual run, or `None`.
pub fn load_manual_delivery_run_by_external_id(
    auth_db_path: impl AsRef<Path>,
    user_id: i64,
    external_id: &str,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    validate_positive_id(user_id, "Manual delivery user id is invalid")?;
    validate_identifier(external_id, "Manual delivery job id is invalid")?;
    let connection = open_delivery_connection(auth_db_path)?;
    connection
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM delivery_runs
                 WHERE trigger_kind = 'manual' AND user_id = ?1 AND external_id = ?2
                 ORDER BY id DESC LIMIT 1"
            ),
            params![user_id, external_id],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

/// Load one manual delivery run by its public external identifier without owner filtering.
///
/// This repository operation is intended for an already-authorized administrator route.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `external_id` - Public opaque job identifier.
///
/// # Returns
///
/// Matching manual run, or `None`.
pub fn load_manual_delivery_run_by_external_id_for_admin(
    auth_db_path: impl AsRef<Path>,
    external_id: &str,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    validate_identifier(external_id, "Manual delivery job id is invalid")?;
    let connection = open_delivery_connection(auth_db_path)?;
    connection
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM delivery_runs
                 WHERE trigger_kind = 'manual' AND external_id = ?1
                 ORDER BY id DESC LIMIT 1"
            ),
            [external_id],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

/// List queued or lease-expired manual runs eligible for bounded dispatch.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `now` - Current Unix timestamp used for expired-lease selection.
/// * `limit` - Positive bounded number of rows to return.
///
/// # Returns
///
/// Oldest dispatchable manual runs in stable insertion order.
pub fn list_dispatchable_manual_delivery_runs(
    auth_db_path: impl AsRef<Path>,
    now: f64,
    limit: usize,
) -> Result<Vec<DeliveryRunRecord>, DeliveryRepositoryError> {
    validate_time(now, "Manual delivery dispatch time is invalid")?;
    if !(1..=MAX_MANUAL_DISPATCH_BATCH).contains(&limit) {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Manual delivery dispatch limit is invalid",
        ));
    }
    let connection = open_delivery_connection(auth_db_path)?;
    let mut statement = connection.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM delivery_runs
         WHERE trigger_kind = 'manual'
           AND (status = 'queued'
                OR (status IN ('claimed', 'running', 'cancelling')
                    AND lease_expires_at <= ?1))
         ORDER BY created_at, id LIMIT ?2"
    ))?;
    let limit = i64::try_from(limit).expect("bounded dispatch limit should fit i64");
    let rows = statement.query_map(params![now, limit], run_from_row)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(DeliveryRepositoryError::from)
}

/// Finalize a queued run that could not start a supervised child process.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `delivery_run_id` - Internal run identifier.
/// * `expected_revision` - Exact queued revision observed by the dispatcher.
/// * `terminal_status` - Failed, cancelled, or timed-out terminal status.
/// * `result_json` - Optional bounded result payload.
/// * `error_code` - Optional fixed terminal classification.
/// * `now` - Terminal Unix timestamp.
///
/// # Returns
///
/// Finalized run, or a conflict when another dispatcher claimed it first.
#[allow(clippy::too_many_arguments)]
pub fn finalize_queued_delivery_run(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    expected_revision: i64,
    terminal_status: DeliveryRunStatus,
    result_json: Option<&str>,
    error_code: Option<&str>,
    now: f64,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    if !matches!(
        terminal_status,
        DeliveryRunStatus::Failed | DeliveryRunStatus::Cancelled | DeliveryRunStatus::TimedOut
    ) {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Queued delivery terminal status is invalid",
        ));
    }
    validate_optional_json(result_json)?;
    validate_optional_symbol(error_code, "Delivery error code is invalid")?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE delivery_runs
         SET status = ?1, result_json = ?2, error_code = ?3, updated_at = ?4,
             finished_at = ?5, revision = revision + 1
         WHERE id = ?6 AND revision = ?7 AND status = 'queued'",
        params![
            terminal_status.as_str(),
            result_json,
            error_code,
            now,
            now,
            delivery_run_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// List all durable items for one delivery run in insertion order.
pub fn list_delivery_run_items(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
) -> Result<Vec<DeliveryRunItemRecord>, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    let connection = open_delivery_connection(auth_db_path)?;
    let mut statement = connection.prepare(&format!(
        "SELECT {ITEM_COLUMNS} FROM delivery_run_items
         WHERE delivery_run_id = ?1 ORDER BY id"
    ))?;
    let rows = statement.query_map([delivery_run_id], item_from_row)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(DeliveryRepositoryError::from)
}

/// Ensure a bounded set of run item identities exists without resetting progress.
pub fn ensure_delivery_run_items(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    items: &[DeliveryRunItemCreate],
    now: f64,
) -> Result<Vec<DeliveryRunItemRecord>, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_time(now, "Delivery item creation time is invalid")?;
    let mut identities = HashSet::new();
    for item in items {
        validate_item_create(item)?;
        if !identities.insert((item.item_kind, item.item_key.as_str())) {
            return Err(DeliveryRepositoryError::InvalidInput(
                "Delivery run items contain duplicate identities",
            ));
        }
    }
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if run.status.is_terminal() {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        transaction.execute(
            "INSERT INTO delivery_run_items
             (delivery_run_id, item_kind, item_key, user_id, article_id, status,
              legacy_status, owner_id, lease_expires_at, attempt_count, result_json,
              error_code, revision, created_at, started_at, updated_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, NULL, 0,
                     NULL, NULL, 0, ?6, NULL, ?7, NULL)
             ON CONFLICT(delivery_run_id, item_kind, item_key) DO NOTHING",
            params![
                delivery_run_id,
                item.item_kind.as_str(),
                item.item_key,
                item.user_id,
                item.article_id,
                now,
                now,
            ],
        )?;
        let record = load_delivery_run_item_by_identity_from_connection(
            &transaction,
            delivery_run_id,
            item.item_kind,
            &item.item_key,
        )?
        .ok_or(DeliveryRepositoryError::NotFound)?;
        if record.user_id != item.user_id || record.article_id != item.article_id {
            return Err(DeliveryRepositoryError::Conflict);
        }
        records.push(record);
    }
    transaction.commit()?;
    Ok(records)
}

/// Load one dedupe identity if it has ever been reserved or completed.
pub fn load_delivery_dedupe(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    user_id: i64,
    article_id: i64,
) -> Result<Option<DeliveryDedupeRecord>, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_positive_id(user_id, "Delivery dedupe user id is invalid")?;
    validate_positive_id(article_id, "Delivery dedupe article id is invalid")?;
    let connection = open_delivery_connection(auth_db_path)?;
    load_delivery_dedupe_from_connection(&connection, workflow, db_name, user_id, article_id)
}

/// List all dedupe rows for one workflow/database scope.
pub fn list_delivery_dedupe_for_scope(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
) -> Result<Vec<DeliveryDedupeRecord>, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    let connection = open_delivery_connection(auth_db_path)?;
    let mut statement = connection.prepare(&format!(
        "SELECT {DEDUPE_COLUMNS} FROM delivery_dedupe
         WHERE workflow = ?1 AND db_name = ?2 ORDER BY user_id, article_id"
    ))?;
    let rows = statement.query_map(params![workflow.as_str(), db_name], dedupe_from_row)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(DeliveryRepositoryError::from)
}

/// Delete confirmed dedupe rows older than a retention cutoff.
pub fn cleanup_confirmed_delivery_dedupe(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    delivered_before: f64,
) -> Result<usize, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_time(
        delivered_before,
        "Delivery dedupe retention cutoff is invalid",
    )?;
    let connection = open_delivery_connection(auth_db_path)?;
    connection
        .execute(
            "DELETE FROM delivery_dedupe
             WHERE workflow = ?1 AND db_name = ?2 AND status = 'confirmed'
               AND delivered_at < ?3",
            params![workflow.as_str(), db_name, delivered_before],
        )
        .map_err(DeliveryRepositoryError::from)
}

/// Load the persistent workflow/database lease row.
pub fn load_delivery_lease(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
) -> Result<Option<DeliveryLeaseRecord>, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    let connection = open_delivery_connection(auth_db_path)?;
    load_delivery_lease_from_connection(&connection, workflow, db_name)
}

/// Claim a queued run or take over the same run after its owner lease expires.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `delivery_run_id` - Internal run identifier.
/// * `owner_id` - New worker owner identifier.
/// * `expected_revision` - Exact revision observed before claiming.
/// * `now` - Current Unix timestamp.
/// * `lease_seconds` - Positive lease duration.
///
/// # Returns
///
/// Claimed run, competing active scope owner, or unavailable requested run.
pub fn claim_delivery_run(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
    lease_seconds: f64,
) -> Result<DeliveryRunClaimOutcome, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery owner id is invalid")?;
    validate_revision_and_lease(expected_revision, now, lease_seconds)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let requested = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if requested.revision != expected_revision {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let can_claim = requested.status == DeliveryRunStatus::Queued
        || (requested.status.is_active()
            && requested
                .lease_expires_at
                .is_some_and(|expires_at| expires_at <= now));
    if !can_claim {
        transaction.commit()?;
        return Ok(DeliveryRunClaimOutcome::Unavailable(requested));
    }
    if let Some(db_name) = requested.db_name.as_deref() {
        if let Some(active) =
            load_competing_active_run(&transaction, delivery_run_id, requested.workflow, db_name)?
        {
            transaction.commit()?;
            return Ok(DeliveryRunClaimOutcome::Busy(active));
        }
    }
    let next_status = if requested.cancellation_requested {
        DeliveryRunStatus::Cancelling
    } else {
        DeliveryRunStatus::Claimed
    };
    let updated = transaction.execute(
        "UPDATE delivery_runs
         SET status = ?1, owner_id = ?2, lease_expires_at = ?3,
             started_at = COALESCE(started_at, ?4), updated_at = ?5, revision = revision + 1
         WHERE id = ?6 AND revision = ?7",
        params![
            next_status.as_str(),
            owner_id,
            now + lease_seconds,
            now,
            now,
            delivery_run_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let claimed = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(DeliveryRunClaimOutcome::Claimed(claimed))
}

/// Renew the lease held by an active run owner with revision CAS.
pub fn renew_delivery_run(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
    lease_seconds: f64,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery owner id is invalid")?;
    validate_revision_and_lease(expected_revision, now, lease_seconds)?;
    update_run_with_owner_cas(
        auth_db_path,
        delivery_run_id,
        owner_id,
        expected_revision,
        "UPDATE delivery_runs
         SET lease_expires_at = ?1, updated_at = ?2, revision = revision + 1
         WHERE id = ?3 AND owner_id = ?4 AND revision = ?5
           AND status IN ('claimed', 'running', 'cancelling')
           AND lease_expires_at > ?2",
        params![
            now + lease_seconds,
            now,
            delivery_run_id,
            owner_id,
            expected_revision,
        ],
    )
}

/// Transition a claimed run to running with revision and owner CAS.
pub fn start_delivery_run(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery owner id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    update_run_with_owner_cas(
        auth_db_path,
        delivery_run_id,
        owner_id,
        expected_revision,
        "UPDATE delivery_runs
         SET status = 'running', updated_at = ?1, revision = revision + 1
         WHERE id = ?2 AND owner_id = ?3 AND revision = ?4
           AND status = 'claimed' AND lease_expires_at > ?1",
        params![now, delivery_run_id, owner_id, expected_revision],
    )
}

/// Request cancellation of a queued or active run with revision CAS.
pub fn request_delivery_run_cancellation(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    expected_revision: i64,
    now: f64,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if current.revision != expected_revision || current.status.is_terminal() {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let (status, finished_at) = if current.status == DeliveryRunStatus::Queued {
        (DeliveryRunStatus::Cancelled, Some(now))
    } else if current.status.is_active() {
        (DeliveryRunStatus::Cancelling, None)
    } else {
        return Err(DeliveryRepositoryError::Conflict);
    };
    let updated = transaction.execute(
        "UPDATE delivery_runs
         SET status = ?1, cancellation_requested = 1, updated_at = ?2, finished_at = ?3,
             revision = revision + 1
         WHERE id = ?4 AND revision = ?5",
        params![
            status.as_str(),
            now,
            finished_at,
            delivery_run_id,
            expected_revision
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// Finalize an active run only for its current owner and revision.
#[allow(clippy::too_many_arguments)]
pub fn finalize_delivery_run(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    terminal_status: DeliveryRunStatus,
    result_json: Option<&str>,
    error_code: Option<&str>,
    now: f64,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery owner id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    if !terminal_status.is_terminal() {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery terminal status is invalid",
        ));
    }
    validate_optional_json(result_json)?;
    validate_optional_symbol(error_code, "Delivery error code is invalid")?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE delivery_runs
         SET status = ?1, owner_id = NULL, lease_expires_at = NULL, result_json = ?2,
             error_code = ?3, updated_at = ?4, finished_at = ?5, revision = revision + 1
         WHERE id = ?6 AND owner_id = ?7 AND revision = ?8
           AND status IN ('claimed', 'running', 'cancelling')",
        params![
            terminal_status.as_str(),
            result_json,
            error_code,
            now,
            now,
            delivery_run_id,
            owner_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// Atomically finalize a run, compare-and-swap its checkpoint, and release its lease.
#[allow(clippy::too_many_arguments)]
pub fn finalize_delivery_run_with_checkpoint(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    expected_run_revision: i64,
    terminal_status: DeliveryRunStatus,
    result_json: Option<&str>,
    error_code: Option<&str>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    expected_checkpoint_revision: Option<i64>,
    checkpoint_update: &DeliveryCheckpointUpdate,
    expected_lease_revision: i64,
) -> Result<DeliveryRunFinalization, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery owner id is invalid")?;
    validate_db_name(db_name)?;
    validate_revision_and_time(expected_run_revision, checkpoint_update.updated_at)?;
    if expected_lease_revision < 0
        || expected_checkpoint_revision.is_some_and(|revision| revision < 0)
    {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery revision is invalid",
        ));
    }
    if !terminal_status.is_terminal() {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery terminal status is invalid",
        ));
    }
    validate_json(&checkpoint_update.snapshot_json)?;
    validate_optional_json(result_json)?;
    validate_optional_symbol(error_code, "Delivery error code is invalid")?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match expected_checkpoint_revision {
        None => {
            if transaction.execute(
                "INSERT INTO delivery_checkpoints
                 (workflow, db_name, status, legacy_status, snapshot_json,
                  last_completed_run_at, revision, legacy_source_hash, legacy_source_name,
                  legacy_imported_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, 0, NULL, NULL, NULL, ?6, ?7)
                 ON CONFLICT(workflow, db_name) DO NOTHING",
                params![
                    workflow.as_str(),
                    db_name,
                    checkpoint_update.status.as_str(),
                    checkpoint_update.snapshot_json,
                    checkpoint_update.last_completed_run_at,
                    checkpoint_update.updated_at,
                    checkpoint_update.updated_at,
                ],
            )? != 1
            {
                return Err(DeliveryRepositoryError::Conflict);
            }
        }
        Some(revision) => {
            if transaction.execute(
                "UPDATE delivery_checkpoints
                 SET status = ?1, legacy_status = NULL, snapshot_json = ?2,
                     last_completed_run_at = ?3, revision = revision + 1, updated_at = ?4
                 WHERE workflow = ?5 AND db_name = ?6 AND revision = ?7",
                params![
                    checkpoint_update.status.as_str(),
                    checkpoint_update.snapshot_json,
                    checkpoint_update.last_completed_run_at,
                    checkpoint_update.updated_at,
                    workflow.as_str(),
                    db_name,
                    revision,
                ],
            )? != 1
            {
                return Err(DeliveryRepositoryError::Conflict);
            }
        }
    }
    if transaction.execute(
        "UPDATE delivery_runs
         SET status = ?1, owner_id = NULL, lease_expires_at = NULL, result_json = ?2,
             error_code = ?3, updated_at = ?4, finished_at = ?5, revision = revision + 1
         WHERE id = ?6 AND workflow = ?7 AND db_name = ?8 AND owner_id = ?9
           AND revision = ?10 AND status IN ('claimed', 'running', 'cancelling')",
        params![
            terminal_status.as_str(),
            result_json,
            error_code,
            checkpoint_update.updated_at,
            checkpoint_update.updated_at,
            delivery_run_id,
            workflow.as_str(),
            db_name,
            owner_id,
            expected_run_revision,
        ],
    )? != 1
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    if transaction.execute(
        "UPDATE delivery_leases
         SET delivery_run_id = NULL, owner_id = NULL, acquired_at = NULL,
             heartbeat_at = NULL, expires_at = NULL, updated_at = ?1, revision = revision + 1
         WHERE workflow = ?2 AND db_name = ?3 AND delivery_run_id = ?4
           AND owner_id = ?5 AND revision = ?6",
        params![
            checkpoint_update.updated_at,
            workflow.as_str(),
            db_name,
            delivery_run_id,
            owner_id,
            expected_lease_revision,
        ],
    )? != 1
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    let checkpoint = load_delivery_checkpoint_from_connection(&transaction, workflow, db_name)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    let lease = load_delivery_lease_from_connection(&transaction, workflow, db_name)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(DeliveryRunFinalization {
        run,
        checkpoint,
        lease,
    })
}

/// Insert a bounded set of unique items for one queued or active run.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `delivery_run_id` - Parent run identifier.
/// * `items` - Stable item identities.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Inserted item rows in input order.
pub fn insert_delivery_run_items(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    items: &[DeliveryRunItemCreate],
    now: f64,
) -> Result<Vec<DeliveryRunItemRecord>, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_time(now, "Delivery item creation time is invalid")?;
    let mut identities = HashSet::new();
    for item in items {
        validate_item_create(item)?;
        if !identities.insert((item.item_kind, item.item_key.as_str())) {
            return Err(DeliveryRepositoryError::InvalidInput(
                "Delivery run items contain duplicate identities",
            ));
        }
    }
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if load_delivery_run_from_connection(&transaction, delivery_run_id)?.is_none() {
        return Err(DeliveryRepositoryError::NotFound);
    }
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        transaction.execute(
            "INSERT INTO delivery_run_items
             (delivery_run_id, item_kind, item_key, user_id, article_id, status,
              legacy_status, owner_id, lease_expires_at, attempt_count, result_json,
              error_code, revision, created_at, started_at, updated_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, NULL, 0,
                     NULL, NULL, 0, ?6, NULL, ?7, NULL)",
            params![
                delivery_run_id,
                item.item_kind.as_str(),
                item.item_key,
                item.user_id,
                item.article_id,
                now,
                now,
            ],
        )?;
        let item_id = transaction.last_insert_rowid();
        records.push(
            load_delivery_run_item_from_connection(&transaction, item_id)?
                .ok_or(DeliveryRepositoryError::NotFound)?,
        );
    }
    transaction.commit()?;
    Ok(records)
}

/// Claim the next pending item or safely take over an expired pre-send claim.
#[allow(clippy::too_many_arguments)]
pub fn claim_next_delivery_run_item(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    run_owner_id: &str,
    run_revision: i64,
    item_owner_id: &str,
    now: f64,
    lease_seconds: f64,
) -> Result<Option<DeliveryRunItemRecord>, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(run_owner_id, "Delivery run owner id is invalid")?;
    validate_identifier(item_owner_id, "Delivery item owner id is invalid")?;
    validate_revision_and_lease(run_revision, now, lease_seconds)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if run.revision != run_revision
        || run.owner_id.as_deref() != Some(run_owner_id)
        || !run.status.is_active()
        || !run
            .lease_expires_at
            .is_some_and(|expires_at| expires_at > now)
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let item_id = transaction
        .query_row(
            "SELECT id FROM delivery_run_items
             WHERE delivery_run_id = ?1
               AND (status = 'pending' OR (status = 'claimed' AND lease_expires_at <= ?2))
             ORDER BY id LIMIT 1",
            params![delivery_run_id, now],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(item_id) = item_id else {
        transaction.commit()?;
        return Ok(None);
    };
    let updated = transaction.execute(
        "UPDATE delivery_run_items
         SET status = 'claimed', owner_id = ?1, lease_expires_at = ?2,
             attempt_count = attempt_count + 1, started_at = COALESCE(started_at, ?3),
             updated_at = ?4, revision = revision + 1
         WHERE id = ?5
           AND (status = 'pending' OR (status = 'claimed' AND lease_expires_at <= ?3))",
        params![item_owner_id, now + lease_seconds, now, now, item_id],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let item = load_delivery_run_item_from_connection(&transaction, item_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(Some(item))
}

/// Claim one known pending item or take over its expired pre-send claim.
#[allow(clippy::too_many_arguments)]
pub fn claim_delivery_run_item(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    run_owner_id: &str,
    run_revision: i64,
    item_id: i64,
    item_owner_id: &str,
    now: f64,
    lease_seconds: f64,
) -> Result<DeliveryRunItemRecord, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_positive_id(item_id, "Delivery item id is invalid")?;
    validate_identifier(run_owner_id, "Delivery run owner id is invalid")?;
    validate_identifier(item_owner_id, "Delivery item owner id is invalid")?;
    validate_revision_and_lease(run_revision, now, lease_seconds)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if run.revision != run_revision
        || run.owner_id.as_deref() != Some(run_owner_id)
        || !run.status.is_active()
        || !run
            .lease_expires_at
            .is_some_and(|expires_at| expires_at > now)
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let item = load_delivery_run_item_from_connection(&transaction, item_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if item.delivery_run_id != delivery_run_id
        || !(item.status == DeliveryItemStatus::Pending
            || (item.status == DeliveryItemStatus::Claimed
                && item
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at <= now)))
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    if transaction.execute(
        "UPDATE delivery_run_items
         SET status = 'claimed', owner_id = ?1, lease_expires_at = ?2,
             attempt_count = attempt_count + 1, started_at = COALESCE(started_at, ?3),
             updated_at = ?4, revision = revision + 1
         WHERE id = ?5 AND delivery_run_id = ?6 AND revision = ?7
           AND (status = 'pending' OR (status = 'claimed' AND lease_expires_at <= ?3))",
        params![
            item_owner_id,
            now + lease_seconds,
            now,
            now,
            item_id,
            delivery_run_id,
            item.revision,
        ],
    )? != 1
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let claimed = load_delivery_run_item_from_connection(&transaction, item_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(claimed)
}

/// Renew a claimed or sending item lease with owner and revision CAS.
pub fn renew_delivery_run_item(
    auth_db_path: impl AsRef<Path>,
    item_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
    lease_seconds: f64,
) -> Result<DeliveryRunItemRecord, DeliveryRepositoryError> {
    validate_positive_id(item_id, "Delivery item id is invalid")?;
    validate_identifier(owner_id, "Delivery item owner id is invalid")?;
    validate_revision_and_lease(expected_revision, now, lease_seconds)?;
    update_item_with_owner_cas(
        auth_db_path,
        item_id,
        owner_id,
        expected_revision,
        "UPDATE delivery_run_items
         SET lease_expires_at = ?1, updated_at = ?2, revision = revision + 1
         WHERE id = ?3 AND owner_id = ?4 AND revision = ?5
           AND status IN ('claimed', 'sending') AND lease_expires_at > ?2",
        params![
            now + lease_seconds,
            now,
            item_id,
            owner_id,
            expected_revision
        ],
    )
}

/// Mark a claimed item as externally sending before the side effect begins.
pub fn mark_delivery_run_item_sending(
    auth_db_path: impl AsRef<Path>,
    item_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
) -> Result<DeliveryRunItemRecord, DeliveryRepositoryError> {
    validate_positive_id(item_id, "Delivery item id is invalid")?;
    validate_identifier(owner_id, "Delivery item owner id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    update_item_with_owner_cas(
        auth_db_path,
        item_id,
        owner_id,
        expected_revision,
        "UPDATE delivery_run_items
         SET status = 'sending', updated_at = ?1, revision = revision + 1
         WHERE id = ?2 AND owner_id = ?3 AND revision = ?4
           AND status = 'claimed' AND lease_expires_at > ?1",
        params![now, item_id, owner_id, expected_revision],
    )
}

/// Finalize a claimed or sending item with owner and revision CAS.
#[allow(clippy::too_many_arguments)]
pub fn finalize_delivery_run_item(
    auth_db_path: impl AsRef<Path>,
    item_id: i64,
    owner_id: &str,
    expected_revision: i64,
    terminal_status: DeliveryItemStatus,
    result_json: Option<&str>,
    error_code: Option<&str>,
    now: f64,
) -> Result<DeliveryRunItemRecord, DeliveryRepositoryError> {
    validate_positive_id(item_id, "Delivery item id is invalid")?;
    validate_identifier(owner_id, "Delivery item owner id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    if !terminal_status.is_terminal() {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery item terminal status is invalid",
        ));
    }
    validate_optional_json(result_json)?;
    validate_optional_symbol(error_code, "Delivery item error code is invalid")?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE delivery_run_items
         SET status = ?1, owner_id = NULL, lease_expires_at = NULL, result_json = ?2,
             error_code = ?3, updated_at = ?4, finished_at = ?5, revision = revision + 1
         WHERE id = ?6 AND owner_id = ?7 AND revision = ?8
           AND status IN ('claimed', 'sending')",
        params![
            terminal_status.as_str(),
            result_json,
            error_code,
            now,
            now,
            item_id,
            owner_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let item = load_delivery_run_item_from_connection(&transaction, item_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(item)
}

/// Reserve one workflow/database/user/article identity using SQLite uniqueness.
#[allow(clippy::too_many_arguments)]
pub fn reserve_delivery_dedupe(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    user_id: i64,
    article_id: i64,
    delivery_run_id: i64,
    owner_id: &str,
    now: f64,
) -> Result<DeliveryDedupeReserveOutcome, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_positive_id(user_id, "Delivery dedupe user id is invalid")?;
    validate_positive_id(article_id, "Delivery dedupe article id is invalid")?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery dedupe owner id is invalid")?;
    validate_time(now, "Delivery dedupe reservation time is invalid")?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if run.workflow != workflow || run.db_name.as_deref() != Some(db_name) {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery dedupe scope does not match its run",
        ));
    }
    let inserted = transaction.execute(
        "INSERT INTO delivery_dedupe
         (workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
          reservation_owner, legacy_delivered_at, revision, reserved_at, delivered_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'reserved', NULL, ?6, NULL, 0, ?7, NULL, ?8)
         ON CONFLICT(workflow, db_name, user_id, article_id) DO NOTHING",
        params![
            workflow.as_str(),
            db_name,
            user_id,
            article_id,
            delivery_run_id,
            owner_id,
            now,
            now,
        ],
    )?;
    let record =
        load_delivery_dedupe_from_connection(&transaction, workflow, db_name, user_id, article_id)?
            .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    if inserted == 1 {
        Ok(DeliveryDedupeReserveOutcome::Reserved(record))
    } else {
        Ok(DeliveryDedupeReserveOutcome::Existing(record))
    }
}

/// Resolve a reservation as confirmed or externally unknown with revision CAS.
#[allow(clippy::too_many_arguments)]
pub fn resolve_delivery_dedupe(
    auth_db_path: impl AsRef<Path>,
    dedupe_id: i64,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    status: DeliveryDedupeStatus,
    message_id: Option<&str>,
    now: f64,
) -> Result<DeliveryDedupeRecord, DeliveryRepositoryError> {
    validate_positive_id(dedupe_id, "Delivery dedupe id is invalid")?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery dedupe owner id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    if status == DeliveryDedupeStatus::Reserved {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery dedupe terminal status is invalid",
        ));
    }
    validate_optional_text(message_id, 256, "Delivery message id is invalid")?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE delivery_dedupe
         SET status = ?1, message_id = ?2, reservation_owner = NULL, delivered_at = ?3,
             updated_at = ?4, revision = revision + 1
         WHERE id = ?5 AND delivery_run_id = ?6 AND reservation_owner = ?7
           AND revision = ?8 AND status = 'reserved'",
        params![
            status.as_str(),
            message_id,
            now,
            now,
            dedupe_id,
            delivery_run_id,
            owner_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_dedupe_by_id_from_connection(&transaction, dedupe_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// Release a pre-send reservation so a future run may reserve the identity.
pub fn release_delivery_dedupe_reservation(
    auth_db_path: impl AsRef<Path>,
    dedupe_id: i64,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
) -> Result<(), DeliveryRepositoryError> {
    validate_positive_id(dedupe_id, "Delivery dedupe id is invalid")?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery dedupe owner id is invalid")?;
    if expected_revision < 0 {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery dedupe revision is invalid",
        ));
    }
    let connection = open_delivery_connection(auth_db_path)?;
    let deleted = connection.execute(
        "DELETE FROM delivery_dedupe
         WHERE id = ?1 AND delivery_run_id = ?2 AND reservation_owner = ?3
           AND revision = ?4 AND status = 'reserved'",
        params![dedupe_id, delivery_run_id, owner_id, expected_revision],
    )?;
    if deleted != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    Ok(())
}

/// Atomically release multiple pre-send reservations owned by one run attempt.
pub fn release_delivery_dedupe_reservations(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    reservations: &[DeliveryDedupeResolution],
) -> Result<usize, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery dedupe owner id is invalid")?;
    validate_dedupe_resolutions(reservations)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for reservation in reservations {
        if transaction.execute(
            "DELETE FROM delivery_dedupe
             WHERE id = ?1 AND delivery_run_id = ?2 AND reservation_owner = ?3
               AND revision = ?4 AND status = 'reserved'",
            params![
                reservation.id,
                delivery_run_id,
                owner_id,
                reservation.expected_revision,
            ],
        )? != 1
        {
            return Err(DeliveryRepositoryError::Conflict);
        }
    }
    transaction.commit()?;
    Ok(reservations.len())
}

/// Atomically finalize one subscriber item and all of its dedupe reservations.
#[allow(clippy::too_many_arguments)]
pub fn finalize_delivery_attempt(
    auth_db_path: impl AsRef<Path>,
    item_id: i64,
    item_owner_id: &str,
    expected_item_revision: i64,
    item_status: DeliveryItemStatus,
    item_result_json: Option<&str>,
    item_error_code: Option<&str>,
    delivery_run_id: i64,
    reservations: &[DeliveryDedupeResolution],
    dedupe_status: DeliveryDedupeStatus,
    message_id: Option<&str>,
    now: f64,
) -> Result<DeliveryRunItemRecord, DeliveryRepositoryError> {
    validate_positive_id(item_id, "Delivery item id is invalid")?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(item_owner_id, "Delivery item owner id is invalid")?;
    validate_revision_and_time(expected_item_revision, now)?;
    if !item_status.is_terminal() || dedupe_status == DeliveryDedupeStatus::Reserved {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery attempt terminal status is invalid",
        ));
    }
    validate_optional_json(item_result_json)?;
    validate_optional_symbol(item_error_code, "Delivery item error code is invalid")?;
    validate_optional_text(message_id, 256, "Delivery message id is invalid")?;
    validate_dedupe_resolutions(reservations)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for reservation in reservations {
        if transaction.execute(
            "UPDATE delivery_dedupe
             SET status = ?1, message_id = ?2, reservation_owner = NULL,
                 delivered_at = ?3, updated_at = ?4, revision = revision + 1
             WHERE id = ?5 AND delivery_run_id = ?6 AND reservation_owner = ?7
               AND revision = ?8 AND status = 'reserved'",
            params![
                dedupe_status.as_str(),
                message_id,
                now,
                now,
                reservation.id,
                delivery_run_id,
                item_owner_id,
                reservation.expected_revision,
            ],
        )? != 1
        {
            return Err(DeliveryRepositoryError::Conflict);
        }
    }
    if transaction.execute(
        "UPDATE delivery_run_items
         SET status = ?1, owner_id = NULL, lease_expires_at = NULL, result_json = ?2,
             error_code = ?3, updated_at = ?4, finished_at = ?5, revision = revision + 1
         WHERE id = ?6 AND delivery_run_id = ?7 AND owner_id = ?8 AND revision = ?9
           AND status IN ('claimed', 'sending')",
        params![
            item_status.as_str(),
            item_result_json,
            item_error_code,
            now,
            now,
            item_id,
            delivery_run_id,
            item_owner_id,
            expected_item_revision,
        ],
    )? != 1
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let item = load_delivery_run_item_from_connection(&transaction, item_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(item)
}

/// Acquire a free workflow/database lease or take it over after expiration.
#[allow(clippy::too_many_arguments)]
pub fn acquire_delivery_lease(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    delivery_run_id: i64,
    owner_id: &str,
    now: f64,
    lease_seconds: f64,
) -> Result<DeliveryLeaseAcquireOutcome, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery lease owner id is invalid")?;
    validate_lease(now, lease_seconds)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if run.workflow != workflow || run.db_name.as_deref() != Some(db_name) {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery lease scope does not match its run",
        ));
    }
    let existing = load_delivery_lease_from_connection(&transaction, workflow, db_name)?;
    let record = match existing {
        None => {
            transaction.execute(
                "INSERT INTO delivery_leases
                 (workflow, db_name, delivery_run_id, owner_id, revision, acquired_at,
                  heartbeat_at, expires_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8)",
                params![
                    workflow.as_str(),
                    db_name,
                    delivery_run_id,
                    owner_id,
                    now,
                    now,
                    now + lease_seconds,
                    now,
                ],
            )?;
            load_delivery_lease_from_connection(&transaction, workflow, db_name)?
                .ok_or(DeliveryRepositoryError::NotFound)?
        }
        Some(existing)
            if existing.owner_id.is_none()
                || existing
                    .expires_at
                    .is_some_and(|expires_at| expires_at <= now) =>
        {
            let updated = transaction.execute(
                "UPDATE delivery_leases
                 SET delivery_run_id = ?1, owner_id = ?2, revision = revision + 1,
                     acquired_at = ?3, heartbeat_at = ?4, expires_at = ?5, updated_at = ?6
                 WHERE id = ?7 AND revision = ?8",
                params![
                    delivery_run_id,
                    owner_id,
                    now,
                    now,
                    now + lease_seconds,
                    now,
                    existing.id,
                    existing.revision,
                ],
            )?;
            if updated != 1 {
                return Err(DeliveryRepositoryError::Conflict);
            }
            load_delivery_lease_from_connection(&transaction, workflow, db_name)?
                .ok_or(DeliveryRepositoryError::NotFound)?
        }
        Some(existing) => {
            transaction.commit()?;
            return Ok(DeliveryLeaseAcquireOutcome::Busy(existing));
        }
    };
    transaction.commit()?;
    Ok(DeliveryLeaseAcquireOutcome::Acquired(record))
}

/// Renew a workflow/database lease with owner, run, and revision CAS.
#[allow(clippy::too_many_arguments)]
pub fn renew_delivery_lease(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
    lease_seconds: f64,
) -> Result<DeliveryLeaseRecord, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery lease owner id is invalid")?;
    validate_revision_and_lease(expected_revision, now, lease_seconds)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE delivery_leases
         SET heartbeat_at = ?1, expires_at = ?2, updated_at = ?3, revision = revision + 1
         WHERE workflow = ?4 AND db_name = ?5 AND delivery_run_id = ?6
           AND owner_id = ?7 AND revision = ?8 AND expires_at > ?1",
        params![
            now,
            now + lease_seconds,
            now,
            workflow.as_str(),
            db_name,
            delivery_run_id,
            owner_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_lease_from_connection(&transaction, workflow, db_name)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// Release a workflow/database lease without deleting its monotonic revision row.
pub fn release_delivery_lease(
    auth_db_path: impl AsRef<Path>,
    workflow: DeliveryWorkflow,
    db_name: &str,
    delivery_run_id: i64,
    owner_id: &str,
    expected_revision: i64,
    now: f64,
) -> Result<DeliveryLeaseRecord, DeliveryRepositoryError> {
    validate_db_name(db_name)?;
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery lease owner id is invalid")?;
    validate_revision_and_time(expected_revision, now)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = transaction.execute(
        "UPDATE delivery_leases
         SET delivery_run_id = NULL, owner_id = NULL, acquired_at = NULL,
             heartbeat_at = NULL, expires_at = NULL, updated_at = ?1, revision = revision + 1
         WHERE workflow = ?2 AND db_name = ?3 AND delivery_run_id = ?4
           AND owner_id = ?5 AND revision = ?6",
        params![
            now,
            workflow.as_str(),
            db_name,
            delivery_run_id,
            owner_id,
            expected_revision,
        ],
    )?;
    if updated != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_lease_from_connection(&transaction, workflow, db_name)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

/// Reconcile item and dedupe rows after an expired run owner is replaced.
///
/// Claimed items have not crossed the external side-effect boundary and return
/// to pending. Sending items and their reservations become terminal unknown so
/// a replacement owner cannot replay an ambiguous notification.
#[allow(clippy::too_many_arguments)]
pub fn reconcile_delivery_run_after_takeover(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    owner_id: &str,
    expected_run_revision: i64,
    now: f64,
) -> Result<DeliveryRecoveryResult, DeliveryRepositoryError> {
    validate_positive_id(delivery_run_id, "Delivery run id is invalid")?;
    validate_identifier(owner_id, "Delivery run owner id is invalid")?;
    validate_revision_and_time(expected_run_revision, now)?;
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if run.owner_id.as_deref() != Some(owner_id)
        || run.revision != expected_run_revision
        || !run.status.is_active()
        || !run
            .lease_expires_at
            .is_some_and(|expires_at| expires_at > now)
    {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let unknown_dedupe_count = transaction.execute(
        "UPDATE delivery_dedupe
         SET status = 'unknown', reservation_owner = NULL, delivered_at = ?1,
             updated_at = ?2, revision = revision + 1
         WHERE delivery_run_id = ?3 AND status = 'reserved'
           AND user_id IN (
               SELECT user_id FROM delivery_run_items
               WHERE delivery_run_id = ?3 AND item_kind = 'subscriber'
                 AND status = 'sending' AND user_id IS NOT NULL
           )",
        params![now, now, delivery_run_id],
    )?;
    let unknown_item_count = transaction.execute(
        "UPDATE delivery_run_items
         SET status = 'unknown', owner_id = NULL, lease_expires_at = NULL,
             error_code = 'abandoned_sending', updated_at = ?1, finished_at = ?2,
             revision = revision + 1
         WHERE delivery_run_id = ?3 AND status = 'sending'",
        params![now, now, delivery_run_id],
    )?;
    let released_dedupe_count = transaction.execute(
        "DELETE FROM delivery_dedupe
         WHERE delivery_run_id = ?1 AND status = 'reserved'",
        [delivery_run_id],
    )?;
    let reset_item_count = transaction.execute(
        "UPDATE delivery_run_items
         SET status = 'pending', owner_id = NULL, lease_expires_at = NULL,
             updated_at = ?1, revision = revision + 1
         WHERE delivery_run_id = ?2 AND status = 'claimed'",
        params![now, delivery_run_id],
    )?;
    transaction.commit()?;
    Ok(DeliveryRecoveryResult {
        reset_item_count,
        unknown_item_count,
        released_dedupe_count,
        unknown_dedupe_count,
    })
}

/// Import all legacy mutable state files in one all-or-nothing transaction.
///
/// # Arguments
///
/// * `config` - Storage configuration containing the authentication database and state roots.
/// * `now` - Current Unix timestamp used for imported rows.
///
/// # Returns
///
/// Aggregate discovered, imported, skipped, item, and dedupe counts.
pub fn import_legacy_delivery_state_files(
    config: &StorageConfig,
    now: f64,
) -> Result<LegacyDeliveryImportResult, DeliveryRepositoryError> {
    validate_time(now, "Legacy delivery import time is invalid")?;
    let started_at = std::time::Instant::now();
    let inputs = match collect_legacy_delivery_inputs(config.project_root()) {
        Ok(inputs) => inputs,
        Err(error) => {
            emit_legacy_import_failed(started_at, legacy_import_error_kind(&error));
            return Err(error);
        }
    };
    let discovered_count = inputs.len();
    let mut connection = open_delivery_connection(config.auth_db_path())?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut result = LegacyDeliveryImportResult {
        discovered_count,
        imported_count: 0,
        skipped_count: 0,
        item_count: 0,
        dedupe_count: 0,
    };
    for input in &inputs {
        match import_one_legacy_state(&transaction, input, now) {
            Ok(LegacyImportDisposition::Imported {
                item_count,
                dedupe_count,
            }) => {
                result.imported_count += 1;
                result.item_count += item_count;
                result.dedupe_count += dedupe_count;
            }
            Ok(LegacyImportDisposition::Skipped) => result.skipped_count += 1,
            Err(error) => {
                emit_legacy_import_failed(started_at, legacy_import_error_kind(&error));
                return Err(error);
            }
        }
    }
    if let Err(error) = transaction.commit() {
        let error = DeliveryRepositoryError::from(error);
        emit_legacy_import_failed(started_at, legacy_import_error_kind(&error));
        return Err(error);
    }
    tracing::info!(
        event = "delivery.legacy_import.completed",
        component = "delivery",
        outcome = "success",
        discovered_count = result.discovered_count,
        imported_count = result.imported_count,
        skipped_count = result.skipped_count,
        item_count = result.item_count,
        dedupe_count = result.dedupe_count,
        duration_ms = started_at.elapsed().as_millis() as u64,
    );
    Ok(result)
}

const CHECKPOINT_COLUMNS: &str =
    "id, workflow, db_name, status, legacy_status, snapshot_json, last_completed_run_at,
     revision, legacy_source_hash, legacy_source_name, legacy_imported_at, created_at, updated_at";
const RUN_COLUMNS: &str =
    "id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at";
const ITEM_COLUMNS: &str =
    "id, delivery_run_id, item_kind, item_key, user_id, article_id, status, legacy_status,
     owner_id, lease_expires_at, attempt_count, result_json, error_code, revision,
     created_at, started_at, updated_at, finished_at";
const DEDUPE_COLUMNS: &str =
    "id, workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
     reservation_owner, legacy_delivered_at, revision, reserved_at, delivered_at, updated_at";
const LEASE_COLUMNS: &str =
    "id, workflow, db_name, delivery_run_id, owner_id, revision, acquired_at,
     heartbeat_at, expires_at, updated_at";

fn open_delivery_connection(
    auth_db_path: impl AsRef<Path>,
) -> Result<Connection, DeliveryRepositoryError> {
    let auth_db_path = auth_db_path.as_ref();
    if let Some(parent) = auth_db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    Ok(open_sqlite_connection(auth_db_path)?)
}

fn load_delivery_checkpoint_from_connection(
    connection: &Connection,
    workflow: DeliveryWorkflow,
    db_name: &str,
) -> Result<Option<DeliveryCheckpointRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {CHECKPOINT_COLUMNS} FROM delivery_checkpoints
                 WHERE workflow = ?1 AND db_name = ?2"
            ),
            params![workflow.as_str(), db_name],
            checkpoint_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_run_from_connection(
    connection: &Connection,
    delivery_run_id: i64,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM delivery_runs WHERE id = ?1"),
            [delivery_run_id],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_run_by_external_id_from_connection(
    connection: &Connection,
    workflow: DeliveryWorkflow,
    scope_key: &str,
    external_id: &str,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM delivery_runs
                 WHERE workflow = ?1 AND scope_key = ?2 AND external_id = ?3"
            ),
            params![workflow.as_str(), scope_key, external_id],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_active_manual_delivery_run_from_connection(
    connection: &Connection,
    user_id: i64,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM delivery_runs
                 WHERE trigger_kind = 'manual' AND user_id = ?1
                   AND status IN ('queued', 'claimed', 'running', 'cancelling')
                 ORDER BY id LIMIT 1"
            ),
            [user_id],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_competing_active_run(
    connection: &Connection,
    delivery_run_id: i64,
    workflow: DeliveryWorkflow,
    db_name: &str,
) -> Result<Option<DeliveryRunRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM delivery_runs
                 WHERE id <> ?1 AND workflow = ?2 AND db_name = ?3
                   AND status IN ('claimed', 'running', 'cancelling')
                 ORDER BY id LIMIT 1"
            ),
            params![delivery_run_id, workflow.as_str(), db_name],
            run_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_run_item_from_connection(
    connection: &Connection,
    item_id: i64,
) -> Result<Option<DeliveryRunItemRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM delivery_run_items WHERE id = ?1"),
            [item_id],
            item_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_run_item_by_identity_from_connection(
    connection: &Connection,
    delivery_run_id: i64,
    item_kind: DeliveryItemKind,
    item_key: &str,
) -> Result<Option<DeliveryRunItemRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {ITEM_COLUMNS} FROM delivery_run_items
                 WHERE delivery_run_id = ?1 AND item_kind = ?2 AND item_key = ?3"
            ),
            params![delivery_run_id, item_kind.as_str(), item_key],
            item_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_dedupe_from_connection(
    connection: &Connection,
    workflow: DeliveryWorkflow,
    db_name: &str,
    user_id: i64,
    article_id: i64,
) -> Result<Option<DeliveryDedupeRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {DEDUPE_COLUMNS} FROM delivery_dedupe
                 WHERE workflow = ?1 AND db_name = ?2 AND user_id = ?3 AND article_id = ?4"
            ),
            params![workflow.as_str(), db_name, user_id, article_id],
            dedupe_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_dedupe_by_id_from_connection(
    connection: &Connection,
    dedupe_id: i64,
) -> Result<Option<DeliveryDedupeRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!("SELECT {DEDUPE_COLUMNS} FROM delivery_dedupe WHERE id = ?1"),
            [dedupe_id],
            dedupe_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn load_delivery_lease_from_connection(
    connection: &Connection,
    workflow: DeliveryWorkflow,
    db_name: &str,
) -> Result<Option<DeliveryLeaseRecord>, DeliveryRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {LEASE_COLUMNS} FROM delivery_leases
                 WHERE workflow = ?1 AND db_name = ?2"
            ),
            params![workflow.as_str(), db_name],
            lease_from_row,
        )
        .optional()
        .map_err(DeliveryRepositoryError::from)
}

fn checkpoint_from_row(row: &Row<'_>) -> rusqlite::Result<DeliveryCheckpointRecord> {
    Ok(DeliveryCheckpointRecord {
        id: row.get(0)?,
        workflow: DeliveryWorkflow::parse(row.get_ref(1)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        db_name: row.get(2)?,
        status: DeliveryCheckpointStatus::parse(row.get_ref(3)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        legacy_status: row.get(4)?,
        snapshot_json: row.get(5)?,
        last_completed_run_at: row.get(6)?,
        revision: row.get(7)?,
        legacy_source_hash: row.get(8)?,
        legacy_source_name: row.get(9)?,
        legacy_imported_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn run_from_row(row: &Row<'_>) -> rusqlite::Result<DeliveryRunRecord> {
    Ok(DeliveryRunRecord {
        id: row.get(0)?,
        external_id: row.get(1)?,
        workflow: DeliveryWorkflow::parse(row.get_ref(2)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        scope_key: row.get(3)?,
        db_name: row.get(4)?,
        trigger_kind: DeliveryTriggerKind::parse(row.get_ref(5)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        mode: DeliveryRunMode::parse(row.get_ref(6)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        user_id: row.get(7)?,
        status: DeliveryRunStatus::parse(row.get_ref(8)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        legacy_status: row.get(9)?,
        owner_id: row.get(10)?,
        lease_expires_at: row.get(11)?,
        deadline_at: row.get(12)?,
        cancellation_requested: row.get::<_, i64>(13)? != 0,
        result_json: row.get(14)?,
        error_code: row.get(15)?,
        revision: row.get(16)?,
        created_at: row.get(17)?,
        started_at: row.get(18)?,
        updated_at: row.get(19)?,
        finished_at: row.get(20)?,
    })
}

fn item_from_row(row: &Row<'_>) -> rusqlite::Result<DeliveryRunItemRecord> {
    Ok(DeliveryRunItemRecord {
        id: row.get(0)?,
        delivery_run_id: row.get(1)?,
        item_kind: DeliveryItemKind::parse(row.get_ref(2)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        item_key: row.get(3)?,
        user_id: row.get(4)?,
        article_id: row.get(5)?,
        status: DeliveryItemStatus::parse(row.get_ref(6)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        legacy_status: row.get(7)?,
        owner_id: row.get(8)?,
        lease_expires_at: row.get(9)?,
        attempt_count: row.get(10)?,
        result_json: row.get(11)?,
        error_code: row.get(12)?,
        revision: row.get(13)?,
        created_at: row.get(14)?,
        started_at: row.get(15)?,
        updated_at: row.get(16)?,
        finished_at: row.get(17)?,
    })
}

fn dedupe_from_row(row: &Row<'_>) -> rusqlite::Result<DeliveryDedupeRecord> {
    Ok(DeliveryDedupeRecord {
        id: row.get(0)?,
        workflow: DeliveryWorkflow::parse(row.get_ref(1)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        db_name: row.get(2)?,
        user_id: row.get(3)?,
        article_id: row.get(4)?,
        delivery_run_id: row.get(5)?,
        status: DeliveryDedupeStatus::parse(row.get_ref(6)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        message_id: row.get(7)?,
        reservation_owner: row.get(8)?,
        legacy_delivered_at: row.get(9)?,
        revision: row.get(10)?,
        reserved_at: row.get(11)?,
        delivered_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn lease_from_row(row: &Row<'_>) -> rusqlite::Result<DeliveryLeaseRecord> {
    Ok(DeliveryLeaseRecord {
        id: row.get(0)?,
        workflow: DeliveryWorkflow::parse(row.get_ref(1)?.as_str()?)
            .map_err(invalid_stored_sqlite_error)?,
        db_name: row.get(2)?,
        delivery_run_id: row.get(3)?,
        owner_id: row.get(4)?,
        revision: row.get(5)?,
        acquired_at: row.get(6)?,
        heartbeat_at: row.get(7)?,
        expires_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn invalid_stored_sqlite_error(_: DeliveryRepositoryError) -> rusqlite::Error {
    rusqlite::Error::InvalidQuery
}

fn update_run_with_owner_cas<P>(
    auth_db_path: impl AsRef<Path>,
    delivery_run_id: i64,
    _owner_id: &str,
    _expected_revision: i64,
    sql: &str,
    parameters: P,
) -> Result<DeliveryRunRecord, DeliveryRepositoryError>
where
    P: rusqlite::Params,
{
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if transaction.execute(sql, parameters)? != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_run_from_connection(&transaction, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

fn update_item_with_owner_cas<P>(
    auth_db_path: impl AsRef<Path>,
    item_id: i64,
    _owner_id: &str,
    _expected_revision: i64,
    sql: &str,
    parameters: P,
) -> Result<DeliveryRunItemRecord, DeliveryRepositoryError>
where
    P: rusqlite::Params,
{
    let mut connection = open_delivery_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if transaction.execute(sql, parameters)? != 1 {
        return Err(DeliveryRepositoryError::Conflict);
    }
    let record = load_delivery_run_item_from_connection(&transaction, item_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    transaction.commit()?;
    Ok(record)
}

#[derive(Debug)]
struct LegacyDeliveryInput {
    workflow: DeliveryWorkflow,
    source_name: String,
    source_hash: String,
    state: LegacyRecommendationState,
}

#[derive(Debug, Deserialize)]
struct LegacyRecommendationState {
    db_name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    last_completed_run_at: Option<String>,
    #[serde(default)]
    snapshot: LegacyRecommendationSnapshot,
    #[serde(default)]
    run: Option<LegacyRecommendationRun>,
    #[serde(default)]
    delivery_dedupe: BTreeMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LegacyRecommendationSnapshot {
    #[serde(default)]
    issue_article_counts: BTreeMap<String, i64>,
    #[serde(default)]
    inpress_article_counts: BTreeMap<String, i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyRecommendationRun {
    run_id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    pending_issue_keys: Vec<String>,
    #[serde(default)]
    done_issue_keys: Vec<String>,
    #[serde(default)]
    pending_inpress_keys: Vec<String>,
    #[serde(default)]
    done_inpress_keys: Vec<String>,
    #[serde(default)]
    delivered_article_ids: Vec<i64>,
    #[serde(default)]
    user_results: Vec<LegacyRecommendationUserResult>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyRecommendationUserResult {
    subscriber_id: String,
    #[serde(default)]
    selected_count: usize,
    #[serde(default)]
    pushed_count: usize,
    #[serde(default)]
    folder_synced_count: Option<usize>,
    #[serde(default)]
    status: String,
}

enum LegacyImportDisposition {
    Imported {
        item_count: usize,
        dedupe_count: usize,
    },
    Skipped,
}

fn collect_legacy_delivery_inputs(
    project_root: &Path,
) -> Result<Vec<LegacyDeliveryInput>, DeliveryRepositoryError> {
    let mut inputs = Vec::new();
    for (directory_name, workflow) in [
        ("push_state", DeliveryWorkflow::Notify),
        ("folder_push_state", DeliveryWorkflow::Push),
    ] {
        let directory = project_root.join("data").join(directory_name);
        if !directory.exists() {
            continue;
        }
        let directory_metadata = fs::symlink_metadata(&directory)?;
        if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
            return Err(DeliveryRepositoryError::InvalidLegacyState);
        }
        let mut paths = fs::read_dir(&directory)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        for path in paths {
            let Some(source_name) = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
            else {
                return Err(DeliveryRepositoryError::InvalidLegacyState);
            };
            if !source_name.ends_with(".json") || source_name.ends_with(".changes.json") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(DeliveryRepositoryError::InvalidLegacyState);
            }
            if metadata.len() > MAX_LEGACY_STATE_BYTES {
                return Err(DeliveryRepositoryError::LegacyStateTooLarge);
            }
            let bytes = fs::read(&path)?;
            let state: LegacyRecommendationState = serde_json::from_slice(&bytes)?;
            validate_legacy_state(&source_name, &state)?;
            inputs.push(LegacyDeliveryInput {
                workflow,
                source_name,
                source_hash: hex::encode(Sha256::digest(&bytes)),
                state,
            });
        }
    }
    inputs.sort_by(|left, right| {
        left.workflow
            .as_str()
            .cmp(right.workflow.as_str())
            .then_with(|| left.source_name.cmp(&right.source_name))
    });
    Ok(inputs)
}

fn validate_legacy_state(
    source_name: &str,
    state: &LegacyRecommendationState,
) -> Result<(), DeliveryRepositoryError> {
    validate_db_name(&state.db_name)?;
    let expected_name = format!(
        "{}.json",
        state
            .db_name
            .strip_suffix(".sqlite")
            .ok_or(DeliveryRepositoryError::InvalidLegacyState)?
    );
    if source_name != expected_name {
        return Err(DeliveryRepositoryError::InvalidLegacyState);
    }
    validate_optional_text(
        state.last_completed_run_at.as_deref(),
        128,
        "Legacy completion timestamp is invalid",
    )?;
    for (key, count) in &state.snapshot.issue_article_counts {
        validate_issue_key(key)?;
        if *count < 0 {
            return Err(DeliveryRepositoryError::InvalidLegacyState);
        }
    }
    for (key, count) in &state.snapshot.inpress_article_counts {
        validate_positive_numeric_key(key)?;
        if *count < 0 {
            return Err(DeliveryRepositoryError::InvalidLegacyState);
        }
    }
    if let Some(run) = &state.run {
        validate_identifier(&run.run_id, "Legacy delivery run id is invalid")?;
        validate_legacy_progress_keys(&run.pending_issue_keys, DeliveryItemKind::Issue)?;
        validate_legacy_progress_keys(&run.done_issue_keys, DeliveryItemKind::Issue)?;
        validate_legacy_progress_keys(&run.pending_inpress_keys, DeliveryItemKind::InPress)?;
        validate_legacy_progress_keys(&run.done_inpress_keys, DeliveryItemKind::InPress)?;
        if run
            .delivered_article_ids
            .iter()
            .any(|article_id| *article_id <= 0)
        {
            return Err(DeliveryRepositoryError::InvalidLegacyState);
        }
        for result in &run.user_results {
            validate_positive_numeric_key(&result.subscriber_id)?;
        }
    }
    for (key, delivered_at) in &state.delivery_dedupe {
        parse_legacy_dedupe_key(key)?;
        validate_text(delivered_at, 128, "Legacy dedupe timestamp is invalid")?;
    }
    Ok(())
}

fn validate_legacy_progress_keys(
    keys: &[String],
    item_kind: DeliveryItemKind,
) -> Result<(), DeliveryRepositoryError> {
    let mut seen = HashSet::new();
    for key in keys {
        match item_kind {
            DeliveryItemKind::Issue => validate_issue_key(key)?,
            DeliveryItemKind::InPress => validate_positive_numeric_key(key)?,
            DeliveryItemKind::Article | DeliveryItemKind::Subscriber => {
                return Err(DeliveryRepositoryError::InvalidLegacyState);
            }
        }
        if !seen.insert(key) {
            return Err(DeliveryRepositoryError::InvalidLegacyState);
        }
    }
    Ok(())
}

fn import_one_legacy_state(
    transaction: &Transaction<'_>,
    input: &LegacyDeliveryInput,
    now: f64,
) -> Result<LegacyImportDisposition, DeliveryRepositoryError> {
    if let Some(existing) =
        load_delivery_checkpoint_from_connection(transaction, input.workflow, &input.state.db_name)?
    {
        if existing.legacy_source_hash.as_deref() == Some(input.source_hash.as_str()) {
            return Ok(LegacyImportDisposition::Skipped);
        }
        return Err(DeliveryRepositoryError::LegacyImportConflict);
    }
    let (checkpoint_status, checkpoint_legacy_status) =
        legacy_checkpoint_status(&input.state.status);
    let snapshot_json = serde_json::to_string(&input.state.snapshot)?;
    transaction.execute(
        "INSERT INTO delivery_checkpoints
         (workflow, db_name, status, legacy_status, snapshot_json, last_completed_run_at,
          revision, legacy_source_hash, legacy_source_name, legacy_imported_at,
          created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10, ?11)",
        params![
            input.workflow.as_str(),
            input.state.db_name,
            checkpoint_status.as_str(),
            checkpoint_legacy_status,
            snapshot_json,
            input.state.last_completed_run_at,
            input.source_hash,
            input.source_name,
            now,
            now,
            now,
        ],
    )?;
    let mut item_count = 0;
    let delivery_run_id = if let Some(run) = &input.state.run {
        let (run_status, legacy_status) = legacy_run_status(&run.status);
        let result_json = legacy_run_summary_json(run)?;
        transaction.execute(
            "INSERT INTO delivery_runs
             (external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id,
              status, legacy_status, owner_id, lease_expires_at, deadline_at,
              cancellation_requested, result_json, error_code, revision, created_at,
              started_at, updated_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, 'legacy', 'execute', NULL, ?5, ?6,
                     NULL, NULL, NULL, 0, ?7, NULL, 0, ?8, NULL, ?9, ?10)",
            params![
                run.run_id,
                input.workflow.as_str(),
                input.state.db_name,
                input.state.db_name,
                run_status.as_str(),
                legacy_status,
                result_json,
                now,
                now,
                run_status.is_terminal().then_some(now),
            ],
        )?;
        let delivery_run_id = transaction.last_insert_rowid();
        let mut item_identities = HashSet::new();
        for (kind, status, keys) in [
            (
                DeliveryItemKind::Issue,
                DeliveryItemStatus::Pending,
                run.pending_issue_keys.as_slice(),
            ),
            (
                DeliveryItemKind::Issue,
                DeliveryItemStatus::Succeeded,
                run.done_issue_keys.as_slice(),
            ),
            (
                DeliveryItemKind::InPress,
                DeliveryItemStatus::Pending,
                run.pending_inpress_keys.as_slice(),
            ),
            (
                DeliveryItemKind::InPress,
                DeliveryItemStatus::Succeeded,
                run.done_inpress_keys.as_slice(),
            ),
        ] {
            for key in keys {
                if !item_identities.insert((kind, key.as_str())) {
                    return Err(DeliveryRepositoryError::InvalidLegacyState);
                }
                insert_legacy_item(
                    transaction,
                    delivery_run_id,
                    kind,
                    key,
                    None,
                    None,
                    status,
                    None,
                    now,
                )?;
                item_count += 1;
            }
        }
        for result in &run.user_results {
            let user_id = result
                .subscriber_id
                .parse::<i64>()
                .map_err(|_| DeliveryRepositoryError::InvalidLegacyState)?;
            let (status, legacy_status) = legacy_item_status(&result.status);
            let result_json = serde_json::to_string(&serde_json::json!({
                "selected_count": result.selected_count,
                "pushed_count": result.pushed_count,
                "folder_synced_count": result.folder_synced_count,
            }))?;
            insert_legacy_item(
                transaction,
                delivery_run_id,
                DeliveryItemKind::Subscriber,
                &result.subscriber_id,
                Some(user_id),
                None,
                status,
                legacy_status,
                now,
            )?;
            transaction.execute(
                "UPDATE delivery_run_items SET result_json = ?1
                 WHERE delivery_run_id = ?2 AND item_kind = 'subscriber' AND item_key = ?3",
                params![result_json, delivery_run_id, result.subscriber_id],
            )?;
            item_count += 1;
        }
        Some(delivery_run_id)
    } else {
        None
    };
    let mut dedupe_count = 0;
    for (key, delivered_at) in &input.state.delivery_dedupe {
        let (user_id, article_id) = parse_legacy_dedupe_key(key)?;
        transaction.execute(
            "INSERT INTO delivery_dedupe
             (workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
              reservation_owner, legacy_delivered_at, revision, reserved_at,
              delivered_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'confirmed', NULL, NULL, ?6, 0, ?7, ?8, ?9)",
            params![
                input.workflow.as_str(),
                input.state.db_name,
                user_id,
                article_id,
                delivery_run_id,
                delivered_at,
                now,
                now,
                now,
            ],
        )?;
        dedupe_count += 1;
    }
    Ok(LegacyImportDisposition::Imported {
        item_count,
        dedupe_count,
    })
}

#[allow(clippy::too_many_arguments)]
fn insert_legacy_item(
    transaction: &Transaction<'_>,
    delivery_run_id: i64,
    item_kind: DeliveryItemKind,
    item_key: &str,
    user_id: Option<i64>,
    article_id: Option<i64>,
    status: DeliveryItemStatus,
    legacy_status: Option<&'static str>,
    now: f64,
) -> Result<(), DeliveryRepositoryError> {
    transaction.execute(
        "INSERT INTO delivery_run_items
         (delivery_run_id, item_kind, item_key, user_id, article_id, status,
          legacy_status, owner_id, lease_expires_at, attempt_count, result_json,
          error_code, revision, created_at, started_at, updated_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, 0, NULL, NULL, 0,
                 ?8, NULL, ?9, ?10)",
        params![
            delivery_run_id,
            item_kind.as_str(),
            item_key,
            user_id,
            article_id,
            status.as_str(),
            legacy_status,
            now,
            now,
            status.is_terminal().then_some(now),
        ],
    )?;
    Ok(())
}

fn legacy_run_summary_json(
    run: &LegacyRecommendationRun,
) -> Result<String, DeliveryRepositoryError> {
    let value = serde_json::json!({
        "pending_issue_keys": run.pending_issue_keys,
        "done_issue_keys": run.done_issue_keys,
        "pending_inpress_keys": run.pending_inpress_keys,
        "done_inpress_keys": run.done_inpress_keys,
        "delivered_article_ids": run.delivered_article_ids,
        "subscriber_count": run.user_results.len(),
    });
    let encoded = serde_json::to_string(&value)?;
    validate_json(&encoded)?;
    Ok(encoded)
}

fn legacy_checkpoint_status(value: &str) -> (DeliveryCheckpointStatus, Option<&'static str>) {
    match value.trim() {
        "" | "idle" => (DeliveryCheckpointStatus::Idle, None),
        "running" => (DeliveryCheckpointStatus::Unknown, Some("abandoned_active")),
        "completed" => (DeliveryCheckpointStatus::Completed, None),
        "failed" => (DeliveryCheckpointStatus::Failed, None),
        "skipped" => (DeliveryCheckpointStatus::Skipped, None),
        "unknown" => (DeliveryCheckpointStatus::Unknown, None),
        _ => (DeliveryCheckpointStatus::Unknown, Some("unrecognized")),
    }
}

fn legacy_run_status(value: &str) -> (DeliveryRunStatus, Option<&'static str>) {
    match value.trim() {
        "completed" => (DeliveryRunStatus::Completed, None),
        "failed" => (DeliveryRunStatus::Failed, None),
        "cancelled" => (DeliveryRunStatus::Cancelled, None),
        "timed_out" => (DeliveryRunStatus::TimedOut, None),
        "skipped" => (DeliveryRunStatus::Skipped, None),
        "unknown" => (DeliveryRunStatus::Unknown, None),
        "running" | "claimed" | "cancelling" => {
            (DeliveryRunStatus::Unknown, Some("abandoned_active"))
        }
        _ => (DeliveryRunStatus::Unknown, Some("unrecognized")),
    }
}

fn legacy_item_status(value: &str) -> (DeliveryItemStatus, Option<&'static str>) {
    match value.trim() {
        "ok" | "completed" | "succeeded" => (DeliveryItemStatus::Succeeded, None),
        "error" | "failed" => (DeliveryItemStatus::Failed, None),
        "skipped" => (DeliveryItemStatus::Skipped, None),
        "cancelled" => (DeliveryItemStatus::Cancelled, None),
        "unknown" => (DeliveryItemStatus::Unknown, None),
        _ => (DeliveryItemStatus::Unknown, Some("unrecognized")),
    }
}

fn parse_legacy_dedupe_key(key: &str) -> Result<(i64, i64), DeliveryRepositoryError> {
    let (user_id, article_id) = key
        .split_once(':')
        .ok_or(DeliveryRepositoryError::InvalidLegacyState)?;
    let user_id = user_id
        .parse::<i64>()
        .map_err(|_| DeliveryRepositoryError::InvalidLegacyState)?;
    let article_id = article_id
        .parse::<i64>()
        .map_err(|_| DeliveryRepositoryError::InvalidLegacyState)?;
    if user_id <= 0 || article_id <= 0 {
        return Err(DeliveryRepositoryError::InvalidLegacyState);
    }
    Ok((user_id, article_id))
}

fn validate_run_create(run: &DeliveryRunCreate) -> Result<(), DeliveryRepositoryError> {
    validate_identifier(&run.external_id, "Delivery external run id is invalid")?;
    validate_text(
        &run.scope_key,
        MAX_DB_NAME_BYTES,
        "Delivery run scope is invalid",
    )?;
    if let Some(db_name) = run.db_name.as_deref() {
        validate_db_name(db_name)?;
    }
    if run.trigger_kind != DeliveryTriggerKind::Manual && run.db_name.is_none() {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Non-manual delivery runs require a database",
        ));
    }
    if run.trigger_kind == DeliveryTriggerKind::Manual && run.user_id.is_none() {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Manual delivery runs require a user",
        ));
    }
    if let Some(user_id) = run.user_id {
        validate_positive_id(user_id, "Delivery run user id is invalid")?;
    }
    validate_time(run.created_at, "Delivery run creation time is invalid")?;
    if let Some(deadline_at) = run.deadline_at {
        validate_time(deadline_at, "Delivery run deadline is invalid")?;
        if deadline_at <= run.created_at {
            return Err(DeliveryRepositoryError::InvalidInput(
                "Delivery run deadline must follow creation",
            ));
        }
    }
    Ok(())
}

fn validate_item_create(item: &DeliveryRunItemCreate) -> Result<(), DeliveryRepositoryError> {
    validate_text(
        &item.item_key,
        MAX_ITEM_KEY_BYTES,
        "Delivery item key is invalid",
    )?;
    if let Some(user_id) = item.user_id {
        validate_positive_id(user_id, "Delivery item user id is invalid")?;
    }
    if let Some(article_id) = item.article_id {
        validate_positive_id(article_id, "Delivery item article id is invalid")?;
    }
    Ok(())
}

fn validate_dedupe_resolutions(
    reservations: &[DeliveryDedupeResolution],
) -> Result<(), DeliveryRepositoryError> {
    let mut ids = HashSet::new();
    for reservation in reservations {
        validate_positive_id(reservation.id, "Delivery dedupe id is invalid")?;
        if reservation.expected_revision < 0 || !ids.insert(reservation.id) {
            return Err(DeliveryRepositoryError::InvalidInput(
                "Delivery dedupe resolutions are invalid",
            ));
        }
    }
    Ok(())
}

fn validate_db_name(db_name: &str) -> Result<(), DeliveryRepositoryError> {
    if db_name.is_empty()
        || db_name.len() > MAX_DB_NAME_BYTES
        || !db_name.ends_with(".sqlite")
        || Path::new(db_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(db_name)
        || db_name.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery database name is invalid",
        ));
    }
    Ok(())
}

fn validate_identifier(value: &str, detail: &'static str) -> Result<(), DeliveryRepositoryError> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !value.is_ascii()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'+')
        })
    {
        return Err(DeliveryRepositoryError::InvalidInput(detail));
    }
    Ok(())
}

fn validate_text(
    value: &str,
    max_bytes: usize,
    detail: &'static str,
) -> Result<(), DeliveryRepositoryError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DeliveryRepositoryError::InvalidInput(detail));
    }
    Ok(())
}

fn validate_optional_text(
    value: Option<&str>,
    max_bytes: usize,
    detail: &'static str,
) -> Result<(), DeliveryRepositoryError> {
    if let Some(value) = value {
        validate_text(value, max_bytes, detail)?;
    }
    Ok(())
}

fn validate_optional_symbol(
    value: Option<&str>,
    detail: &'static str,
) -> Result<(), DeliveryRepositoryError> {
    if let Some(value) = value {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(DeliveryRepositoryError::InvalidInput(detail));
        }
    }
    Ok(())
}

fn validate_json(value: &str) -> Result<(), DeliveryRepositoryError> {
    if value.len() > MAX_RESULT_JSON_BYTES {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery JSON exceeds its size limit",
        ));
    }
    serde_json::from_str::<serde_json::Value>(value)?;
    Ok(())
}

fn validate_optional_json(value: Option<&str>) -> Result<(), DeliveryRepositoryError> {
    if let Some(value) = value {
        validate_json(value)?;
    }
    Ok(())
}

fn validate_positive_id(value: i64, detail: &'static str) -> Result<(), DeliveryRepositoryError> {
    if value <= 0 {
        return Err(DeliveryRepositoryError::InvalidInput(detail));
    }
    Ok(())
}

fn validate_time(value: f64, detail: &'static str) -> Result<(), DeliveryRepositoryError> {
    if !value.is_finite() || value < 0.0 {
        return Err(DeliveryRepositoryError::InvalidInput(detail));
    }
    Ok(())
}

fn validate_revision_and_time(revision: i64, now: f64) -> Result<(), DeliveryRepositoryError> {
    if revision < 0 {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery revision is invalid",
        ));
    }
    validate_time(now, "Delivery update time is invalid")
}

fn validate_lease(now: f64, lease_seconds: f64) -> Result<(), DeliveryRepositoryError> {
    validate_time(now, "Delivery lease time is invalid")?;
    if !lease_seconds.is_finite() || lease_seconds <= 0.0 || lease_seconds > 86_400.0 {
        return Err(DeliveryRepositoryError::InvalidInput(
            "Delivery lease duration is invalid",
        ));
    }
    Ok(())
}

fn validate_revision_and_lease(
    revision: i64,
    now: f64,
    lease_seconds: f64,
) -> Result<(), DeliveryRepositoryError> {
    validate_revision_and_time(revision, now)?;
    validate_lease(now, lease_seconds)
}

fn validate_issue_key(value: &str) -> Result<(), DeliveryRepositoryError> {
    let (journal_id, issue_id) = value
        .split_once(':')
        .ok_or(DeliveryRepositoryError::InvalidLegacyState)?;
    validate_positive_numeric_key(journal_id)?;
    validate_positive_numeric_key(issue_id)
}

fn validate_positive_numeric_key(value: &str) -> Result<(), DeliveryRepositoryError> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| DeliveryRepositoryError::InvalidLegacyState)?;
    if parsed <= 0 {
        return Err(DeliveryRepositoryError::InvalidLegacyState);
    }
    Ok(())
}

fn emit_legacy_import_failed(started_at: std::time::Instant, error_kind: &'static str) {
    tracing::error!(
        event = "delivery.legacy_import.failed",
        component = "delivery",
        outcome = "failure",
        error_kind,
        duration_ms = started_at.elapsed().as_millis() as u64,
    );
}

fn legacy_import_error_kind(error: &DeliveryRepositoryError) -> &'static str {
    match error {
        DeliveryRepositoryError::Sqlite(_) => "sqlite",
        DeliveryRepositoryError::Io(_) => "io",
        DeliveryRepositoryError::Json(_) => "invalid_json",
        DeliveryRepositoryError::InvalidInput(_) => "invalid_input",
        DeliveryRepositoryError::InvalidStoredState => "invalid_stored_state",
        DeliveryRepositoryError::NotFound => "not_found",
        DeliveryRepositoryError::Conflict => "conflict",
        DeliveryRepositoryError::InvalidLegacyState => "invalid_legacy_state",
        DeliveryRepositoryError::LegacyImportConflict => "legacy_import_conflict",
        DeliveryRepositoryError::LegacyStateTooLarge => "legacy_state_too_large",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use rusqlite::Connection;
    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::migrate_auth_database;

    fn migrated_auth_database() -> (TempDir, PathBuf) {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let path = temp_dir.path().join("data/auth.sqlite");
        migrate_auth_database(&path).expect("auth database should migrate");
        (temp_dir, path)
    }

    fn scheduled_run(external_id: &str, db_name: &str, created_at: f64) -> DeliveryRunCreate {
        DeliveryRunCreate {
            external_id: external_id.to_string(),
            workflow: DeliveryWorkflow::Notify,
            scope_key: db_name.to_string(),
            db_name: Some(db_name.to_string()),
            trigger_kind: DeliveryTriggerKind::Scheduled,
            mode: DeliveryRunMode::Execute,
            user_id: None,
            deadline_at: Some(created_at + 1_000.0),
            created_at,
        }
    }

    fn manual_run(external_id: &str, user_id: i64, created_at: f64) -> DeliveryRunCreate {
        DeliveryRunCreate {
            external_id: external_id.to_string(),
            workflow: DeliveryWorkflow::Push,
            scope_key: format!("manual-user-{user_id}"),
            db_name: None,
            trigger_kind: DeliveryTriggerKind::Manual,
            mode: DeliveryRunMode::Execute,
            user_id: Some(user_id),
            deadline_at: Some(created_at + 600.0),
            created_at,
        }
    }

    fn expect_claimed(outcome: DeliveryRunClaimOutcome) -> DeliveryRunRecord {
        match outcome {
            DeliveryRunClaimOutcome::Claimed(record) => record,
            other => panic!("run should be claimed, got {other:?}"),
        }
    }

    fn row_count(path: &Path, table: &str) -> i64 {
        Connection::open(path)
            .expect("database should open")
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("row count should be readable")
    }

    #[test]
    fn checkpoint_compare_and_swap_allows_one_concurrent_writer() {
        let (_temp_dir, path) = migrated_auth_database();
        let initial = compare_and_swap_delivery_checkpoint(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            None,
            &DeliveryCheckpointUpdate {
                status: DeliveryCheckpointStatus::Idle,
                snapshot_json: "{}".to_string(),
                last_completed_run_at: None,
                updated_at: 1.0,
            },
        )
        .expect("initial checkpoint should insert");
        assert_eq!(initial.revision, 0);

        let barrier = Arc::new(Barrier::new(3));
        let handles = ["first", "second"].map(|name| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                compare_and_swap_delivery_checkpoint(
                    path,
                    DeliveryWorkflow::Notify,
                    "main.sqlite",
                    Some(0),
                    &DeliveryCheckpointUpdate {
                        status: DeliveryCheckpointStatus::Completed,
                        snapshot_json: format!(r#"{{"writer":"{name}"}}"#),
                        last_completed_run_at: Some("2026-07-27T00:00:00Z".to_string()),
                        updated_at: 2.0,
                    },
                )
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("writer should finish"));

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(DeliveryRepositoryError::Conflict)))
                .count(),
            1
        );
        let stored = load_delivery_checkpoint(&path, DeliveryWorkflow::Notify, "main.sqlite")
            .expect("checkpoint should load")
            .expect("checkpoint should exist");
        assert_eq!(stored.revision, 1);
        assert_eq!(stored.status, DeliveryCheckpointStatus::Completed);
    }

    #[test]
    fn manual_run_admission_is_per_user_and_persists_latest_status() {
        let (_temp_dir, path) = migrated_auth_database();
        let barrier = Arc::new(Barrier::new(3));
        let handles =
            [("manual-one", 11_i64), ("manual-two", 12_i64)].map(|(external_id, user_id)| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    admit_delivery_run(path, &manual_run(external_id, user_id, 10.0))
                })
            });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("admission thread should finish"));

        assert!(results
            .iter()
            .all(|result| matches!(result, Ok(DeliveryRunAdmissionOutcome::Enqueued(_)))));
        let duplicate = admit_delivery_run(&path, &manual_run("manual-three", 11, 11.0))
            .expect("same-user admission should return the active run");
        let DeliveryRunAdmissionOutcome::Busy(existing) = duplicate else {
            panic!("same-user admission should be busy")
        };
        assert_eq!(existing.external_id, "manual-one");
        assert_eq!(
            load_latest_manual_delivery_run(&path, 11)
                .expect("latest run should load")
                .expect("latest run should exist")
                .external_id,
            "manual-one"
        );
        assert_eq!(
            load_manual_delivery_run_by_external_id(&path, 11, "manual-one")
                .expect("manual run should load")
                .expect("manual run should exist")
                .id,
            existing.id
        );
        assert!(
            load_manual_delivery_run_by_external_id(&path, 12, "manual-one")
                .expect("cross-user lookup should complete")
                .is_none()
        );
        assert_eq!(
            list_dispatchable_manual_delivery_runs(&path, 12.0, 8)
                .expect("dispatchable runs should list")
                .len(),
            2
        );
    }

    #[test]
    fn queued_manual_run_finalization_is_revision_fenced() {
        let (_temp_dir, path) = migrated_auth_database();
        let run = enqueue_delivery_run(&path, &manual_run("manual-failed", 21, 10.0))
            .expect("manual run should enqueue");
        let finalized = finalize_queued_delivery_run(
            &path,
            run.id,
            run.revision,
            DeliveryRunStatus::Failed,
            Some(r#"{"pushed":0}"#),
            Some("dispatch_spawn_failed"),
            11.0,
        )
        .expect("queued run should finalize");

        assert_eq!(finalized.status, DeliveryRunStatus::Failed);
        assert_eq!(
            finalized.error_code.as_deref(),
            Some("dispatch_spawn_failed")
        );
        assert!(finalized.finished_at.is_some());
        assert!(matches!(
            finalize_queued_delivery_run(
                &path,
                run.id,
                run.revision,
                DeliveryRunStatus::Failed,
                None,
                Some("dispatch_spawn_failed"),
                12.0,
            ),
            Err(DeliveryRepositoryError::Conflict)
        ));
        assert!(list_dispatchable_manual_delivery_runs(&path, 12.0, 8)
            .expect("terminal run should not dispatch")
            .is_empty());
    }

    #[test]
    fn run_claims_enforce_revision_owner_and_active_scope() {
        let (_temp_dir, path) = migrated_auth_database();
        let first = enqueue_delivery_run(&path, &scheduled_run("run-1", "main.sqlite", 1.0))
            .expect("first run should enqueue");
        let second = enqueue_delivery_run(&path, &scheduled_run("run-2", "main.sqlite", 2.0))
            .expect("second run should enqueue");
        let barrier = Arc::new(Barrier::new(3));
        let handles = ["owner-a", "owner-b"].map(|owner| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                claim_delivery_run(path, first.id, owner, 0, 3.0, 100.0)
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("claimer should finish"));
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(DeliveryRunClaimOutcome::Claimed(_))))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(DeliveryRepositoryError::Conflict)))
                .count(),
            1
        );

        let claimed = load_delivery_run(&path, first.id)
            .expect("run should load")
            .expect("run should exist");
        let owner_id = claimed
            .owner_id
            .clone()
            .expect("claimed run should have owner");
        assert!(matches!(
            claim_delivery_run(&path, second.id, "owner-c", 0, 4.0, 100.0)
                .expect("competing claim should be classified"),
            DeliveryRunClaimOutcome::Busy(record) if record.id == first.id
        ));
        assert!(matches!(
            renew_delivery_run(&path, first.id, "wrong-owner", claimed.revision, 4.0, 100.0),
            Err(DeliveryRepositoryError::Conflict)
        ));
        let running = start_delivery_run(&path, first.id, &owner_id, claimed.revision, 4.0)
            .expect("current owner should start run");
        assert!(matches!(
            finalize_delivery_run(
                &path,
                first.id,
                &owner_id,
                claimed.revision,
                DeliveryRunStatus::Completed,
                None,
                None,
                5.0,
            ),
            Err(DeliveryRepositoryError::Conflict)
        ));
        let completed = finalize_delivery_run(
            &path,
            first.id,
            &owner_id,
            running.revision,
            DeliveryRunStatus::Completed,
            Some(r#"{"delivered":1}"#),
            None,
            5.0,
        )
        .expect("current owner and revision should finalize run");
        assert_eq!(completed.status, DeliveryRunStatus::Completed);
        assert!(matches!(
            claim_delivery_run(&path, second.id, "owner-c", 0, 6.0, 100.0)
                .expect("second claim should succeed after first completes"),
            DeliveryRunClaimOutcome::Claimed(_)
        ));
    }

    #[test]
    fn run_items_do_not_reclaim_ambiguous_sending_work() {
        let (_temp_dir, path) = migrated_auth_database();
        let run = enqueue_delivery_run(&path, &scheduled_run("item-run", "main.sqlite", 1.0))
            .expect("run should enqueue");
        let claimed = expect_claimed(
            claim_delivery_run(&path, run.id, "run-owner", run.revision, 2.0, 100.0)
                .expect("run should claim"),
        );
        let running = start_delivery_run(&path, run.id, "run-owner", claimed.revision, 3.0)
            .expect("run should start");
        insert_delivery_run_items(
            &path,
            run.id,
            &[DeliveryRunItemCreate {
                item_kind: DeliveryItemKind::Article,
                item_key: "article-41".to_string(),
                user_id: Some(7),
                article_id: Some(41),
            }],
            3.0,
        )
        .expect("item should insert");
        let claimed_item = claim_next_delivery_run_item(
            &path,
            run.id,
            "run-owner",
            running.revision,
            "item-owner",
            4.0,
            1.0,
        )
        .expect("item claim should succeed")
        .expect("pending item should be returned");
        let sending = mark_delivery_run_item_sending(
            &path,
            claimed_item.id,
            "item-owner",
            claimed_item.revision,
            4.5,
        )
        .expect("claimed item should enter sending state");
        assert!(claim_next_delivery_run_item(
            &path,
            run.id,
            "run-owner",
            running.revision,
            "replacement-owner",
            6.0,
            1.0,
        )
        .expect("claim scan should succeed")
        .is_none());
        assert!(matches!(
            finalize_delivery_run_item(
                &path,
                sending.id,
                "replacement-owner",
                sending.revision,
                DeliveryItemStatus::Succeeded,
                None,
                None,
                6.0,
            ),
            Err(DeliveryRepositoryError::Conflict)
        ));
        let unknown = finalize_delivery_run_item(
            &path,
            sending.id,
            "item-owner",
            sending.revision,
            DeliveryItemStatus::Unknown,
            None,
            Some("ambiguous_response"),
            6.0,
        )
        .expect("original owner should persist ambiguous outcome");
        assert_eq!(unknown.status, DeliveryItemStatus::Unknown);
        assert!(unknown.status.is_terminal());
    }

    #[test]
    fn expired_pre_send_item_claim_can_be_taken_over() {
        let (_temp_dir, path) = migrated_auth_database();
        let run = enqueue_delivery_run(&path, &scheduled_run("takeover-run", "main.sqlite", 1.0))
            .expect("run should enqueue");
        let claimed = expect_claimed(
            claim_delivery_run(&path, run.id, "run-owner", run.revision, 2.0, 100.0)
                .expect("run should claim"),
        );
        let running = start_delivery_run(&path, run.id, "run-owner", claimed.revision, 3.0)
            .expect("run should start");
        insert_delivery_run_items(
            &path,
            run.id,
            &[DeliveryRunItemCreate {
                item_kind: DeliveryItemKind::Article,
                item_key: "article-42".to_string(),
                user_id: Some(7),
                article_id: Some(42),
            }],
            3.0,
        )
        .expect("item should insert");
        let first = claim_next_delivery_run_item(
            &path,
            run.id,
            "run-owner",
            running.revision,
            "first-item-owner",
            4.0,
            1.0,
        )
        .expect("first item claim should succeed")
        .expect("item should be returned");
        let takeover = claim_next_delivery_run_item(
            &path,
            run.id,
            "run-owner",
            running.revision,
            "second-item-owner",
            6.0,
            1.0,
        )
        .expect("expired item claim should be reclaimed")
        .expect("same item should be returned");
        assert_eq!(takeover.id, first.id);
        assert_eq!(takeover.attempt_count, 2);
        assert!(takeover.revision > first.revision);
        assert!(matches!(
            finalize_delivery_run_item(
                &path,
                first.id,
                "first-item-owner",
                first.revision,
                DeliveryItemStatus::Succeeded,
                None,
                None,
                6.5,
            ),
            Err(DeliveryRepositoryError::Conflict)
        ));
    }

    #[test]
    fn workflow_leases_preserve_monotonic_revision_across_takeover_and_release() {
        let (_temp_dir, path) = migrated_auth_database();
        let first = enqueue_delivery_run(&path, &scheduled_run("lease-1", "main.sqlite", 1.0))
            .expect("first run should enqueue");
        let second = enqueue_delivery_run(&path, &scheduled_run("lease-2", "main.sqlite", 2.0))
            .expect("second run should enqueue");
        let barrier = Arc::new(Barrier::new(3));
        let handles =
            [(first.id, "owner-a"), (second.id, "owner-b")].map(|(delivery_run_id, owner_id)| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    acquire_delivery_lease(
                        path,
                        DeliveryWorkflow::Notify,
                        "main.sqlite",
                        delivery_run_id,
                        owner_id,
                        3.0,
                        2.0,
                    )
                })
            });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("lease contender should finish"));
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(DeliveryLeaseAcquireOutcome::Acquired(_))))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(DeliveryLeaseAcquireOutcome::Busy(_))))
                .count(),
            1
        );
        let first_lease = results
            .iter()
            .find_map(|result| match result {
                Ok(DeliveryLeaseAcquireOutcome::Acquired(record)) => Some(record.clone()),
                _ => None,
            })
            .expect("one contender should own the lease");
        let first_owner = first_lease
            .owner_id
            .clone()
            .expect("acquired lease should have owner");
        let first_run_id = first_lease
            .delivery_run_id
            .expect("acquired lease should have run");
        let (takeover_run_id, takeover_owner) = if first_run_id == first.id {
            (second.id, "owner-b")
        } else {
            (first.id, "owner-a")
        };
        let takeover = match acquire_delivery_lease(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            takeover_run_id,
            takeover_owner,
            6.0,
            2.0,
        )
        .expect("expired lease should be acquired")
        {
            DeliveryLeaseAcquireOutcome::Acquired(record) => record,
            other => panic!("expired lease should be acquired, got {other:?}"),
        };
        assert!(takeover.revision > first_lease.revision);
        assert!(matches!(
            release_delivery_lease(
                &path,
                DeliveryWorkflow::Notify,
                "main.sqlite",
                first_run_id,
                &first_owner,
                first_lease.revision,
                6.5,
            ),
            Err(DeliveryRepositoryError::Conflict)
        ));
        let renewed = renew_delivery_lease(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            takeover_run_id,
            takeover_owner,
            takeover.revision,
            6.5,
            2.0,
        )
        .expect("current owner should renew lease");
        let released = release_delivery_lease(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            takeover_run_id,
            takeover_owner,
            renewed.revision,
            7.0,
        )
        .expect("current owner should release lease");
        assert!(released.owner_id.is_none());
        assert!(released.revision > renewed.revision);
    }

    #[test]
    fn dedupe_identity_allows_one_reservation_and_terminal_resolution() {
        let (_temp_dir, path) = migrated_auth_database();
        let run = enqueue_delivery_run(&path, &scheduled_run("dedupe-run", "main.sqlite", 1.0))
            .expect("run should enqueue");
        let barrier = Arc::new(Barrier::new(3));
        let handles = ["owner-a", "owner-b"].map(|owner| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                reserve_delivery_dedupe(
                    path,
                    DeliveryWorkflow::Notify,
                    "main.sqlite",
                    7,
                    41,
                    run.id,
                    owner,
                    2.0,
                )
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("reserver should finish"));
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(DeliveryDedupeReserveOutcome::Reserved(_))))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(DeliveryDedupeReserveOutcome::Existing(_))))
                .count(),
            1
        );
        let reservation =
            load_delivery_dedupe(&path, DeliveryWorkflow::Notify, "main.sqlite", 7, 41)
                .expect("dedupe should load")
                .expect("dedupe should exist");
        let owner_id = reservation
            .reservation_owner
            .clone()
            .expect("reservation should have an owner");
        let resolved = resolve_delivery_dedupe(
            &path,
            reservation.id,
            run.id,
            &owner_id,
            reservation.revision,
            DeliveryDedupeStatus::Unknown,
            None,
            3.0,
        )
        .expect("reservation should resolve");
        assert_eq!(resolved.status, DeliveryDedupeStatus::Unknown);
        assert!(matches!(
            reserve_delivery_dedupe(
                &path,
                DeliveryWorkflow::Notify,
                "main.sqlite",
                7,
                41,
                run.id,
                "owner-c",
                4.0,
            )
            .expect("terminal dedupe should be returned"),
            DeliveryDedupeReserveOutcome::Existing(record)
                if record.status == DeliveryDedupeStatus::Unknown
        ));
    }

    #[test]
    fn idempotent_admission_and_atomic_finalization_preserve_all_revisions() {
        let (_temp_dir, path) = migrated_auth_database();
        let create = scheduled_run("atomic-run", "main.sqlite", 1.0);
        let enqueued =
            match admit_delivery_run(&path, &create).expect("first admission should succeed") {
                DeliveryRunAdmissionOutcome::Enqueued(record) => record,
                other => panic!("first admission should enqueue, got {other:?}"),
            };
        assert!(matches!(
            admit_delivery_run(&path, &create).expect("repeated admission should load"),
            DeliveryRunAdmissionOutcome::Existing(record) if record.id == enqueued.id
        ));
        let claimed = expect_claimed(
            claim_delivery_run(&path, enqueued.id, "owner-a", enqueued.revision, 2.0, 100.0)
                .expect("run should claim"),
        );
        let lease = match acquire_delivery_lease(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            enqueued.id,
            "owner-a",
            2.0,
            100.0,
        )
        .expect("workflow lease should acquire")
        {
            DeliveryLeaseAcquireOutcome::Acquired(record) => record,
            other => panic!("workflow lease should acquire, got {other:?}"),
        };
        let running = start_delivery_run(&path, enqueued.id, "owner-a", claimed.revision, 3.0)
            .expect("run should start");
        let item_create = DeliveryRunItemCreate {
            item_kind: DeliveryItemKind::Subscriber,
            item_key: "7".to_string(),
            user_id: Some(7),
            article_id: None,
        };
        let first_items =
            ensure_delivery_run_items(&path, enqueued.id, std::slice::from_ref(&item_create), 3.0)
                .expect("item should be ensured");
        let repeated_items = ensure_delivery_run_items(&path, enqueued.id, &[item_create], 4.0)
            .expect("item ensure should be idempotent");
        assert_eq!(first_items[0].id, repeated_items[0].id);
        assert_eq!(first_items[0].revision, repeated_items[0].revision);
        let claimed_item = claim_delivery_run_item(
            &path,
            enqueued.id,
            "owner-a",
            running.revision,
            first_items[0].id,
            "owner-a",
            4.0,
            100.0,
        )
        .expect("subscriber item should claim");
        let sending = mark_delivery_run_item_sending(
            &path,
            claimed_item.id,
            "owner-a",
            claimed_item.revision,
            5.0,
        )
        .expect("subscriber item should enter sending");
        let reservation = match reserve_delivery_dedupe(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            7,
            41,
            enqueued.id,
            "owner-a",
            5.0,
        )
        .expect("dedupe should reserve")
        {
            DeliveryDedupeReserveOutcome::Reserved(record) => record,
            other => panic!("dedupe should reserve, got {other:?}"),
        };
        let terminal_item = finalize_delivery_attempt(
            &path,
            sending.id,
            "owner-a",
            sending.revision,
            DeliveryItemStatus::Unknown,
            Some(r#"{"selected_article_ids":[41]}"#),
            Some("ambiguous_delivery"),
            enqueued.id,
            &[DeliveryDedupeResolution {
                id: reservation.id,
                expected_revision: reservation.revision,
            }],
            DeliveryDedupeStatus::Unknown,
            None,
            6.0,
        )
        .expect("item and dedupe should finalize atomically");
        assert_eq!(terminal_item.status, DeliveryItemStatus::Unknown);
        let update = DeliveryCheckpointUpdate {
            status: DeliveryCheckpointStatus::Unknown,
            snapshot_json: "{}".to_string(),
            last_completed_run_at: None,
            updated_at: 7.0,
        };
        assert!(matches!(
            finalize_delivery_run_with_checkpoint(
                &path,
                enqueued.id,
                "owner-a",
                running.revision,
                DeliveryRunStatus::Unknown,
                Some(r#"{"subscriber_count":1}"#),
                Some("ambiguous_delivery"),
                DeliveryWorkflow::Notify,
                "main.sqlite",
                None,
                &update,
                lease.revision + 1,
            ),
            Err(DeliveryRepositoryError::Conflict)
        ));
        assert!(
            load_delivery_checkpoint(&path, DeliveryWorkflow::Notify, "main.sqlite")
                .expect("checkpoint lookup should succeed")
                .is_none()
        );
        assert_eq!(
            load_delivery_run(&path, enqueued.id)
                .expect("run should load")
                .expect("run should exist")
                .status,
            DeliveryRunStatus::Running
        );
        let finalization = finalize_delivery_run_with_checkpoint(
            &path,
            enqueued.id,
            "owner-a",
            running.revision,
            DeliveryRunStatus::Unknown,
            Some(r#"{"subscriber_count":1}"#),
            Some("ambiguous_delivery"),
            DeliveryWorkflow::Notify,
            "main.sqlite",
            None,
            &update,
            lease.revision,
        )
        .expect("run checkpoint and lease should finalize atomically");
        assert_eq!(finalization.run.status, DeliveryRunStatus::Unknown);
        assert_eq!(
            finalization.checkpoint.status,
            DeliveryCheckpointStatus::Unknown
        );
        assert!(finalization.lease.owner_id.is_none());
    }

    #[test]
    fn takeover_recovery_retries_claimed_and_quarantines_sending_items() {
        let (_temp_dir, path) = migrated_auth_database();
        let run = enqueue_delivery_run(&path, &scheduled_run("recovery-run", "main.sqlite", 1.0))
            .expect("run should enqueue");
        let claimed = expect_claimed(
            claim_delivery_run(&path, run.id, "owner-a", run.revision, 2.0, 1.0)
                .expect("run should claim"),
        );
        match acquire_delivery_lease(
            &path,
            DeliveryWorkflow::Notify,
            "main.sqlite",
            run.id,
            "owner-a",
            2.0,
            1.0,
        )
        .expect("workflow lease should acquire")
        {
            DeliveryLeaseAcquireOutcome::Acquired(_) => {}
            other => panic!("workflow lease should acquire, got {other:?}"),
        }
        let running = start_delivery_run(&path, run.id, "owner-a", claimed.revision, 2.1)
            .expect("run should start");
        let items = ensure_delivery_run_items(
            &path,
            run.id,
            &[
                DeliveryRunItemCreate {
                    item_kind: DeliveryItemKind::Subscriber,
                    item_key: "7".to_string(),
                    user_id: Some(7),
                    article_id: None,
                },
                DeliveryRunItemCreate {
                    item_kind: DeliveryItemKind::Subscriber,
                    item_key: "8".to_string(),
                    user_id: Some(8),
                    article_id: None,
                },
            ],
            2.1,
        )
        .expect("subscriber items should insert");
        let claimed_before_send = claim_delivery_run_item(
            &path,
            run.id,
            "owner-a",
            running.revision,
            items[0].id,
            "owner-a",
            2.2,
            100.0,
        )
        .expect("first item should claim");
        let sending = claim_delivery_run_item(
            &path,
            run.id,
            "owner-a",
            running.revision,
            items[1].id,
            "owner-a",
            2.2,
            100.0,
        )
        .and_then(|item| {
            mark_delivery_run_item_sending(&path, item.id, "owner-a", item.revision, 2.3)
        })
        .expect("second item should enter sending");
        for (user_id, article_id) in [(7, 41), (8, 42)] {
            assert!(matches!(
                reserve_delivery_dedupe(
                    &path,
                    DeliveryWorkflow::Notify,
                    "main.sqlite",
                    user_id,
                    article_id,
                    run.id,
                    "owner-a",
                    2.4,
                )
                .expect("dedupe should reserve"),
                DeliveryDedupeReserveOutcome::Reserved(_)
            ));
        }
        let takeover = expect_claimed(
            claim_delivery_run(&path, run.id, "owner-b", running.revision, 4.0, 100.0)
                .expect("expired run should be reclaimed"),
        );
        assert!(matches!(
            acquire_delivery_lease(
                &path,
                DeliveryWorkflow::Notify,
                "main.sqlite",
                run.id,
                "owner-b",
                4.0,
                100.0,
            )
            .expect("expired workflow lease should be reclaimed"),
            DeliveryLeaseAcquireOutcome::Acquired(_)
        ));
        let recovery =
            reconcile_delivery_run_after_takeover(&path, run.id, "owner-b", takeover.revision, 4.0)
                .expect("expired item state should reconcile");
        assert_eq!(recovery.reset_item_count, 1);
        assert_eq!(recovery.unknown_item_count, 1);
        assert_eq!(recovery.released_dedupe_count, 1);
        assert_eq!(recovery.unknown_dedupe_count, 1);
        let stored_items = list_delivery_run_items(&path, run.id).expect("items should load");
        assert_eq!(
            stored_items
                .iter()
                .find(|item| item.id == claimed_before_send.id)
                .expect("claimed item should remain")
                .status,
            DeliveryItemStatus::Pending
        );
        assert_eq!(
            stored_items
                .iter()
                .find(|item| item.id == sending.id)
                .expect("sending item should remain")
                .status,
            DeliveryItemStatus::Unknown
        );
        assert!(
            load_delivery_dedupe(&path, DeliveryWorkflow::Notify, "main.sqlite", 7, 41,)
                .expect("released dedupe should load")
                .is_none()
        );
        assert_eq!(
            load_delivery_dedupe(&path, DeliveryWorkflow::Notify, "main.sqlite", 8, 42,)
                .expect("unknown dedupe should load")
                .expect("unknown dedupe should exist")
                .status,
            DeliveryDedupeStatus::Unknown
        );
    }

    #[test]
    fn legacy_import_is_idempotent_preserves_files_and_sanitizes_unknown_statuses() {
        let (temp_dir, path) = migrated_auth_database();
        let config = StorageConfig::from_project_root(temp_dir.path()).with_auth_db_path(&path);
        let state_dir = temp_dir.path().join("data/push_state");
        fs::create_dir_all(&state_dir).expect("legacy state directory should be created");
        let state_path = state_dir.join("sample.json");
        let changes_path = state_dir.join("sample.changes.json");
        let state = serde_json::json!({
            "db_name": "sample.sqlite",
            "status": "private-checkpoint-status",
            "last_completed_run_at": "2026-07-27T00:00:00Z",
            "snapshot": {
                "issue_article_counts": {"1:2": 3},
                "inpress_article_counts": {"4": 5}
            },
            "run": {
                "run_id": "legacy-run",
                "status": "running",
                "pending_issue_keys": ["1:2"],
                "done_issue_keys": ["1:3"],
                "pending_inpress_keys": ["4"],
                "done_inpress_keys": ["5"],
                "delivered_article_ids": [31],
                "user_results": [{
                    "subscriber_id": "7",
                    "selected_count": 2,
                    "pushed_count": 1,
                    "folder_synced_count": 0,
                    "status": "private-user-status"
                }]
            },
            "delivery_dedupe": {"7:31": "2026-07-27T00:00:01Z"}
        });
        let state_bytes = serde_json::to_vec(&state).expect("legacy state should encode");
        fs::write(&state_path, &state_bytes).expect("legacy state should be written");
        fs::write(&changes_path, b"not-json-and-must-remain-untouched")
            .expect("changes file should be written");

        let imported = import_legacy_delivery_state_files(&config, 100.0)
            .expect("valid legacy state should import");
        assert_eq!(
            imported,
            LegacyDeliveryImportResult {
                discovered_count: 1,
                imported_count: 1,
                skipped_count: 0,
                item_count: 5,
                dedupe_count: 1,
            }
        );
        let checkpoint = load_delivery_checkpoint(&path, DeliveryWorkflow::Notify, "sample.sqlite")
            .expect("checkpoint should load")
            .expect("checkpoint should exist");
        assert_eq!(checkpoint.status, DeliveryCheckpointStatus::Unknown);
        assert_eq!(checkpoint.legacy_status.as_deref(), Some("unrecognized"));
        let connection = Connection::open(&path).expect("database should open");
        let run_id: i64 = connection
            .query_row(
                "SELECT id FROM delivery_runs WHERE external_id = 'legacy-run'",
                [],
                |row| row.get(0),
            )
            .expect("legacy run should exist");
        drop(connection);
        let run = load_delivery_run(&path, run_id)
            .expect("run should load")
            .expect("run should exist");
        assert_eq!(run.status, DeliveryRunStatus::Unknown);
        assert_eq!(run.legacy_status.as_deref(), Some("abandoned_active"));
        let items = list_delivery_run_items(&path, run_id).expect("legacy items should load");
        assert_eq!(items.len(), 5);
        assert!(items.iter().any(|item| {
            item.item_kind == DeliveryItemKind::Subscriber
                && item.status == DeliveryItemStatus::Unknown
                && item.legacy_status.as_deref() == Some("unrecognized")
        }));
        let persisted = format!("{checkpoint:?}{run:?}{items:?}");
        assert!(!persisted.contains("private-checkpoint-status"));
        assert!(!persisted.contains("private-user-status"));
        assert_eq!(
            fs::read(&state_path).expect("state should remain"),
            state_bytes
        );
        assert_eq!(
            fs::read(&changes_path).expect("changes file should remain"),
            b"not-json-and-must-remain-untouched"
        );

        let repeated = import_legacy_delivery_state_files(&config, 101.0)
            .expect("same legacy hash should be idempotent");
        assert_eq!(repeated.imported_count, 0);
        assert_eq!(repeated.skipped_count, 1);
        fs::write(
            &state_path,
            serde_json::to_vec(&serde_json::json!({
                "db_name": "sample.sqlite",
                "status": "completed"
            }))
            .expect("changed state should encode"),
        )
        .expect("legacy state should change");
        assert!(matches!(
            import_legacy_delivery_state_files(&config, 102.0),
            Err(DeliveryRepositoryError::LegacyImportConflict)
        ));
        assert_eq!(row_count(&path, "delivery_checkpoints"), 1);
        assert_eq!(row_count(&path, "delivery_runs"), 1);
        assert_eq!(row_count(&path, "delivery_run_items"), 5);
        assert_eq!(row_count(&path, "delivery_dedupe"), 1);
    }

    #[test]
    fn corrupt_legacy_file_prevents_every_file_from_importing() {
        let (temp_dir, path) = migrated_auth_database();
        let config = StorageConfig::from_project_root(temp_dir.path()).with_auth_db_path(&path);
        let state_dir = temp_dir.path().join("data/folder_push_state");
        fs::create_dir_all(&state_dir).expect("legacy state directory should be created");
        let valid_path = state_dir.join("alpha.json");
        let corrupt_path = state_dir.join("beta.json");
        fs::write(
            &valid_path,
            serde_json::to_vec(&serde_json::json!({
                "db_name": "alpha.sqlite",
                "status": "completed"
            }))
            .expect("valid state should encode"),
        )
        .expect("valid state should be written");
        fs::write(&corrupt_path, b"{").expect("corrupt state should be written");

        assert!(matches!(
            import_legacy_delivery_state_files(&config, 100.0),
            Err(DeliveryRepositoryError::Json(_))
        ));
        assert_eq!(row_count(&path, "delivery_checkpoints"), 0);
        assert_eq!(row_count(&path, "delivery_runs"), 0);
        assert!(valid_path.exists());
        assert!(corrupt_path.exists());
    }

    #[test]
    fn legacy_status_translation_recognizes_existing_writer_values() {
        assert_eq!(
            legacy_item_status("error"),
            (DeliveryItemStatus::Failed, None)
        );
        assert_eq!(
            legacy_item_status("future-private-value"),
            (DeliveryItemStatus::Unknown, Some("unrecognized"))
        );
        assert_eq!(
            legacy_run_status("running"),
            (DeliveryRunStatus::Unknown, Some("abandoned_active"))
        );
    }
}
