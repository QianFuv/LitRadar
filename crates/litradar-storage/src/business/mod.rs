//! Typed repositories for migrated auth database business routes.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_domain::{
    is_valid_invite_code_policy, validate_scheduled_task_timing, AdminInviteCodeInfo,
    AdminStatsResponse, AdminUserInfo, AnnouncementInfo, AuthStats, FavoriteAdd,
    FavoriteArticleRef, FavoriteArticleResponse, FavoriteBatchCheckResponse, FavoriteCheckResponse,
    FavoriteResponse, FolderResponse, IndexDatabaseStats, IndexStats, InputValidationError,
    InviteCodeStatus, NotificationSettings, NotificationSettingsUpdate, NotificationSubscriberInfo,
    ProviderOrderConfiguration, PushStats, PushStatsState, RuntimeSecretItemInfo,
    RuntimeSecretPoolUpdate, RuntimeSettingInfo, RuntimeSettingValue, ScheduledJobSpec,
    ScheduledTaskInfo, ScheduledTaskRunInfo, SchedulerRunState, SchedulerStatusResponse,
    SchedulerWorkerInfo, UserId, DEFAULT_INVITE_CODE_MAX_USES, DEFAULT_INVITE_CODE_TTL_SECONDS,
};
use rusqlite::types::Type;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::Value;

use crate::secrets::{notification_context, runtime_context};
use crate::{open_sqlite_connection, random_hex, SecretCodec, SecretError, StorageConfig};

mod admin;
mod delivery;
mod favorites;
mod notifications;
mod runtime_settings;
mod scheduled_tasks;
mod security_audit;
mod shared;

