//! Typed repositories for migrated auth database business routes.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::{
    validate_scheduled_task_timing, AdminInviteCodeInfo, AdminStatsResponse, AdminUserInfo,
    AnnouncementInfo, AuthStats, FavoriteAdd, FavoriteArticleRef, FavoriteArticleResponse,
    FavoriteBatchCheckResponse, FavoriteCheckResponse, FavoriteResponse, FolderResponse,
    IndexDatabaseStats, IndexStats, NotificationSettings, NotificationSettingsUpdate,
    NotificationSubscriberInfo, PushStats, RuntimeSettingInfo, RuntimeSettingValue,
    ScheduledJobSpec, ScheduledTaskInfo, ScheduledTaskRunInfo, SchedulerStatusResponse,
    SchedulerWorkerInfo, UserId,
};
use rusqlite::types::Type;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::Value;

use crate::secrets::{notification_context, runtime_context};
use crate::{open_sqlite_connection, random_hex, SecretCodec, SecretError, StorageConfig};

mod admin;
mod favorites;
mod notifications;
mod runtime_settings;
mod scheduled_tasks;
mod shared;

pub use admin::{
    admin_create_invite_code, create_announcement, delete_announcement, delete_invite_code,
    delete_user, get_admin_stats, get_announcement, list_all_announcements, list_all_invite_codes,
    list_all_users, set_user_admin, update_announcement,
};
pub use favorites::{
    add_favorite, batch_is_favorited, bulk_add_favorites, bulk_move_favorites,
    bulk_remove_favorites, count_favorites, create_folder, delete_folder, get_tracking_folder,
    is_favorited, list_favorite_articles, list_favorites, list_folders, remove_favorite,
    rename_folder, set_tracking_folder,
};
pub use notifications::{
    get_notification_settings, list_notification_subscribers, upsert_notification_settings,
};
pub use runtime_settings::{list_runtime_settings, load_runtime_settings, upsert_runtime_settings};
pub use scheduled_tasks::{
    claim_ready_scheduled_runs, create_scheduled_task, delete_scheduled_task,
    enqueue_scheduled_runs, finish_scheduled_run, get_scheduled_task,
    get_scheduler_last_checked_at, get_scheduler_status, heartbeat_scheduled_run,
    list_scheduled_tasks, record_scheduled_task_run, record_scheduler_check,
    record_scheduler_heartbeat, start_scheduled_run, update_scheduled_task, ScheduledRunClaim,
    ScheduledTaskCreateParams, ScheduledTaskUpdateParams,
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
    /// Secret encryption or decryption failed.
    Secret(SecretError),
    /// Folder name duplicates an existing user folder.
    DuplicateFolderName,
    /// Folder does not exist for the user.
    FolderNotFound,
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
    /// A null update attempted to clear a non-secret runtime setting.
    NonSecretRuntimeSettingCannotBeCleared(String),
    /// Scheduled job arguments failed allowlist validation.
    InvalidScheduledJob(String),
    /// Scheduled task timing settings failed validation.
    InvalidScheduledTask(String),
    /// A migrated legacy task was enabled without a typed replacement job.
    LegacyScheduledTaskCannotBeEnabled,
}

impl fmt::Display for BusinessRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Secret(error) => write!(formatter, "{error}"),
            Self::DuplicateFolderName => formatter.write_str("Folder name already exists"),
            Self::FolderNotFound => formatter.write_str("Folder not found"),
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
            Self::NonSecretRuntimeSettingCannotBeCleared(field) => {
                write!(formatter, "Only secret runtime settings may be cleared: {field}")
            }
            Self::InvalidScheduledJob(message) => formatter.write_str(message),
            Self::InvalidScheduledTask(message) => formatter.write_str(message),
            Self::LegacyScheduledTaskCannotBeEnabled => formatter.write_str(
                "A legacy scheduled task must be replaced with a typed job before it can be enabled",
            ),
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