pub use admin::{
    admin_create_invite_code, admin_create_invite_code_with_audit,
    admin_create_invite_code_with_policy_and_audit, create_announcement,
    create_announcement_with_audit, delete_announcement, delete_announcement_with_audit,
    delete_user, delete_user_with_audit, get_admin_stats, get_announcement, list_all_announcements,
    list_all_invite_codes, list_all_users, revoke_admin_invite_code,
    revoke_admin_invite_code_with_audit, set_user_admin, set_user_admin_with_audit,
    update_announcement, update_announcement_with_audit,
};
pub use delivery::{
    acquire_delivery_lease, admit_delivery_run, admit_manual_delivery_run, claim_delivery_run,
    claim_delivery_run_item, claim_next_delivery_run_item, cleanup_confirmed_delivery_dedupe,
    compare_and_swap_delivery_checkpoint, enqueue_delivery_run, ensure_delivery_run_items,
    finalize_delivery_attempt, finalize_delivery_run, finalize_delivery_run_item,
    finalize_delivery_run_with_checkpoint, finalize_queued_delivery_run,
    import_legacy_delivery_state_files, insert_delivery_run_items, list_delivery_dedupe_for_scope,
    list_delivery_run_items, list_dispatchable_manual_delivery_runs, load_delivery_checkpoint,
    load_delivery_dedupe, load_delivery_lease, load_delivery_run, load_latest_manual_delivery_run,
    load_manual_delivery_run_by_external_id, load_manual_delivery_run_by_external_id_for_admin,
    mark_delivery_run_item_sending, reconcile_delivery_run_after_takeover,
    release_delivery_dedupe_reservation, release_delivery_dedupe_reservations,
    release_delivery_lease, renew_delivery_lease, renew_delivery_run, renew_delivery_run_item,
    request_delivery_run_cancellation, reserve_delivery_dedupe, resolve_delivery_dedupe,
    start_delivery_run, DeliveryCheckpointRecord, DeliveryCheckpointStatus,
    DeliveryCheckpointUpdate, DeliveryDedupeRecord, DeliveryDedupeReserveOutcome,
    DeliveryDedupeResolution, DeliveryDedupeStatus, DeliveryItemKind, DeliveryItemStatus,
    DeliveryLeaseAcquireOutcome, DeliveryLeaseRecord, DeliveryRecoveryResult,
    DeliveryRepositoryError, DeliveryRunAdmissionOutcome, DeliveryRunClaimOutcome,
    DeliveryRunCreate, DeliveryRunFinalization, DeliveryRunItemCreate, DeliveryRunItemRecord,
    DeliveryRunMode, DeliveryRunRecord, DeliveryRunStatus, DeliveryTriggerKind, DeliveryWorkflow,
    LegacyDeliveryImportResult, ManualDeliveryRunAdmissionOutcome,
};
pub use favorites::{
    add_favorite, batch_is_favorited, bulk_add_favorites, bulk_move_favorites,
    bulk_remove_favorites, count_favorites, create_folder, delete_folder, get_tracking_folder,
    is_favorited, list_favorite_articles, list_favorites, list_folders, remove_favorite,
    rename_folder, set_tracking_folder,
};
pub use notifications::{
    get_notification_settings, get_notification_subscriber, list_notification_subscribers,
    upsert_notification_settings,
};
pub use runtime_settings::{
    canonicalize_outbound_base_url, list_runtime_settings, load_ai_allowed_base_urls,
    load_audit_retention_days, load_delivery_worker_concurrency, load_runtime_logging_settings,
    load_runtime_settings, parse_runtime_setting, runtime_setting_default, upsert_runtime_settings,
    upsert_runtime_settings_with_audit, AuthRateLimitPolicy, ParsedRuntimeSettingValue,
    RuntimeLoggingSettings, RuntimeSettingKey, TokenBucketPolicy, TrustedProxyCidr,
    DEFAULT_AUTH_RATE_LIMIT_POLICY_JSON, DEFAULT_DELIVERY_WORKER_CONCURRENCY,
    DEFAULT_RUNTIME_LOG_FILTER, DEFAULT_RUNTIME_LOG_FORMAT, MAX_DELIVERY_WORKER_CONCURRENCY,
};
pub use scheduled_tasks::{
    claim_ready_scheduled_runs, create_scheduled_task, create_scheduled_task_with_audit,
    delete_scheduled_task, delete_scheduled_task_with_audit, enqueue_scheduled_runs,
    finish_scheduled_run, get_scheduled_task, get_scheduler_last_checked_at, get_scheduler_status,
    heartbeat_scheduled_run, list_scheduled_tasks, record_scheduled_task_run,
    record_scheduler_check, record_scheduler_heartbeat, start_scheduled_run, update_scheduled_task,
    update_scheduled_task_with_audit, ScheduledRunClaim, ScheduledTaskCreateParams,
    ScheduledTaskUpdateParams,
};
pub(crate) use security_audit::insert_required_security_audit_event;
pub use security_audit::{
    append_security_audit_event, cleanup_security_audit_events, list_security_audit_events,
    report_security_audit_persistence_failure, security_audit_persistence_failure_count,
    SecurityAuditError, SecurityAuditEvent, SecurityAuditRecord, SecurityAuditRetentionResult,
    DEFAULT_AUDIT_RETENTION_DAYS, MAX_AUDIT_RETENTION_DAYS, MIN_AUDIT_RETENTION_DAYS,
};
pub use shared::{count_weekly_articles, list_available_database_names, normalize_database_names};

/// Repository errors for migrated business routes.
#[derive(Debug)]
pub enum BusinessRepositoryError {
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// Filesystem access failed.
    Io(std::io::Error),
    /// JSON parsing or encoding failed.
    Json(serde_json::Error),
    /// Stored notification list JSON is malformed or contains non-string values.
    InvalidNotificationListState,
    /// Secret encryption or decryption failed.
    Secret(SecretError),
    /// Folder name duplicates an existing user folder.
    DuplicateFolderName,
    /// Folder does not exist for the user.
    FolderNotFound,
    /// A shared business-input bound was violated.
    InvalidInput(InputValidationError),
    /// Source and target folder are identical.
    SourceAndTargetFoldersSame,
    /// Source folder does not exist for the user.
    SourceFolderNotFound,
    /// Target folder does not exist for the user.
    TargetFolderNotFound,
    /// Runtime setting field is not managed.
    UnknownRuntimeSetting(String),
    /// Runtime boolean could not be parsed.
    InvalidRuntimeBoolean(String),
    /// A managed runtime setting failed structural validation.
    InvalidRuntimeSetting(String),
    /// A user-selected outbound endpoint is not in the administrator allowlist.
    OutboundEndpointNotAllowed,
    /// A null update attempted to clear a non-secret runtime setting.
    NonSecretRuntimeSettingCannotBeCleared(String),
    /// A secret-pool mutation contained an invalid field or item reference.
    InvalidRuntimeSecretPoolUpdate(String),
    /// Scheduled job arguments failed allowlist validation.
    InvalidScheduledJob(String),
    /// Scheduled task timing settings failed validation.
    InvalidScheduledTask(String),
    /// A migrated legacy task was enabled without a typed replacement job.
    LegacyScheduledTaskCannotBeEnabled,
    /// An invite expiration or use quota falls outside the managed policy.
    InvalidInvitePolicy,
    /// The administrative actor no longer exists or lacks administrator privileges.
    AdministratorActorForbidden,
    /// The target user does not exist.
    AdministratorTargetNotFound,
    /// A mutation would remove the final administrator.
    AdministratorInvariantViolation,
    /// A required durable security audit row could not be persisted.
    AuditPersistence(SecurityAuditError),
}

impl fmt::Display for BusinessRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::InvalidNotificationListState => {
                formatter.write_str("Stored notification list state is invalid")
            }
            Self::Secret(error) => write!(formatter, "{error}"),
            Self::DuplicateFolderName => formatter.write_str("Folder name already exists"),
            Self::FolderNotFound => formatter.write_str("Folder not found"),
            Self::InvalidInput(error) => write!(formatter, "{error}"),
            Self::SourceAndTargetFoldersSame => {
                formatter.write_str("Source and target folders must be different")
            }
            Self::SourceFolderNotFound => formatter.write_str("Source folder not found"),
            Self::TargetFolderNotFound => formatter.write_str("Target folder not found"),
            Self::UnknownRuntimeSetting(field) => {
                write!(formatter, "Unknown runtime setting: {field}")
            }
            Self::InvalidRuntimeBoolean(value) => {
                write!(formatter, "Invalid boolean value: {value}")
            }
            Self::InvalidRuntimeSetting(message) => formatter.write_str(message),
            Self::OutboundEndpointNotAllowed => {
                formatter.write_str("Outbound endpoint is not allowed")
            }
            Self::NonSecretRuntimeSettingCannotBeCleared(field) => {
                write!(formatter, "Only secret runtime settings may be cleared: {field}")
            }
            Self::InvalidRuntimeSecretPoolUpdate(field) => {
                write!(formatter, "Invalid runtime secret pool update: {field}")
            }
            Self::InvalidScheduledJob(message) => formatter.write_str(message),
            Self::InvalidScheduledTask(message) => formatter.write_str(message),
            Self::LegacyScheduledTaskCannotBeEnabled => formatter.write_str(
                "A legacy scheduled task must be replaced with a typed job before it can be enabled",
            ),
            Self::InvalidInvitePolicy => formatter.write_str(
                "Invite code expires_at must be in the next 365 days and max_uses must be between 1 and 1000",
            ),
            Self::AdministratorActorForbidden => formatter.write_str("Admin access required"),
            Self::AdministratorTargetNotFound => formatter.write_str("User not found"),
            Self::AdministratorInvariantViolation => {
                formatter.write_str("At least one administrator is required")
            }
            Self::AuditPersistence(_) => formatter.write_str("Security audit persistence failed"),
        }
    }
}

impl Error for BusinessRepositoryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Secret(error) => Some(error),
            Self::InvalidInput(error) => Some(error),
            Self::AuditPersistence(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for BusinessRepositoryError {
    /// Convert SQLite errors into repository errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for BusinessRepositoryError {
    /// Convert IO errors into repository errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for BusinessRepositoryError {
    /// Convert JSON errors into repository errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<SecretError> for BusinessRepositoryError {
    /// Convert secret errors into repository errors.
    fn from(error: SecretError) -> Self {
        Self::Secret(error)
    }
}

impl From<InputValidationError> for BusinessRepositoryError {
    /// Convert shared input validation errors into repository errors.
    fn from(error: InputValidationError) -> Self {
        Self::InvalidInput(error)
    }
}

impl From<SecurityAuditError> for BusinessRepositoryError {
    /// Convert required audit persistence failures into fail-closed business errors.
    fn from(error: SecurityAuditError) -> Self {
        Self::AuditPersistence(error)
    }
}
