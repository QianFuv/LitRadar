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

const ADMIN_INVITE_CODE_BYTES: i64 = 8;

#[derive(Debug, Clone, Copy)]
struct RuntimeConfigDefinition {
    field: &'static str,
    label: &'static str,
    input_type: &'static str,
    is_secret: bool,
    description: &'static str,
    default_value: &'static str,
}

const RUNTIME_CONFIG_DEFINITIONS: [RuntimeConfigDefinition; 7] = [
    RuntimeConfigDefinition {
        field: "openalex_api_key_pool",
        label: "OpenAlex API key pool",
        input_type: "password",
        is_secret: true,
        description: "OpenAlex authenticated request key pool.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "semantic_scholar_api_key_pool",
        label: "Semantic Scholar API key pool",
        input_type: "password",
        is_secret: true,
        description: "Comma- or semicolon-separated Semantic Scholar REST API keys.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "crossref_mailto_pool",
        label: "Crossref mailto pool",
        input_type: "email",
        is_secret: false,
        description: "Comma- or semicolon-separated Crossref contact emails.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "cors_allowed_origins",
        label: "CORS allowed origins",
        input_type: "text",
        is_secret: false,
        description: "Comma-separated browser origins allowed to send credentialed API requests.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "mcp_allowed_hosts",
        label: "MCP allowed hosts",
        input_type: "text",
        is_secret: false,
        description: "Comma-separated hosts accepted by the Streamable HTTP MCP endpoint.",
        default_value: "localhost,127.0.0.1,::1",
    },
    RuntimeConfigDefinition {
        field: "mcp_allowed_origins",
        label: "MCP allowed origins",
        input_type: "text",
        is_secret: false,
        description:
            "Comma-separated browser origins accepted by the Streamable HTTP MCP endpoint.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "secure_cookies",
        label: "Secure session cookies",
        input_type: "boolean",
        is_secret: false,
        description: "Whether session cookies include the Secure attribute.",
        default_value: "false",
    },
];

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

/// Borrowed values used to create one scheduled task.
#[derive(Debug, Clone, Copy)]
pub struct ScheduledTaskCreateParams<'a> {
    /// Task name.
    pub name: &'a str,
    /// Validated job specification.
    pub job: &'a ScheduledJobSpec,
    /// Five-field cron expression.
    pub cron: &'a str,
    /// IANA time zone used for cron evaluation.
    pub timezone: &'a str,
    /// Maximum task runtime.
    pub timeout_seconds: u64,
    /// Whether missed slots collapse to the latest slot.
    pub coalesce: bool,
    /// Whether the task is enabled.
    pub enabled: bool,
}

/// Borrowed optional values used to update one scheduled task.
#[derive(Debug, Clone, Copy)]
pub struct ScheduledTaskUpdateParams<'a> {
    /// Scheduled task row identifier.
    pub task_id: i64,
    /// Optional replacement task name.
    pub name: Option<&'a str>,
    /// Optional replacement job specification.
    pub job: Option<&'a ScheduledJobSpec>,
    /// Optional replacement cron expression.
    pub cron: Option<&'a str>,
    /// Optional replacement IANA time zone.
    pub timezone: Option<&'a str>,
    /// Optional replacement timeout.
    pub timeout_seconds: Option<u64>,
    /// Optional replacement coalescing flag.
    pub coalesce: Option<bool>,
    /// Optional replacement enabled flag.
    pub enabled: Option<bool>,
}

/// Durable scheduled run claimed for one worker execution.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledRunClaim {
    /// Run row identifier.
    pub run_id: i64,
    /// Scheduled UTC Unix timestamp.
    pub scheduled_for: i64,
    /// Claiming worker identifier.
    pub worker_id: String,
    /// Current task definition used for execution.
    pub task: ScheduledTaskInfo,
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

/// Create a folder for a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `codec` - Deployment secret codec.
/// * `user_id` - Owner user identifier.
/// * `name` - Trimmed folder name.
/// * `is_tracking` - Whether the folder becomes the tracking folder.
///
/// # Returns
///
/// Created folder response.
pub fn create_folder(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    name: &str,
    is_tracking: bool,
) -> Result<FolderResponse, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let now = now_seconds();
    if is_tracking {
        connection.execute(
            "UPDATE folders SET is_tracking = 0, updated_at = ?1 WHERE user_id = ?2",
            params![now, user_id.value()],
        )?;
    }
    match connection.execute(
        "INSERT INTO folders (user_id, name, is_tracking, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user_id.value(), name, is_tracking as i64, now, now],
    ) {
        Ok(_) => Ok(FolderResponse {
            id: connection.last_insert_rowid(),
            name: name.to_string(),
            is_tracking,
            article_count: 0,
            created_at: now,
        }),
        Err(error) if is_constraint_error(&error) => {
            Err(BusinessRepositoryError::DuplicateFolderName)
        }
        Err(error) => Err(error.into()),
    }
}

/// List user folders with favorite counts.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
///
/// # Returns
///
/// Folder responses ordered by creation time.
pub fn list_folders(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<Vec<FolderResponse>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT f.id, f.name, f.is_tracking, f.created_at, COUNT(fav.id) AS article_count \
         FROM folders f LEFT JOIN favorites fav ON fav.folder_id = f.id \
         WHERE f.user_id = ?1 GROUP BY f.id ORDER BY f.created_at",
    )?;
    let rows = statement.query_map([user_id.value()], folder_from_row)?;
    collect_rows(rows)
}

/// Rename a folder.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
/// * `name` - Replacement folder name.
///
/// # Returns
///
/// True when a row was updated.
pub fn rename_folder(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
    name: &str,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    match connection.execute(
        "UPDATE folders SET name = ?1, updated_at = ?2 WHERE id = ?3 AND user_id = ?4",
        params![name, now_seconds(), folder_id, user_id.value()],
    ) {
        Ok(count) => Ok(count > 0),
        Err(error) if is_constraint_error(&error) => {
            Err(BusinessRepositoryError::DuplicateFolderName)
        }
        Err(error) => Err(error.into()),
    }
}

/// Delete a folder.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_folder(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM folders WHERE id = ?1 AND user_id = ?2",
        params![folder_id, user_id.value()],
    )?;
    Ok(count > 0)
}

/// Return a user's current tracking folder.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
///
/// # Returns
///
/// Tracking folder or None.
pub fn get_tracking_folder(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<Option<FolderResponse>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT id, name, is_tracking, created_at, 0 AS article_count \
             FROM folders WHERE user_id = ?1 AND is_tracking = 1 LIMIT 1",
            [user_id.value()],
            folder_from_row,
        )
        .optional()
        .map_err(BusinessRepositoryError::from)
}

/// Set a user's tracking folder.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
///
/// # Returns
///
/// True when the target folder exists and was selected.
pub fn set_tracking_folder(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let target = connection
        .query_row(
            "SELECT id FROM folders WHERE id = ?1 AND user_id = ?2",
            params![folder_id, user_id.value()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if target.is_none() {
        return Ok(false);
    }
    let now = now_seconds();
    connection.execute(
        "UPDATE folders SET is_tracking = 0, updated_at = ?1 WHERE user_id = ?2",
        params![now, user_id.value()],
    )?;
    connection.execute(
        "UPDATE folders SET is_tracking = 1, updated_at = ?1 WHERE id = ?2 AND user_id = ?3",
        params![now, folder_id, user_id.value()],
    )?;
    Ok(true)
}

/// Add one favorite row.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
/// * `favorite` - Favorite payload to insert.
///
/// # Returns
///
/// Favorite row response.
pub fn add_favorite(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
    favorite: &FavoriteAdd,
) -> Result<FavoriteResponse, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    ensure_folder_exists(
        &connection,
        user_id,
        folder_id,
        BusinessRepositoryError::FolderNotFound,
    )?;
    let now = now_seconds();
    connection.execute(
        "INSERT OR IGNORE INTO favorites \
         (user_id, folder_id, article_id, db_name, note, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            user_id.value(),
            folder_id,
            favorite.article_id.value(),
            favorite.db_name,
            favorite.note,
            now
        ],
    )?;
    Ok(FavoriteResponse {
        id: connection.last_insert_rowid(),
        folder_id,
        article_id: favorite.article_id,
        db_name: favorite.db_name.clone(),
        note: favorite.note.clone(),
        created_at: now,
    })
}

/// Remove one favorite row.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
/// * `article_id` - Article identifier.
/// * `db_name` - Source database name.
///
/// # Returns
///
/// True when a row was deleted.
pub fn remove_favorite(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
    article_id: i64,
    db_name: &str,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
         AND article_id = ?3 AND db_name = ?4",
        params![user_id.value(), folder_id, article_id, db_name],
    )?;
    Ok(count > 0)
}

/// List favorites as enriched article payloads where index metadata is available.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Optional folder filter.
/// * `limit` - Maximum row count.
/// * `offset` - Offset row count.
///
/// # Returns
///
/// Favorite article responses.
pub fn list_favorite_articles(
    config: &StorageConfig,
    user_id: UserId,
    folder_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<Vec<FavoriteArticleResponse>, BusinessRepositoryError> {
    let favorites = list_favorites(config.auth_db_path(), user_id, folder_id, limit, offset)?;
    let metadata = load_favorite_metadata(config, &favorites);
    Ok(favorites
        .into_iter()
        .map(|favorite| {
            let key = (favorite.db_name.clone(), favorite.article_id.value());
            let mut response = FavoriteArticleResponse::from(favorite);
            if let Some(article_metadata) = metadata.get(&key) {
                response.journal_id = article_metadata.journal_id;
                response.issue_id = article_metadata.issue_id;
                response.title = article_metadata.title.clone();
                response.date = article_metadata.date.clone();
                response.authors = article_metadata.authors.clone();
                response.abstract_text = article_metadata.abstract_text.clone();
                response.doi = article_metadata.doi.clone();
                response.platform_id = article_metadata.platform_id.clone();
                response.permalink = article_metadata.permalink.clone();
                response.journal_title = article_metadata.journal_title.clone();
                response.open_access = article_metadata.open_access;
                response.in_press = article_metadata.in_press;
                response.volume = article_metadata.volume.clone();
                response.number = article_metadata.number.clone();
                response.issn = article_metadata.issn.clone();
                response.eissn = article_metadata.eissn.clone();
                response.full_text_file = article_metadata.full_text_file.clone();
            }
            response
        })
        .collect())
}

/// List favorite rows without index metadata.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Optional folder filter.
/// * `limit` - Maximum row count.
/// * `offset` - Offset row count.
///
/// # Returns
///
/// Favorite rows ordered by creation time descending.
pub fn list_favorites(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<Vec<FavoriteResponse>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    if let Some(folder_id) = folder_id {
        let mut statement = connection.prepare(
            "SELECT id, folder_id, article_id, db_name, note, created_at \
             FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
             ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
        )?;
        let rows = statement.query_map(
            params![user_id.value(), folder_id, limit, offset],
            favorite_from_row,
        )?;
        collect_rows(rows)
    } else {
        let mut statement = connection.prepare(
            "SELECT id, folder_id, article_id, db_name, note, created_at \
             FROM favorites WHERE user_id = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows =
            statement.query_map(params![user_id.value(), limit, offset], favorite_from_row)?;
        collect_rows(rows)
    }
}

/// Count favorites.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Optional folder filter.
///
/// # Returns
///
/// Favorite row count.
pub fn count_favorites(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: Option<i64>,
) -> Result<i64, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    if let Some(folder_id) = folder_id {
        Ok(connection.query_row(
            "SELECT COUNT(*) FROM favorites WHERE user_id = ?1 AND folder_id = ?2",
            params![user_id.value(), folder_id],
            |row| row.get(0),
        )?)
    } else {
        Ok(connection.query_row(
            "SELECT COUNT(*) FROM favorites WHERE user_id = ?1",
            [user_id.value()],
            |row| row.get(0),
        )?)
    }
}

/// Check favorite folder memberships for one article.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `article_id` - Article identifier.
/// * `db_name` - Source database name.
///
/// # Returns
///
/// Favorite check rows.
pub fn is_favorited(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    article_id: i64,
    db_name: &str,
) -> Result<Vec<FavoriteCheckResponse>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT fav.folder_id, f.name AS folder_name \
         FROM favorites fav JOIN folders f ON fav.folder_id = f.id \
         WHERE fav.user_id = ?1 AND fav.article_id = ?2 AND fav.db_name = ?3",
    )?;
    let rows = statement.query_map(params![user_id.value(), article_id, db_name], |row| {
        Ok(FavoriteCheckResponse {
            folder_id: row.get(0)?,
            folder_name: row.get(1)?,
        })
    })?;
    collect_rows(rows)
}

/// Batch check favorite folder memberships.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `article_ids` - Article identifiers to check.
/// * `db_name` - Source database name.
///
/// # Returns
///
/// Batch favorite response items in request order after Python-compatible de-duplication.
pub fn batch_is_favorited(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    article_ids: &[i64],
    db_name: &str,
) -> Result<Vec<FavoriteBatchCheckResponse>, BusinessRepositoryError> {
    let article_ids = normalize_article_ids(article_ids);
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let connection = open_business_connection(auth_db_path)?;
    let placeholders = repeat_placeholders(article_ids.len(), 3);
    let sql = format!(
        "SELECT fav.article_id, fav.folder_id, f.name AS folder_name \
         FROM favorites fav JOIN folders f ON fav.folder_id = f.id \
         WHERE fav.user_id = ?1 AND fav.db_name = ?2 AND fav.article_id IN ({placeholders}) \
         ORDER BY fav.article_id, fav.created_at"
    );
    let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(article_ids.len() + 2);
    values.push(&user_id.0);
    values.push(&db_name);
    for article_id in &article_ids {
        values.push(article_id);
    }
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(values.as_slice())?;
    let mut by_article: HashMap<i64, Vec<FavoriteCheckResponse>> = article_ids
        .iter()
        .copied()
        .map(|id| (id, Vec::new()))
        .collect();
    while let Some(row) = rows.next()? {
        let article_id = row.get::<_, i64>(0)?;
        by_article
            .entry(article_id)
            .or_default()
            .push(FavoriteCheckResponse {
                folder_id: row.get(1)?,
                folder_name: row.get(2)?,
            });
    }
    Ok(article_ids
        .into_iter()
        .map(|article_id| FavoriteBatchCheckResponse {
            article_id: ps_domain::ArticleId(article_id),
            folders: by_article.remove(&article_id).unwrap_or_default(),
        })
        .collect())
}

/// Bulk add favorites.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
/// * `articles` - Favorite add payloads.
///
/// # Returns
///
/// Inserted row count.
pub fn bulk_add_favorites(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
    articles: &[FavoriteAdd],
) -> Result<i64, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    ensure_folder_exists(
        &connection,
        user_id,
        folder_id,
        BusinessRepositoryError::FolderNotFound,
    )?;
    let now = now_seconds();
    let before = connection.total_changes();
    {
        let mut statement = connection.prepare(
            "INSERT OR IGNORE INTO favorites \
             (user_id, folder_id, article_id, db_name, note, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for article in articles {
            statement.execute(params![
                user_id.value(),
                folder_id,
                article.article_id.value(),
                article.db_name,
                article.note,
                now
            ])?;
        }
    }
    Ok((connection.total_changes() - before) as i64)
}

/// Bulk remove favorites.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
/// * `articles` - Favorite references.
///
/// # Returns
///
/// Deleted row count.
pub fn bulk_remove_favorites(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
    articles: &[FavoriteArticleRef],
) -> Result<i64, BusinessRepositoryError> {
    let normalized = normalize_favorite_articles(articles);
    if normalized.is_empty() {
        return Ok(0);
    }
    let connection = open_business_connection(auth_db_path)?;
    ensure_folder_exists(
        &connection,
        user_id,
        folder_id,
        BusinessRepositoryError::FolderNotFound,
    )?;
    let before = connection.total_changes();
    {
        let mut statement = connection.prepare(
            "DELETE FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
             AND article_id = ?3 AND db_name = ?4",
        )?;
        for (article_id, db_name) in normalized {
            statement.execute(params![user_id.value(), folder_id, article_id, db_name])?;
        }
    }
    Ok((connection.total_changes() - before) as i64)
}

/// Bulk move favorites.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `source_folder_id` - Source folder identifier.
/// * `target_folder_id` - Target folder identifier.
/// * `articles` - Favorite references.
///
/// # Returns
///
/// Removed source row count.
pub fn bulk_move_favorites(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    source_folder_id: i64,
    target_folder_id: i64,
    articles: &[FavoriteArticleRef],
) -> Result<i64, BusinessRepositoryError> {
    if source_folder_id == target_folder_id {
        return Err(BusinessRepositoryError::SourceAndTargetFoldersSame);
    }
    let normalized = normalize_favorite_articles(articles);
    if normalized.is_empty() {
        return Ok(0);
    }
    let mut connection = open_business_connection(auth_db_path)?;
    ensure_folder_exists(
        &connection,
        user_id,
        source_folder_id,
        BusinessRepositoryError::SourceFolderNotFound,
    )?;
    ensure_folder_exists(
        &connection,
        user_id,
        target_folder_id,
        BusinessRepositoryError::TargetFolderNotFound,
    )?;
    let transaction = connection.transaction()?;
    let now = now_seconds();
    {
        let mut insert = transaction.prepare(
            "INSERT OR IGNORE INTO favorites \
             (user_id, folder_id, article_id, db_name, note, created_at) \
             SELECT user_id, ?1, article_id, db_name, note, ?2 \
             FROM favorites WHERE user_id = ?3 AND folder_id = ?4 \
             AND article_id = ?5 AND db_name = ?6",
        )?;
        for (article_id, db_name) in &normalized {
            insert.execute(params![
                target_folder_id,
                now,
                user_id.value(),
                source_folder_id,
                article_id,
                db_name
            ])?;
        }
    }
    let before_delete = transaction.total_changes();
    {
        let mut delete = transaction.prepare(
            "DELETE FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
             AND article_id = ?3 AND db_name = ?4",
        )?;
        for (article_id, db_name) in normalized {
            delete.execute(params![
                user_id.value(),
                source_folder_id,
                article_id,
                db_name
            ])?;
        }
    }
    let deleted = transaction.total_changes() - before_delete;
    transaction.commit()?;
    Ok(deleted as i64)
}

/// Get notification settings for a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
///
/// # Returns
///
/// Notification settings or None.
pub fn get_notification_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<Option<NotificationSettings>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT id, user_id, keywords, directions, selected_databases, delivery_method, \
             pushplus_token, pushplus_template, pushplus_topic, pushplus_channel, \
             sync_to_tracking_folder, ai_base_url, ai_api_key, ai_model, ai_system_prompt, \
             ai_backup_base_url, ai_backup_api_key, ai_backup_model, ai_backup_system_prompt, \
             ai_retry_attempts, enabled, created_at, updated_at \
            FROM notification_settings WHERE user_id = ?1",
            [user_id.value()],
            |row| notification_settings_from_row(row, codec),
        )
        .optional()
        .map_err(BusinessRepositoryError::from)?
        .transpose()
}

/// List all enabled notification subscribers with tracking folder metadata.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
///
/// # Returns
///
/// Enabled subscriber settings ordered by user id.
pub fn list_notification_subscribers(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<NotificationSubscriberInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT ns.user_id, u.username, ns.keywords, ns.directions, ns.selected_databases, \
         ns.delivery_method, ns.pushplus_token, ns.pushplus_template, ns.pushplus_topic, \
         ns.pushplus_channel, ns.sync_to_tracking_folder, ns.ai_base_url, ns.ai_api_key, \
         ns.ai_model, ns.ai_system_prompt, ns.ai_backup_base_url, ns.ai_backup_api_key, \
         ns.ai_backup_model, ns.ai_backup_system_prompt, ns.ai_retry_attempts, \
         (SELECT id FROM folders f WHERE f.user_id = ns.user_id AND f.is_tracking = 1 LIMIT 1) \
             AS tracking_folder_id \
         FROM notification_settings ns JOIN users u ON u.id = ns.user_id \
         WHERE ns.enabled = 1 ORDER BY ns.user_id",
    )?;
    let rows = statement.query_map([], |row| notification_subscriber_from_row(row, codec))?;
    collect_nested_rows(rows)
}

/// Create or update notification settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `user_id` - Owner user identifier.
/// * `settings` - Normalized notification settings.
///
/// # Returns
///
/// Updated notification settings.
pub fn upsert_notification_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
    settings: &NotificationSettingsUpdate,
) -> Result<NotificationSettings, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let keywords = serde_json::to_string(&settings.keywords)?;
    let directions = serde_json::to_string(&settings.directions)?;
    let selected_databases = serde_json::to_string(&settings.selected_databases)?;
    let current_secrets = connection
        .query_row(
            "SELECT pushplus_token, ai_api_key, ai_backup_api_key \
             FROM notification_settings WHERE user_id = ?1",
            [user_id.value()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((pushplus_token, ai_api_key, ai_backup_api_key)) = current_secrets.as_ref() {
        codec.decrypt(
            pushplus_token,
            &notification_context(user_id.value(), "pushplus_token"),
        )?;
        codec.decrypt(
            ai_api_key,
            &notification_context(user_id.value(), "ai_api_key"),
        )?;
        codec.decrypt(
            ai_backup_api_key,
            &notification_context(user_id.value(), "ai_backup_api_key"),
        )?;
    }
    let pushplus_token = resolve_notification_secret(
        codec,
        user_id,
        "pushplus_token",
        &settings.pushplus_token,
        current_secrets.as_ref().map(|values| values.0.as_str()),
    )?;
    let ai_api_key = resolve_notification_secret(
        codec,
        user_id,
        "ai_api_key",
        &settings.ai_api_key,
        current_secrets.as_ref().map(|values| values.1.as_str()),
    )?;
    let ai_backup_api_key = resolve_notification_secret(
        codec,
        user_id,
        "ai_backup_api_key",
        &settings.ai_backup_api_key,
        current_secrets.as_ref().map(|values| values.2.as_str()),
    )?;
    connection.execute(
        "INSERT INTO notification_settings \
         (user_id, keywords, directions, selected_databases, delivery_method, \
          pushplus_token, pushplus_template, pushplus_topic, pushplus_channel, \
          sync_to_tracking_folder, ai_base_url, ai_api_key, ai_model, ai_system_prompt, \
          ai_backup_base_url, ai_backup_api_key, ai_backup_model, ai_backup_system_prompt, \
          ai_retry_attempts, enabled, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22) \
         ON CONFLICT(user_id) DO UPDATE SET \
          keywords = excluded.keywords, directions = excluded.directions, \
          selected_databases = excluded.selected_databases, delivery_method = excluded.delivery_method, \
          pushplus_token = excluded.pushplus_token, pushplus_template = excluded.pushplus_template, \
          pushplus_topic = excluded.pushplus_topic, pushplus_channel = excluded.pushplus_channel, \
          sync_to_tracking_folder = excluded.sync_to_tracking_folder, \
          ai_base_url = excluded.ai_base_url, ai_api_key = excluded.ai_api_key, \
          ai_model = excluded.ai_model, ai_system_prompt = excluded.ai_system_prompt, \
          ai_backup_base_url = excluded.ai_backup_base_url, ai_backup_api_key = excluded.ai_backup_api_key, \
          ai_backup_model = excluded.ai_backup_model, \
          ai_backup_system_prompt = excluded.ai_backup_system_prompt, \
          ai_retry_attempts = excluded.ai_retry_attempts, enabled = excluded.enabled, \
          updated_at = excluded.updated_at",
        params![
            user_id.value(),
            keywords,
            directions,
            selected_databases,
            settings.delivery_method,
            pushplus_token,
            settings.pushplus_template,
            settings.pushplus_topic,
            settings.pushplus_channel,
            settings.sync_to_tracking_folder as i64,
            settings.ai_base_url,
            ai_api_key,
            settings.ai_model,
            settings.ai_system_prompt,
            settings.ai_backup_base_url,
            ai_backup_api_key,
            settings.ai_backup_model,
            settings.ai_backup_system_prompt,
            settings.ai_retry_attempts,
            settings.enabled as i64,
            now,
            now
        ],
    )?;
    get_notification_settings(auth_db_path, codec, user_id)?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

fn resolve_notification_secret(
    codec: &SecretCodec,
    user_id: UserId,
    field: &str,
    update: &Option<Option<String>>,
    existing: Option<&str>,
) -> Result<String, BusinessRepositoryError> {
    match update {
        None => Ok(existing.unwrap_or_default().to_string()),
        Some(None) => Ok(String::new()),
        Some(Some(value)) if value.trim().is_empty() => {
            Ok(existing.unwrap_or_default().to_string())
        }
        Some(Some(value)) => codec
            .encrypt(value.trim(), &notification_context(user_id.value(), field))
            .map_err(BusinessRepositoryError::from),
    }
}

/// List all users with admin dashboard counts.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Admin user payloads.
pub fn list_all_users(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<AdminUserInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT u.id, u.username, u.is_admin, u.created_at, u.updated_at, \
         (SELECT COUNT(*) FROM folders f WHERE f.user_id = u.id) AS folder_count, \
         (SELECT COUNT(*) FROM favorites fv WHERE fv.user_id = u.id) AS favorite_count, \
         (SELECT COUNT(*) FROM notification_settings ns WHERE ns.user_id = u.id AND ns.enabled = 1) \
             AS notify_enabled \
         FROM users u ORDER BY u.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AdminUserInfo {
            id: UserId(row.get(0)?),
            username: row.get(1)?,
            is_admin: row.get::<_, i64>(2)? != 0,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            folder_count: row.get(5)?,
            favorite_count: row.get(6)?,
            notify_enabled: row.get::<_, i64>(7)? != 0,
        })
    })?;
    collect_rows(rows)
}

/// Set or revoke admin status.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Target user identifier.
/// * `is_admin` - Replacement admin flag.
///
/// # Returns
///
/// True when a row was updated.
pub fn set_user_admin(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    is_admin: bool,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "UPDATE users SET is_admin = ?1, updated_at = ?2 WHERE id = ?3",
        params![is_admin as i64, now_seconds(), user_id.value()],
    )?;
    Ok(count > 0)
}

/// Delete a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Target user identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_user(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute("DELETE FROM users WHERE id = ?1", [user_id.value()])?;
    Ok(count > 0)
}

/// List invite codes for the admin dashboard.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Invite code payloads.
pub fn list_all_invite_codes(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<AdminInviteCodeInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT ic.id, ic.code, ic.created_by, ic.used_by, ic.used_at, ic.created_at, \
         uc.username AS created_by_name, uu.username AS used_by_name \
         FROM invite_codes ic \
         LEFT JOIN users uc ON ic.created_by = uc.id \
         LEFT JOIN users uu ON ic.used_by = uu.id \
         ORDER BY ic.created_at DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AdminInviteCodeInfo {
            id: row.get(0)?,
            code: row.get(1)?,
            created_by: row.get::<_, Option<i64>>(2)?.map(UserId),
            used_by: row.get::<_, Option<i64>>(3)?.map(UserId),
            used_at: row.get(4)?,
            created_at: row.get(5)?,
            created_by_name: row.get(6)?,
            used_by_name: row.get(7)?,
        })
    })?;
    collect_rows(rows)
}

/// Create an admin-generated invite code.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Created invite code payload.
pub fn admin_create_invite_code(
    auth_db_path: impl AsRef<Path>,
) -> Result<AdminInviteCodeInfo, BusinessRepositoryError> {
    let code = random_hex(auth_db_path.as_ref(), ADMIN_INVITE_CODE_BYTES)
        .map_err(|error| BusinessRepositoryError::Sqlite(error.into_sqlite_error()))?;
    let connection = open_business_connection(auth_db_path)?;
    let now = now_seconds();
    connection.execute(
        "INSERT INTO invite_codes (code, created_by, created_at) VALUES (?1, NULL, ?2)",
        params![code, now],
    )?;
    Ok(AdminInviteCodeInfo {
        id: connection.last_insert_rowid(),
        code,
        created_by: None,
        created_by_name: None,
        used_by: None,
        used_by_name: None,
        used_at: None,
        created_at: now,
    })
}

/// Delete an unused invite code.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `code_id` - Invite code row identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_invite_code(
    auth_db_path: impl AsRef<Path>,
    code_id: i64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM invite_codes WHERE id = ?1 AND used_by IS NULL",
        [code_id],
    )?;
    Ok(count > 0)
}

/// Return aggregate admin stats.
///
/// # Arguments
///
/// * `config` - Storage path configuration.
///
/// # Returns
///
/// Admin stats payload.
pub fn get_admin_stats(
    config: &StorageConfig,
) -> Result<AdminStatsResponse, BusinessRepositoryError> {
    Ok(AdminStatsResponse {
        auth: get_auth_stats(config.auth_db_path())?,
        index: get_index_stats(config)?,
        push: get_push_stats(config)?,
    })
}

/// List managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Runtime setting payloads.
pub fn list_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let rows = read_runtime_setting_rows(&connection)?;
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            public_runtime_setting_from_definition(definition, rows.get(definition.field), codec)
        })
        .collect()
}

/// Load managed runtime settings for trusted backend consumers.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
///
/// # Returns
///
/// Effective values with secret fields decrypted in non-serializable types.
pub fn load_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSettingValue>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let rows = read_runtime_setting_rows(&connection)?;
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            internal_runtime_setting_from_definition(definition, rows.get(definition.field), codec)
        })
        .collect()
}

/// Upsert managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `values` - Values keyed by API field name; null clears secret fields.
///
/// # Returns
///
/// Updated runtime setting payloads.
pub fn upsert_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    values: &HashMap<String, Option<String>>,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = read_runtime_setting_rows(&transaction)?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?;
        for (field, update) in values {
            let definition = runtime_definition_by_field(field)
                .ok_or_else(|| BusinessRepositoryError::UnknownRuntimeSetting(field.clone()))?;
            let current = existing.get(field).map(|row| row.0.as_str());
            let mut value = if definition.is_secret {
                if let Some(stored) = current {
                    codec.decrypt(stored, &runtime_context(field))?;
                }
                match update {
                    None => String::new(),
                    Some(raw_value) if raw_value.trim().is_empty() => {
                        current.unwrap_or_default().to_string()
                    }
                    Some(raw_value) => codec.encrypt(raw_value.trim(), &runtime_context(field))?,
                }
            } else {
                update
                    .as_deref()
                    .ok_or_else(|| {
                        BusinessRepositoryError::NonSecretRuntimeSettingCannotBeCleared(
                            field.clone(),
                        )
                    })?
                    .trim()
                    .to_string()
            };
            if !definition.is_secret && definition.input_type == "boolean" {
                let default = definition.default_value.trim().eq_ignore_ascii_case("true");
                value = runtime_bool_to_text(&value, default)?;
            }
            statement.execute(params![definition.field, value, now])?;
        }
    }
    transaction.commit()?;
    list_runtime_settings(auth_db_path, codec)
}

/// List scheduled tasks.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Scheduled tasks ordered by creation time descending.
pub fn list_scheduled_tasks(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT id, name, job_spec, legacy_command, cron, timezone, timeout_seconds, coalesce, \
                enabled, last_run_at, last_status, created_at, updated_at \
         FROM scheduled_tasks ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map([], scheduled_task_from_row)?;
    collect_rows(rows)
}

fn validate_scheduled_job(job: &ScheduledJobSpec) -> Result<(), BusinessRepositoryError> {
    job.validate()
        .map_err(|error| BusinessRepositoryError::InvalidScheduledJob(error.to_string()))
}

fn validate_scheduled_timing(
    timezone: &str,
    timeout_seconds: u64,
) -> Result<(), BusinessRepositoryError> {
    validate_scheduled_task_timing(timezone, timeout_seconds)
        .map_err(|error| BusinessRepositoryError::InvalidScheduledTask(error.to_string()))
}

/// Get one scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
///
/// # Returns
///
/// Scheduled task payload when it exists.
pub fn get_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    get_scheduled_task_from_connection(&connection, task_id)
}

/// Create a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task` - Validated task creation values.
///
/// # Returns
///
/// Created task payload.
pub fn create_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task: ScheduledTaskCreateParams<'_>,
) -> Result<ScheduledTaskInfo, BusinessRepositoryError> {
    validate_scheduled_job(task.job)?;
    validate_scheduled_timing(task.timezone, task.timeout_seconds)?;
    let connection = open_business_connection(auth_db_path)?;
    let now = now_seconds();
    let job_spec = serde_json::to_string(task.job)?;
    connection.execute(
        "INSERT INTO scheduled_tasks \
         (name, job_spec, legacy_command, cron, timezone, timeout_seconds, coalesce, enabled, \
          last_run_at, last_status, created_at, updated_at) \
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, NULL, '', ?8, ?9)",
        params![
            task.name,
            job_spec,
            task.cron,
            task.timezone,
            task.timeout_seconds,
            task.coalesce as i64,
            task.enabled as i64,
            now,
            now
        ],
    )?;
    get_scheduled_task_from_connection(&connection, connection.last_insert_rowid())?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

/// Update a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task` - Scheduled task identifier and optional replacement values.
///
/// # Returns
///
/// Updated task payload or None.
pub fn update_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task: ScheduledTaskUpdateParams<'_>,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let Some(current) = get_scheduled_task_from_connection(&connection, task.task_id)? else {
        return Ok(None);
    };
    let next_job = task.job.or(current.job.as_ref());
    if let Some(next_job) = next_job {
        validate_scheduled_job(next_job)?;
    }
    let next_timezone = task.timezone.unwrap_or(&current.timezone);
    let next_timeout_seconds = task.timeout_seconds.unwrap_or(current.timeout_seconds);
    validate_scheduled_timing(next_timezone, next_timeout_seconds)?;
    let next_enabled = task.enabled.unwrap_or(current.enabled);
    if next_job.is_none() && next_enabled {
        return Err(BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled);
    }
    let job_spec = next_job.map(serde_json::to_string).transpose()?;
    let legacy_command = if task.job.is_some() {
        None
    } else {
        current.legacy_command.as_deref()
    };
    connection.execute(
        "UPDATE scheduled_tasks SET name = ?1, job_spec = ?2, legacy_command = ?3, cron = ?4, \
         timezone = ?5, timeout_seconds = ?6, coalesce = ?7, enabled = ?8, \
         updated_at = ?9 WHERE id = ?10",
        params![
            task.name.unwrap_or(&current.name),
            job_spec,
            legacy_command,
            task.cron.unwrap_or(&current.cron),
            next_timezone,
            next_timeout_seconds,
            task.coalesce.unwrap_or(current.coalesce) as i64,
            next_enabled as i64,
            now_seconds(),
            task.task_id
        ],
    )?;
    get_scheduled_task_from_connection(&connection, task.task_id)
}

/// Delete a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute("DELETE FROM scheduled_tasks WHERE id = ?1", [task_id])?;
    Ok(count > 0)
}

/// Record one scheduled task run result.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
/// * `status` - Python-compatible status string.
/// * `ran_at` - Unix timestamp when the job started.
///
/// # Returns
///
/// True when a task row was updated.
pub fn record_scheduled_task_run(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
    status: &str,
    ran_at: f64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "UPDATE scheduled_tasks SET last_run_at = ?1, last_status = ?2, \
         updated_at = ?3 WHERE id = ?4",
        rusqlite::params![ran_at, status, now_seconds(), task_id],
    )?;
    Ok(count > 0)
}

/// Read the persisted scheduler cursor.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Last completed scheduler check, or None before the first check.
pub fn get_scheduler_last_checked_at(
    auth_db_path: impl AsRef<Path>,
) -> Result<Option<f64>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT last_checked_at FROM scheduler_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(BusinessRepositoryError::from)
}

/// Advance the scheduler cursor without allowing time to move backward.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `checked_at` - Completed wall-clock check timestamp.
///
/// # Returns
///
/// Empty result after the cursor is persisted.
pub fn record_scheduler_check(
    auth_db_path: impl AsRef<Path>,
    checked_at: f64,
) -> Result<(), BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection.execute(
        "UPDATE scheduler_state
         SET last_checked_at = CASE
             WHEN last_checked_at IS NULL OR last_checked_at < ?1 THEN ?1
             ELSE last_checked_at
         END
         WHERE id = 1",
        [checked_at],
    )?;
    Ok(())
}

/// Persist one scheduler worker heartbeat.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `worker_id` - Stable worker process identifier.
/// * `heartbeat_at` - Current Unix timestamp.
///
/// # Returns
///
/// Empty result after the heartbeat is persisted.
pub fn record_scheduler_heartbeat(
    auth_db_path: impl AsRef<Path>,
    worker_id: &str,
    heartbeat_at: f64,
) -> Result<(), BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection.execute(
        "INSERT INTO scheduler_workers (worker_id, started_at, heartbeat_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(worker_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at, heartbeat_at],
    )?;
    connection.execute(
        "INSERT INTO service_heartbeats (service, instance_id, started_at, heartbeat_at)
         VALUES ('worker', ?1, ?2, ?2)
         ON CONFLICT(service, instance_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at],
    )?;
    connection.execute(
        "DELETE FROM scheduler_workers WHERE worker_id <> ?1 AND heartbeat_at < ?2",
        params![worker_id, heartbeat_at - 604_800.0],
    )?;
    Ok(())
}

/// Queue durable scheduled slots for one task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task` - Current scheduled task definition.
/// * `scheduled_slots` - UTC Unix minute timestamps.
///
/// # Returns
///
/// Number of newly inserted run rows.
pub fn enqueue_scheduled_runs(
    auth_db_path: impl AsRef<Path>,
    task: &ScheduledTaskInfo,
    scheduled_slots: &[i64],
) -> Result<usize, BusinessRepositoryError> {
    if scheduled_slots.is_empty() {
        return Ok(0);
    }
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if task.coalesce {
        let latest_slot = scheduled_slots[scheduled_slots.len() - 1];
        let latest_pending = transaction.query_row(
            "SELECT MAX(scheduled_for) FROM scheduled_task_runs
             WHERE task_id = ?1 AND status = 'pending'",
            [task.id],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        let selected_slot = latest_pending.map_or(latest_slot, |pending| pending.max(latest_slot));
        transaction.execute(
            "DELETE FROM scheduled_task_runs
             WHERE task_id = ?1 AND status = 'pending' AND scheduled_for < ?2",
            params![task.id, selected_slot],
        )?;
        let inserted = if selected_slot == latest_slot {
            transaction.execute(
                "INSERT OR IGNORE INTO scheduled_task_runs
                 (task_id, task_name, scheduled_for, status)
                 VALUES (?1, ?2, ?3, 'pending')",
                params![task.id, task.name, selected_slot],
            )?
        } else {
            0
        };
        transaction.commit()?;
        return Ok(inserted);
    }
    let mut inserted = 0;
    for scheduled_for in scheduled_slots {
        inserted += transaction.execute(
            "INSERT OR IGNORE INTO scheduled_task_runs
             (task_id, task_name, scheduled_for, status)
             VALUES (?1, ?2, ?3, 'pending')",
            params![task.id, task.name, scheduled_for],
        )?;
    }
    transaction.commit()?;
    Ok(inserted)
}

/// Reconcile stale runs and claim one pending run per available task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `worker_id` - Claiming worker identifier.
/// * `claimed_at` - Deterministic claim timestamp.
/// * `lease_seconds` - Claim lease duration.
///
/// # Returns
///
/// Claims owned by the requesting worker.
pub fn claim_ready_scheduled_runs(
    auth_db_path: impl AsRef<Path>,
    worker_id: &str,
    claimed_at: f64,
    lease_seconds: f64,
) -> Result<Vec<ScheduledRunClaim>, BusinessRepositoryError> {
    let auth_db_path = auth_db_path.as_ref();
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "UPDATE scheduled_task_runs
         SET status = 'unknown', finished_at = ?1, claim_expires_at = NULL
         WHERE status = 'running' AND claim_expires_at <= ?1",
        [claimed_at],
    )?;
    transaction.execute(
        "UPDATE scheduled_tasks
         SET last_run_at = ?1, last_status = 'unknown', updated_at = ?1
         WHERE id IN (
             SELECT task_id FROM scheduled_task_runs
             WHERE status = 'unknown' AND finished_at = ?1
         )",
        [claimed_at],
    )?;
    transaction.execute(
        "UPDATE scheduled_task_runs
         SET status = 'pending', worker_id = NULL, claim_expires_at = NULL,
             claimed_at = NULL
         WHERE status = 'claimed' AND claim_expires_at <= ?1",
        [claimed_at],
    )?;

    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT run.id, run.task_id, run.scheduled_for
             FROM scheduled_task_runs AS run
             JOIN scheduled_tasks AS task ON task.id = run.task_id
             WHERE run.status = 'pending'
               AND task.enabled = 1
               AND task.job_spec IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM scheduled_task_runs AS active
                   WHERE active.task_id = run.task_id
                     AND active.status IN ('claimed', 'running')
               )
             ORDER BY run.scheduled_for, run.id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut claimed_rows = Vec::new();
    for (run_id, task_id, scheduled_for) in candidates {
        let count = transaction.execute(
            "UPDATE scheduled_task_runs
             SET status = 'claimed', worker_id = ?1, claimed_at = ?2,
                 claim_expires_at = ?3
             WHERE id = ?4 AND status = 'pending'
               AND NOT EXISTS (
                   SELECT 1 FROM scheduled_task_runs AS active
                   WHERE active.task_id = ?5
                     AND active.status IN ('claimed', 'running')
               )",
            params![
                worker_id,
                claimed_at,
                claimed_at + lease_seconds,
                run_id,
                task_id
            ],
        )?;
        if count > 0 {
            claimed_rows.push((run_id, task_id, scheduled_for));
        }
    }
    transaction.commit()?;

    let mut claims = Vec::new();
    for (run_id, task_id, scheduled_for) in claimed_rows {
        let Some(task) = get_scheduled_task(auth_db_path, task_id)? else {
            fail_unexecutable_claim(auth_db_path, run_id, worker_id, claimed_at)?;
            continue;
        };
        if !task.enabled || task.job.is_none() {
            fail_unexecutable_claim(auth_db_path, run_id, worker_id, claimed_at)?;
            continue;
        }
        claims.push(ScheduledRunClaim {
            run_id,
            scheduled_for,
            worker_id: worker_id.to_string(),
            task,
        });
    }
    Ok(claims)
}

/// Mark a claimed scheduled run as started.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `run_id` - Run row identifier.
/// * `worker_id` - Owning worker identifier.
/// * `started_at` - Execution start timestamp.
/// * `lease_seconds` - Running lease duration.
///
/// # Returns
///
/// True when the owning claim transitioned to running.
pub fn start_scheduled_run(
    auth_db_path: impl AsRef<Path>,
    run_id: i64,
    worker_id: &str,
    started_at: f64,
    lease_seconds: f64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "UPDATE scheduled_task_runs
         SET status = 'running', started_at = ?1, claim_expires_at = ?2
         WHERE id = ?3 AND worker_id = ?4 AND status = 'claimed'",
        params![started_at, started_at + lease_seconds, run_id, worker_id],
    )?;
    Ok(count > 0)
}

/// Renew a running claim and worker heartbeat together.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `run_id` - Run row identifier.
/// * `worker_id` - Owning worker identifier.
/// * `heartbeat_at` - Current Unix timestamp.
/// * `lease_seconds` - Running lease duration.
///
/// # Returns
///
/// True when the run lease was renewed.
pub fn heartbeat_scheduled_run(
    auth_db_path: impl AsRef<Path>,
    run_id: i64,
    worker_id: &str,
    heartbeat_at: f64,
    lease_seconds: f64,
) -> Result<bool, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "INSERT INTO scheduler_workers (worker_id, started_at, heartbeat_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(worker_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at, heartbeat_at],
    )?;
    transaction.execute(
        "INSERT INTO service_heartbeats (service, instance_id, started_at, heartbeat_at)
         VALUES ('worker', ?1, ?2, ?2)
         ON CONFLICT(service, instance_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at],
    )?;
    let count = transaction.execute(
        "UPDATE scheduled_task_runs SET claim_expires_at = ?1
         WHERE id = ?2 AND worker_id = ?3 AND status = 'running'",
        params![heartbeat_at + lease_seconds, run_id, worker_id],
    )?;
    transaction.commit()?;
    Ok(count > 0)
}

/// Finish one claimed or running scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `claim` - Durable run claim.
/// * `status` - Terminal run status.
/// * `output_summary` - Bounded internal output summary.
/// * `finished_at` - Completion timestamp.
///
/// # Returns
///
/// True when the owning run was finalized.
pub fn finish_scheduled_run(
    auth_db_path: impl AsRef<Path>,
    claim: &ScheduledRunClaim,
    status: &str,
    output_summary: &str,
    finished_at: f64,
) -> Result<bool, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count = transaction.execute(
        "UPDATE scheduled_task_runs
         SET status = ?1, finished_at = ?2, claim_expires_at = NULL,
             output_summary = ?3
         WHERE id = ?4 AND worker_id = ?5 AND status IN ('claimed', 'running')",
        params![
            status,
            finished_at,
            output_summary,
            claim.run_id,
            claim.worker_id
        ],
    )?;
    if count > 0 {
        transaction.execute(
            "UPDATE scheduled_tasks
             SET last_run_at = ?1, last_status = ?2, updated_at = ?1
             WHERE id = ?3",
            params![finished_at, status, claim.task.id],
        )?;
    }
    transaction.commit()?;
    Ok(count > 0)
}

/// Read scheduler cursor, worker heartbeats, and recent run statuses.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `current_time` - Current Unix timestamp used for health classification.
/// * `healthy_window_seconds` - Maximum healthy heartbeat age.
/// * `run_limit` - Maximum recent run count.
///
/// # Returns
///
/// Administrator scheduler status payload.
pub fn get_scheduler_status(
    auth_db_path: impl AsRef<Path>,
    current_time: f64,
    healthy_window_seconds: f64,
    run_limit: usize,
) -> Result<SchedulerStatusResponse, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let last_checked_at = connection.query_row(
        "SELECT last_checked_at FROM scheduler_state WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let workers = {
        let mut statement = connection.prepare(
            "SELECT worker_id, started_at, heartbeat_at
             FROM scheduler_workers ORDER BY heartbeat_at DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                let heartbeat_at = row.get::<_, f64>(2)?;
                Ok(SchedulerWorkerInfo {
                    worker_id: row.get(0)?,
                    started_at: row.get(1)?,
                    heartbeat_at,
                    is_healthy: heartbeat_at >= current_time - healthy_window_seconds,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let recent_runs = {
        let mut statement = connection.prepare(
            "SELECT id, task_id, task_name, scheduled_for, status, worker_id,
                    claimed_at, started_at, finished_at
             FROM scheduled_task_runs ORDER BY scheduled_for DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([run_limit as i64], scheduled_task_run_from_row)?;
        collect_rows(rows)?
    };
    Ok(SchedulerStatusResponse {
        last_checked_at,
        workers,
        recent_runs,
    })
}

fn fail_unexecutable_claim(
    auth_db_path: &Path,
    run_id: i64,
    worker_id: &str,
    finished_at: f64,
) -> Result<(), BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection.execute(
        "UPDATE scheduled_task_runs
         SET status = 'error', finished_at = ?1, claim_expires_at = NULL,
             output_summary = 'Task is no longer executable'
         WHERE id = ?2 AND worker_id = ?3 AND status = 'claimed'",
        params![finished_at, run_id, worker_id],
    )?;
    Ok(())
}

/// List all announcements for admin management.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Announcement payloads ordered by creation time descending.
pub fn list_all_announcements(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<AnnouncementInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT id, title, message, priority, enabled, created_at, updated_at \
         FROM announcements ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map([], announcement_from_row)?;
    collect_rows(rows)
}

/// Get one announcement.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `announcement_id` - Announcement row identifier.
///
/// # Returns
///
/// Announcement payload or None.
pub fn get_announcement(
    auth_db_path: impl AsRef<Path>,
    announcement_id: i64,
) -> Result<Option<AnnouncementInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    get_announcement_from_connection(&connection, announcement_id)
}

/// Create an announcement.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `title` - Announcement title.
/// * `message` - Announcement message.
/// * `priority` - Priority label.
/// * `enabled` - Whether the announcement is visible.
///
/// # Returns
///
/// Created announcement payload.
pub fn create_announcement(
    auth_db_path: impl AsRef<Path>,
    title: &str,
    message: &str,
    priority: &str,
    enabled: bool,
) -> Result<AnnouncementInfo, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let now = now_seconds();
    connection.execute(
        "INSERT INTO announcements (title, message, priority, enabled, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![title, message, priority, enabled as i64, now, now],
    )?;
    get_announcement_from_connection(&connection, connection.last_insert_rowid())?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

/// Update an announcement.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `announcement_id` - Announcement row identifier.
/// * `title` - Optional replacement title.
/// * `message` - Optional replacement message.
/// * `priority` - Optional replacement priority.
/// * `enabled` - Optional enabled flag.
///
/// # Returns
///
/// Updated announcement payload or None.
pub fn update_announcement(
    auth_db_path: impl AsRef<Path>,
    announcement_id: i64,
    title: Option<&str>,
    message: Option<&str>,
    priority: Option<&str>,
    enabled: Option<bool>,
) -> Result<Option<AnnouncementInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let Some(current) = get_announcement_from_connection(&connection, announcement_id)? else {
        return Ok(None);
    };
    connection.execute(
        "UPDATE announcements SET title = ?1, message = ?2, priority = ?3, enabled = ?4, \
         updated_at = ?5 WHERE id = ?6",
        params![
            title.unwrap_or(&current.title),
            message.unwrap_or(&current.message),
            priority.unwrap_or(&current.priority),
            enabled.unwrap_or(current.enabled) as i64,
            now_seconds(),
            announcement_id
        ],
    )?;
    get_announcement_from_connection(&connection, announcement_id)
}

/// Delete an announcement.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `announcement_id` - Announcement row identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_announcement(
    auth_db_path: impl AsRef<Path>,
    announcement_id: i64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute("DELETE FROM announcements WHERE id = ?1", [announcement_id])?;
    Ok(count > 0)
}

/// Normalize database names using Python-compatible filename semantics.
///
/// # Arguments
///
/// * `db_names` - Raw database names.
///
/// # Returns
///
/// Normalized `.sqlite` filenames in first-seen order.
pub fn normalize_database_names(db_names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for db_name in db_names {
        let Some(filename) = Path::new(db_name.trim())
            .file_name()
            .and_then(|value| value.to_str())
        else {
            continue;
        };
        if filename.is_empty() {
            continue;
        }
        let candidate = if filename.ends_with(".sqlite") {
            filename.to_string()
        } else {
            format!("{filename}.sqlite")
        };
        if seen.insert(candidate.clone()) {
            normalized.push(candidate);
        }
    }
    normalized
}

/// List available index database filenames.
///
/// # Arguments
///
/// * `config` - Storage path configuration.
///
/// # Returns
///
/// Sorted database filenames.
pub fn list_available_database_names(
    config: &StorageConfig,
) -> Result<Vec<String>, BusinessRepositoryError> {
    Ok(config
        .list_index_databases()
        .map_err(|error| BusinessRepositoryError::Io(std::io::Error::other(error)))?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .collect())
}

/// Count weekly article ids from push-state change manifests.
///
/// # Arguments
///
/// * `config` - Storage path configuration.
/// * `selected_databases` - Normalized selected database names; empty means all.
///
/// # Returns
///
/// Number of unique weekly article/database pairs.
pub fn count_weekly_articles(
    config: &StorageConfig,
    selected_databases: &[String],
) -> Result<usize, BusinessRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(0);
    }
    let mut seen = HashSet::new();
    for entry in fs::read_dir(push_state_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json")
            || !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".changes.json"))
        {
            continue;
        }
        let manifest = read_weekly_article_count_manifest(&path)?;
        let Some(db_name) = manifest
            .db_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if !is_database_selected(selected_databases, &db_name) {
            continue;
        }
        for article_id in manifest
            .notifiable_article_ids
            .into_iter()
            .chain(manifest.backfill_article_ids)
        {
            seen.insert((db_name.clone(), article_id));
        }
    }
    Ok(seen.len())
}

#[derive(Debug, Deserialize)]
struct WeeklyArticleCountManifest {
    db_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    notifiable_article_ids: Vec<i64>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    backfill_article_ids: Vec<i64>,
}

fn read_weekly_article_count_manifest(
    path: &Path,
) -> Result<WeeklyArticleCountManifest, BusinessRepositoryError> {
    let reader = std::io::BufReader::new(fs::File::open(path)?);
    Ok(serde_json::from_reader(reader)?)
}

fn deserialize_json_i64_list<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(Value::as_i64).collect())
}

fn get_auth_stats(auth_db_path: impl AsRef<Path>) -> Result<AuthStats, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let current_time = now_seconds();
    connection.execute(
        "DELETE FROM access_tokens WHERE expires_at <= ?1",
        [current_time],
    )?;
    let total_users = count_table(&connection, "users", None)?;
    let admin_count = count_table(&connection, "users", Some("is_admin = 1"))?;
    let total_folders = count_table(&connection, "folders", None)?;
    let total_favorites = count_table(&connection, "favorites", None)?;
    let total_invite_codes = count_table(&connection, "invite_codes", None)?;
    let used_invite_codes = count_table(&connection, "invite_codes", Some("used_by IS NOT NULL"))?;
    Ok(AuthStats {
        total_users,
        admin_count,
        total_folders,
        total_favorites,
        total_invite_codes,
        used_invite_codes,
        unused_invite_codes: total_invite_codes - used_invite_codes,
        active_tokens: connection.query_row(
            "SELECT COUNT(*) FROM access_tokens WHERE expires_at > ?1",
            [current_time],
            |row| row.get(0),
        )?,
        notification_subscribers: count_table(
            &connection,
            "notification_settings",
            Some("enabled = 1"),
        )?,
        scheduled_tasks: count_table(&connection, "scheduled_tasks", None)?,
        active_announcements: count_table(&connection, "announcements", Some("enabled = 1"))?,
    })
}

fn get_index_stats(config: &StorageConfig) -> Result<IndexStats, BusinessRepositoryError> {
    let mut databases = Vec::new();
    let mut total_articles = 0;
    let mut total_journals = 0;
    for path in config
        .list_index_databases()
        .map_err(|error| BusinessRepositoryError::Io(std::io::Error::other(error)))?
    {
        match index_database_stats(&path) {
            Ok(stats) => {
                total_articles += stats.articles;
                total_journals += stats.journals;
                databases.push(stats);
            }
            Err(_) => databases.push(IndexDatabaseStats {
                db_name: filename_string(&path),
                articles: 0,
                journals: 0,
                issues: 0,
                error: Some(true),
            }),
        }
    }
    Ok(IndexStats {
        databases,
        total_articles,
        total_journals,
    })
}

fn get_push_stats(config: &StorageConfig) -> Result<Vec<PushStats>, BusinessRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(push_state_dir)? {
        let path = entry?.path();
        if is_push_state_run_file(&path) {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|path| match read_json_file(&path) {
            Ok(value) => {
                let run = value.get("run").and_then(Value::as_object);
                PushStats {
                    db_name: path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    status: value
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    last_completed: value
                        .get("last_completed_run_at")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    delivered_count: run
                        .and_then(|items| items.get("delivered_article_ids"))
                        .and_then(Value::as_array)
                        .map(Vec::len),
                    user_results: run
                        .and_then(|items| items.get("user_results"))
                        .and_then(Value::as_array)
                        .map(Vec::len),
                }
            }
            Err(_) => PushStats {
                db_name: path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string(),
                status: "error".to_string(),
                last_completed: None,
                delivered_count: None,
                user_results: None,
            },
        })
        .collect())
}

fn is_push_state_run_file(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("json")
        && !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".changes.json"))
}

fn index_database_stats(path: &Path) -> Result<IndexDatabaseStats, BusinessRepositoryError> {
    let connection = Connection::open(path)?;
    let articles = connection.query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))?;
    let journals = connection.query_row("SELECT COUNT(*) FROM journals", [], |row| row.get(0))?;
    let issues = connection
        .query_row("SELECT COUNT(*) FROM issues", [], |row| row.get(0))
        .unwrap_or(0);
    Ok(IndexDatabaseStats {
        db_name: filename_string(path),
        articles,
        journals,
        issues,
        error: None,
    })
}

fn load_favorite_metadata(
    config: &StorageConfig,
    favorites: &[FavoriteResponse],
) -> HashMap<(String, i64), FavoriteArticleResponse> {
    let mut by_db: HashMap<String, Vec<i64>> = HashMap::new();
    for favorite in favorites {
        by_db
            .entry(favorite.db_name.clone())
            .or_default()
            .push(favorite.article_id.value());
    }
    let mut result = HashMap::new();
    for (db_name, article_ids) in by_db {
        let Ok(db_path) = config.resolve_index_db_path((!db_name.is_empty()).then_some(&db_name))
        else {
            continue;
        };
        let Ok(items) = load_metadata_from_index(&db_path, &db_name, &article_ids) else {
            continue;
        };
        result.extend(items);
    }
    result
}

fn load_metadata_from_index(
    db_path: &Path,
    db_name: &str,
    article_ids: &[i64],
) -> Result<HashMap<(String, i64), FavoriteArticleResponse>, BusinessRepositoryError> {
    let unique_ids = normalize_article_ids(article_ids);
    if unique_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = repeat_placeholders(unique_ids.len(), 1);
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.date, a.authors, \
         a.abstract, a.doi, a.platform_id, a.open_access, a.in_press, a.permalink, \
         a.full_text_file, j.title AS journal_title, j.issn, j.eissn, i.volume, i.number \
         FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
         JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.article_id IN ({placeholders})"
    );
    let connection = Connection::open(db_path)?;
    let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(unique_ids.len());
    for article_id in &unique_ids {
        values.push(article_id);
    }
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(values.as_slice(), |row| {
        let article_id = row.get::<_, i64>(0)?;
        Ok((
            (db_name.to_string(), article_id),
            FavoriteArticleResponse {
                id: 0,
                folder_id: 0,
                article_id: ps_domain::ArticleId(article_id),
                db_name: db_name.to_string(),
                note: String::new(),
                created_at: 0.0,
                journal_id: row.get::<_, Option<i64>>(1)?.map(ps_domain::JournalId),
                issue_id: row.get(2)?,
                title: row.get(3)?,
                date: row.get(4)?,
                authors: row.get(5)?,
                abstract_text: row.get(6)?,
                doi: row.get(7)?,
                platform_id: row.get(8)?,
                open_access: row.get(9)?,
                in_press: row.get(10)?,
                permalink: row.get(11)?,
                full_text_file: row.get(12)?,
                journal_title: row.get(13)?,
                issn: row.get(14)?,
                eissn: row.get(15)?,
                volume: row.get(16)?,
                number: row.get(17)?,
            },
        ))
    })?;
    collect_rows(rows)
        .map(|items: Vec<((String, i64), FavoriteArticleResponse)>| items.into_iter().collect())
}

fn open_business_connection(path: impl AsRef<Path>) -> Result<Connection, BusinessRepositoryError> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    Ok(open_sqlite_connection(path)?)
}

fn ensure_folder_exists(
    connection: &Connection,
    user_id: UserId,
    folder_id: i64,
    error: BusinessRepositoryError,
) -> Result<(), BusinessRepositoryError> {
    let folder = connection
        .query_row(
            "SELECT id FROM folders WHERE id = ?1 AND user_id = ?2",
            params![folder_id, user_id.value()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if folder.is_some() {
        Ok(())
    } else {
        Err(error)
    }
}

fn notification_settings_from_row(
    row: &rusqlite::Row<'_>,
    codec: &SecretCodec,
) -> rusqlite::Result<Result<NotificationSettings, BusinessRepositoryError>> {
    Ok((|| {
        let user_id = UserId(row.get(1)?);
        Ok(NotificationSettings {
            id: row.get(0)?,
            user_id,
            keywords: parse_string_list(row.get::<_, String>(2)?),
            directions: parse_string_list(row.get::<_, String>(3)?),
            selected_databases: parse_string_list(row.get::<_, String>(4)?),
            delivery_method: row.get(5)?,
            pushplus_token: codec.decrypt(
                &row.get::<_, String>(6)?,
                &notification_context(user_id.value(), "pushplus_token"),
            )?,
            pushplus_template: row.get(7)?,
            pushplus_topic: row.get(8)?,
            pushplus_channel: row.get(9)?,
            sync_to_tracking_folder: row.get::<_, i64>(10)? != 0,
            ai_base_url: row.get(11)?,
            ai_api_key: codec.decrypt(
                &row.get::<_, String>(12)?,
                &notification_context(user_id.value(), "ai_api_key"),
            )?,
            ai_model: row.get(13)?,
            ai_system_prompt: row.get(14)?,
            ai_backup_base_url: row.get(15)?,
            ai_backup_api_key: codec.decrypt(
                &row.get::<_, String>(16)?,
                &notification_context(user_id.value(), "ai_backup_api_key"),
            )?,
            ai_backup_model: row.get(17)?,
            ai_backup_system_prompt: row.get(18)?,
            ai_retry_attempts: row.get::<_, i64>(19)?.max(1),
            enabled: row.get::<_, i64>(20)? != 0,
            created_at: row.get(21)?,
            updated_at: row.get(22)?,
        })
    })())
}

fn notification_subscriber_from_row(
    row: &rusqlite::Row<'_>,
    codec: &SecretCodec,
) -> rusqlite::Result<Result<NotificationSubscriberInfo, BusinessRepositoryError>> {
    let user_id = row.get::<_, i64>(0)?;
    Ok((|| {
        Ok(NotificationSubscriberInfo {
            subscriber_id: user_id.to_string(),
            user_id,
            name: row.get(1)?,
            keywords: parse_string_list(row.get::<_, String>(2)?),
            directions: parse_string_list(row.get::<_, String>(3)?),
            selected_databases: parse_string_list(row.get::<_, String>(4)?),
            delivery_method: row.get(5)?,
            pushplus_token: codec.decrypt(
                &row.get::<_, String>(6)?,
                &notification_context(user_id, "pushplus_token"),
            )?,
            template: optional_trimmed(row.get::<_, String>(7)?),
            topic: optional_trimmed(row.get::<_, String>(8)?),
            channel: optional_trimmed(row.get::<_, String>(9)?),
            sync_to_tracking_folder: row.get::<_, i64>(10)? != 0,
            ai_base_url: optional_trimmed(row.get::<_, String>(11)?),
            ai_api_key: optional_trimmed(codec.decrypt(
                &row.get::<_, String>(12)?,
                &notification_context(user_id, "ai_api_key"),
            )?),
            ai_model: optional_trimmed(row.get::<_, String>(13)?),
            ai_system_prompt: optional_trimmed(row.get::<_, String>(14)?),
            ai_backup_base_url: optional_trimmed(row.get::<_, String>(15)?),
            ai_backup_api_key: optional_trimmed(codec.decrypt(
                &row.get::<_, String>(16)?,
                &notification_context(user_id, "ai_backup_api_key"),
            )?),
            ai_backup_model: optional_trimmed(row.get::<_, String>(17)?),
            ai_backup_system_prompt: optional_trimmed(row.get::<_, String>(18)?),
            ai_retry_attempts: row.get::<_, i64>(19)?.max(1),
            tracking_folder_id: row.get(20)?,
        })
    })())
}

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FolderResponse> {
    Ok(FolderResponse {
        id: row.get(0)?,
        name: row.get(1)?,
        is_tracking: row.get::<_, i64>(2)? != 0,
        created_at: row.get(3)?,
        article_count: row.get(4)?,
    })
}

fn favorite_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FavoriteResponse> {
    Ok(FavoriteResponse {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        article_id: ps_domain::ArticleId(row.get(2)?),
        db_name: row.get(3)?,
        note: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn scheduled_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTaskInfo> {
    let job_spec = row.get::<_, Option<String>>(2)?;
    let job = job_spec
        .as_deref()
        .map(|value| {
            serde_json::from_str(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
            })
        })
        .transpose()?;
    Ok(ScheduledTaskInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        job,
        legacy_command: row.get(3)?,
        cron: row.get(4)?,
        timezone: row.get(5)?,
        timeout_seconds: row.get(6)?,
        coalesce: row.get::<_, i64>(7)? != 0,
        enabled: row.get::<_, i64>(8)? != 0,
        last_run_at: row.get(9)?,
        last_status: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn scheduled_task_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTaskRunInfo> {
    Ok(ScheduledTaskRunInfo {
        id: row.get(0)?,
        task_id: row.get(1)?,
        task_name: row.get(2)?,
        scheduled_for: row.get(3)?,
        status: row.get(4)?,
        worker_id: row.get(5)?,
        claimed_at: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

fn announcement_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnnouncementInfo> {
    Ok(AnnouncementInfo {
        id: row.get(0)?,
        title: row.get(1)?,
        message: row.get(2)?,
        priority: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn get_scheduled_task_from_connection(
    connection: &Connection,
    task_id: i64,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    connection
        .query_row(
            "SELECT id, name, job_spec, legacy_command, cron, timezone, timeout_seconds, coalesce, \
                    enabled, last_run_at, last_status, created_at, updated_at \
             FROM scheduled_tasks WHERE id = ?1",
            [task_id],
            scheduled_task_from_row,
        )
        .optional()
        .map_err(BusinessRepositoryError::from)
}

fn get_announcement_from_connection(
    connection: &Connection,
    announcement_id: i64,
) -> Result<Option<AnnouncementInfo>, BusinessRepositoryError> {
    connection
        .query_row(
            "SELECT id, title, message, priority, enabled, created_at, updated_at \
             FROM announcements WHERE id = ?1",
            [announcement_id],
            announcement_from_row,
        )
        .optional()
        .map_err(BusinessRepositoryError::from)
}

fn read_runtime_setting_rows(
    connection: &Connection,
) -> Result<HashMap<String, (String, f64)>, BusinessRepositoryError> {
    let mut statement =
        connection.prepare("SELECT key, value, updated_at FROM runtime_settings")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    Ok(collect_rows(rows)?
        .into_iter()
        .map(|(key, value, updated_at)| (key, (value, updated_at)))
        .collect())
}

fn public_runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
    codec: &SecretCodec,
) -> Result<RuntimeSettingInfo, BusinessRepositoryError> {
    let internal = internal_runtime_setting_from_definition(definition, row, codec)?;
    let has_value = !internal.value.trim().is_empty();
    Ok(RuntimeSettingInfo {
        field: definition.field.to_string(),
        label: definition.label.to_string(),
        description: definition.description.to_string(),
        input_type: definition.input_type.to_string(),
        is_secret: definition.is_secret,
        value: if definition.is_secret {
            String::new()
        } else {
            internal.value
        },
        has_value,
        masked_value: if definition.is_secret && has_value {
            "••••".to_string()
        } else {
            String::new()
        },
        source: internal.source,
        updated_at: internal.updated_at,
    })
}

fn internal_runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
    codec: &SecretCodec,
) -> Result<RuntimeSettingValue, BusinessRepositoryError> {
    let (stored, source, updated_at) = if let Some((value, updated_at)) = row {
        (value.as_str(), "database".to_string(), Some(*updated_at))
    } else {
        (definition.default_value, "default".to_string(), None)
    };
    let value = if definition.is_secret && row.is_some() {
        codec.decrypt(stored, &runtime_context(definition.field))?
    } else {
        stored.to_string()
    };
    Ok(RuntimeSettingValue {
        field: definition.field.to_string(),
        value,
        source,
        updated_at,
    })
}

fn runtime_definition_by_field(field: &str) -> Option<&'static RuntimeConfigDefinition> {
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .find(|definition| definition.field == field)
}

fn runtime_bool_to_text(value: &str, default: bool) -> Result<String, BusinessRepositoryError> {
    let text = value.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Ok(default.to_string());
    }
    if matches!(text.as_str(), "1" | "true" | "yes" | "on") {
        return Ok("true".to_string());
    }
    if matches!(text.as_str(), "0" | "false" | "no" | "off") {
        return Ok("false".to_string());
    }
    Err(BusinessRepositoryError::InvalidRuntimeBoolean(
        value.to_string(),
    ))
}

fn parse_string_list(value: String) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(&value).unwrap_or_default()
}

fn optional_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn normalize_article_ids(article_ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for article_id in article_ids {
        if *article_id <= 0 || !seen.insert(*article_id) {
            continue;
        }
        normalized.push(*article_id);
    }
    normalized
}

fn normalize_favorite_articles(articles: &[FavoriteArticleRef]) -> Vec<(i64, String)> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for article in articles {
        let article_id = article.article_id.value();
        if article_id <= 0 {
            continue;
        }
        let key = (article_id, article.db_name.clone());
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    normalized
}

fn repeat_placeholders(count: usize, start_index: usize) -> String {
    (start_index..start_index + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn count_table(
    connection: &Connection,
    table_name: &str,
    where_clause: Option<&str>,
) -> Result<i64, BusinessRepositoryError> {
    let sql = if let Some(where_clause) = where_clause {
        format!("SELECT COUNT(*) FROM {table_name} WHERE {where_clause}")
    } else {
        format!("SELECT COUNT(*) FROM {table_name}")
    };
    Ok(connection.query_row(&sql, [], |row| row.get(0))?)
}

fn read_json_file(path: &Path) -> Result<Value, BusinessRepositoryError> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn is_database_selected(selected_databases: &[String], db_name: &str) -> bool {
    let normalized_target = normalize_database_names(&[db_name.to_string()]);
    if normalized_target.is_empty() {
        return false;
    }
    selected_databases.is_empty() || selected_databases.contains(&normalized_target[0])
}

fn filename_string(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string()
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, BusinessRepositoryError> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

fn collect_nested_rows<T>(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Result<T, BusinessRepositoryError>>,
    >,
) -> Result<Vec<T>, BusinessRepositoryError> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row??);
    }
    Ok(items)
}

fn is_constraint_error(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == ErrorCode::ConstraintViolation
    )
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

trait AuthRepositorySqliteError {
    fn into_sqlite_error(self) -> rusqlite::Error;
}

impl AuthRepositorySqliteError for crate::AuthRepositoryError {
    fn into_sqlite_error(self) -> rusqlite::Error {
        match self {
            Self::Sqlite(error) => error,
            error => rusqlite::Error::ToSqlConversionFailure(Box::new(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::{collections::HashMap, fs, thread};

    use ps_domain::{
        NotificationSettingsResponse, NotificationSettingsUpdate, ScheduledIndexJob,
        ScheduledJobSpec,
    };
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{
        claim_ready_scheduled_runs, count_weekly_articles, create_scheduled_task,
        enqueue_scheduled_runs, get_admin_stats, get_scheduler_last_checked_at,
        get_scheduler_status, list_runtime_settings, record_scheduler_check,
        record_scheduler_heartbeat, start_scheduled_run, update_scheduled_task,
        upsert_runtime_settings, BusinessRepositoryError, ScheduledTaskCreateParams,
        ScheduledTaskUpdateParams,
    };
    use crate::{migrate_auth_database, SecretCodec, StorageConfig};

    #[test]
    fn scheduler_repository_validates_typed_jobs_and_replaces_legacy_rows() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let valid_job = ScheduledJobSpec::Index(ScheduledIndexJob {
            metadata_file: Some("journals.csv".to_string()),
            notify: true,
            push: false,
        });
        let created = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Typed index",
                job: &valid_job,
                cron: "0 1 * * *",
                timezone: "UTC",
                timeout_seconds: 3_600,
                coalesce: true,
                enabled: true,
            },
        )
        .expect("typed task should be created");

        assert_eq!(created.job.as_ref(), Some(&valid_job));
        assert_eq!(created.legacy_command, None);

        let invalid_job = ScheduledJobSpec::Index(ScheduledIndexJob {
            metadata_file: Some("../journals.csv".to_string()),
            notify: false,
            push: false,
        });
        let error = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Invalid index",
                job: &invalid_job,
                cron: "0 1 * * *",
                timezone: "UTC",
                timeout_seconds: 3_600,
                coalesce: true,
                enabled: true,
            },
        )
        .expect_err("unsafe path should be rejected");
        assert!(matches!(
            error,
            BusinessRepositoryError::InvalidScheduledJob(_)
        ));

        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO scheduled_tasks
                 (name, job_spec, legacy_command, cron, enabled, last_status, created_at, updated_at)
                 VALUES ('Legacy', NULL, 'index --update && push', '0 2 * * *', 0, '', 1.0, 1.0)",
                [],
            )
            .expect("legacy fixture should insert");
        let legacy_id = connection.last_insert_rowid();
        drop(connection);

        let error = update_scheduled_task(
            &auth_db_path,
            ScheduledTaskUpdateParams {
                task_id: legacy_id,
                name: None,
                job: None,
                cron: None,
                timezone: None,
                timeout_seconds: None,
                coalesce: None,
                enabled: Some(true),
            },
        )
        .expect_err("legacy task should not be enabled");
        assert!(matches!(
            error,
            BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled
        ));

        let replaced = update_scheduled_task(
            &auth_db_path,
            ScheduledTaskUpdateParams {
                task_id: legacy_id,
                name: None,
                job: Some(&valid_job),
                cron: None,
                timezone: None,
                timeout_seconds: None,
                coalesce: None,
                enabled: Some(true),
            },
        )
        .expect("legacy task should accept a typed replacement")
        .expect("legacy task should still exist");
        assert_eq!(replaced.job, Some(valid_job));
        assert_eq!(replaced.legacy_command, None);
        assert!(replaced.enabled);
    }

    #[test]
    fn scheduler_claims_are_unique_and_follow_crash_recovery_rules() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let task = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Durable task",
                job: &ScheduledJobSpec::Index(ScheduledIndexJob {
                    metadata_file: None,
                    notify: false,
                    push: false,
                }),
                cron: "* * * * *",
                timezone: "UTC",
                timeout_seconds: 60,
                coalesce: false,
                enabled: true,
            },
        )
        .expect("task should be created");
        assert_eq!(
            enqueue_scheduled_runs(&auth_db_path, &task, &[60]).expect("run should be queued"),
            1
        );

        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for worker_id in ["worker-a", "worker-b"] {
            let auth_db_path = auth_db_path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                claim_ready_scheduled_runs(&auth_db_path, worker_id, 100.0, 10.0)
                    .expect("concurrent claim should complete")
            }));
        }
        barrier.wait();
        let claims = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("claim thread should finish"))
            .collect::<Vec<_>>();
        assert_eq!(claims.len(), 1);
        let original_run_id = claims[0].run_id;

        let reclaimed = claim_ready_scheduled_runs(&auth_db_path, "worker-c", 111.0, 10.0)
            .expect("stale unstarted claim should be reclaimed");
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].run_id, original_run_id);
        assert_eq!(reclaimed[0].worker_id, "worker-c");
        assert!(
            start_scheduled_run(&auth_db_path, original_run_id, "worker-c", 112.0, 10.0)
                .expect("reclaimed run should start")
        );

        let after_running_expiry =
            claim_ready_scheduled_runs(&auth_db_path, "worker-d", 123.0, 10.0)
                .expect("stale running reconciliation should complete");
        assert!(after_running_expiry.is_empty());
        let status = get_scheduler_status(&auth_db_path, 123.0, 90.0, 10)
            .expect("scheduler status should load");
        assert_eq!(status.recent_runs[0].status, "unknown");
        assert_eq!(status.recent_runs[0].id, original_run_id);
    }

    #[test]
    fn scheduler_cursor_heartbeat_and_coalescing_are_persistent() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let task = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Coalesced task",
                job: &ScheduledJobSpec::Index(ScheduledIndexJob {
                    metadata_file: None,
                    notify: false,
                    push: false,
                }),
                cron: "* * * * *",
                timezone: "UTC",
                timeout_seconds: 60,
                coalesce: true,
                enabled: true,
            },
        )
        .expect("task should be created");

        record_scheduler_check(&auth_db_path, 100.0).expect("cursor should advance");
        record_scheduler_check(&auth_db_path, 90.0).expect("older cursor should be ignored");
        assert_eq!(
            get_scheduler_last_checked_at(&auth_db_path).expect("cursor should load"),
            Some(100.0)
        );
        record_scheduler_heartbeat(&auth_db_path, "worker-a", 110.0)
            .expect("heartbeat should persist");
        assert!(
            crate::has_recent_service_heartbeat(&auth_db_path, 150.0, 60.0)
                .expect("restore-safety heartbeat should load")
        );
        assert!(
            get_scheduler_status(&auth_db_path, 150.0, 60.0, 10)
                .expect("healthy status should load")
                .workers[0]
                .is_healthy
        );
        assert!(
            !get_scheduler_status(&auth_db_path, 200.0, 60.0, 10)
                .expect("stale status should load")
                .workers[0]
                .is_healthy
        );

        assert_eq!(
            enqueue_scheduled_runs(&auth_db_path, &task, &[60, 120, 180])
                .expect("coalesced slots should queue"),
            1
        );
        assert_eq!(
            enqueue_scheduled_runs(&auth_db_path, &task, &[120])
                .expect("an older competing tick should not replace the latest slot"),
            0
        );
        let runs = get_scheduler_status(&auth_db_path, 200.0, 60.0, 10)
            .expect("run status should load")
            .recent_runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].scheduled_for, 180);
    }

    #[test]
    fn runtime_settings_ignore_stale_env_keys_and_proxy_pool() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                ("OPENALEX_API_KEY_POOL", "env-key", 1.0_f64),
            )
            .expect("stale env-key row should insert");
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                ("PROXY_POOL", "proxy", 1.0_f64),
            )
            .expect("stale proxy row should insert");

        let codec = SecretCodec::from_key([8_u8; 32]);
        let settings =
            list_runtime_settings(&auth_db_path, &codec).expect("runtime settings should load");
        let fields = settings
            .iter()
            .map(|setting| setting.field.as_str())
            .collect::<Vec<_>>();

        assert_eq!(settings.len(), 7);
        assert!(fields.contains(&"openalex_api_key_pool"));
        assert!(fields.contains(&"secure_cookies"));
        assert!(!fields.contains(&"proxy_pool"));
        assert!(settings
            .iter()
            .all(|setting| setting.source == "database" || setting.source == "default"));
        let openalex = settings
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert_eq!(openalex.source, "default");
    }

    #[test]
    fn notification_credentials_are_encrypted_masked_preserved_and_cleared_explicitly() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let user = crate::bootstrap_admin(&auth_db_path, "secret-user", "hash", "salt", 1.0)
            .expect("fixture user should bootstrap");
        let codec = SecretCodec::from_key([19_u8; 32]);
        let settings = NotificationSettingsUpdate {
            keywords: vec!["systems".to_string()],
            directions: vec!["security".to_string()],
            selected_databases: Vec::new(),
            delivery_method: "pushplus".to_string(),
            pushplus_token: Some(Some("push-secret-value".to_string())),
            pushplus_template: "markdown".to_string(),
            pushplus_topic: String::new(),
            pushplus_channel: "wechat".to_string(),
            sync_to_tracking_folder: false,
            ai_base_url: "https://ai.example/v1".to_string(),
            ai_api_key: Some(Some("primary-secret-value".to_string())),
            ai_model: "fixture-model".to_string(),
            ai_system_prompt: String::new(),
            ai_backup_base_url: "https://backup.example/v1".to_string(),
            ai_backup_api_key: Some(Some("backup-secret-value".to_string())),
            ai_backup_model: "backup-model".to_string(),
            ai_backup_system_prompt: String::new(),
            ai_retry_attempts: 3,
            enabled: true,
        };

        let stored = super::upsert_notification_settings(&auth_db_path, &codec, user.id, &settings)
            .expect("notification settings should persist");
        assert_eq!(stored.pushplus_token, "push-secret-value");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        let raw = connection
            .query_row(
                "SELECT pushplus_token, ai_api_key, ai_backup_api_key \
                 FROM notification_settings WHERE user_id = ?1",
                [user.id.value()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("encrypted settings should load");
        for ciphertext in [&raw.0, &raw.1, &raw.2] {
            assert!(ciphertext.starts_with("psenc:v1:"));
            assert!(!ciphertext.contains("secret-value"));
        }
        let response = NotificationSettingsResponse::from(&stored);
        let response_json = serde_json::to_string(&response).expect("response should serialize");
        assert!(response.has_pushplus_token);
        assert_eq!(response.pushplus_token_mask, "••••");
        assert!(!response_json.contains("push-secret-value"));
        assert!(!response_json.contains("psenc:v1:"));

        let mut preserve = settings.clone();
        preserve.pushplus_token = Some(Some("   ".to_string()));
        preserve.ai_api_key = None;
        preserve.ai_backup_api_key = None;
        let preserved =
            super::upsert_notification_settings(&auth_db_path, &codec, user.id, &preserve)
                .expect("blank and omitted secrets should preserve");
        assert_eq!(preserved.pushplus_token, "push-secret-value");
        assert_eq!(preserved.ai_api_key, "primary-secret-value");

        preserve.pushplus_token = Some(None);
        let cleared =
            super::upsert_notification_settings(&auth_db_path, &codec, user.id, &preserve)
                .expect("explicit null should clear");
        assert!(cleared.pushplus_token.is_empty());
        assert_eq!(cleared.ai_api_key, "primary-secret-value");
    }

    #[test]
    fn runtime_settings_reject_proxy_pool_and_normalize_boolean() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([8_u8; 32]);
        let mut values = HashMap::new();
        values.insert("secure_cookies".to_string(), Some("yes".to_string()));

        let settings = upsert_runtime_settings(&auth_db_path, &codec, &values)
            .expect("runtime settings should update");
        let secure_cookies = settings
            .iter()
            .find(|setting| setting.field == "secure_cookies")
            .expect("secure cookie setting should exist");

        assert_eq!(secure_cookies.value, "true");
        assert_eq!(secure_cookies.source, "database");

        values.clear();
        values.insert("proxy_pool".to_string(), Some("proxy".to_string()));
        let error = upsert_runtime_settings(&auth_db_path, &codec, &values)
            .expect_err("proxy pool should be rejected");

        assert!(matches!(
            error,
            BusinessRepositoryError::UnknownRuntimeSetting(field) if field == "proxy_pool"
        ));
    }

    #[test]
    fn runtime_credentials_are_encrypted_and_use_preserve_replace_clear_updates() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([23_u8; 32]);
        let values = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            Some("key-one,key-two".to_string()),
        )]);

        let public = upsert_runtime_settings(&auth_db_path, &codec, &values)
            .expect("secret runtime setting should update");
        let openalex = public
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert!(openalex.has_value);
        assert_eq!(openalex.masked_value, "••••");
        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted setting should load");
        assert!(raw.starts_with("psenc:v1:"));
        assert!(!raw.contains("key-one"));
        let internal = super::load_runtime_settings(&auth_db_path, &codec)
            .expect("trusted settings should decrypt");
        assert_eq!(
            internal
                .iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "key-one,key-two"
        );

        upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), Some(" ".to_string()))]),
        )
        .expect("blank secret should preserve");
        assert_eq!(
            super::load_runtime_settings(&auth_db_path, &codec)
                .expect("trusted settings should decrypt")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "key-one,key-two"
        );

        let cleared = upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), None)]),
        )
        .expect("null secret should clear");
        let openalex = cleared
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert!(!openalex.has_value);
        assert!(openalex.masked_value.is_empty());
    }

    #[test]
    fn admin_stats_skip_change_manifests_in_push_state_dir() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        fs::create_dir_all(
            config
                .auth_db_path()
                .parent()
                .expect("auth parent should exist"),
        )
        .expect("data dir should be created");
        migrate_auth_database(config.auth_db_path()).expect("auth database should migrate");
        let push_state_dir = config.project_root().join("data").join("push_state");
        fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
        fs::write(
            push_state_dir.join("runtime.json"),
            r#"{"status":"completed","last_completed_run_at":"2026-07-06T00:00:00Z","run":{"delivered_article_ids":[1,2],"user_results":[{}]}}"#,
        )
        .expect("push state should write");
        fs::write(
            push_state_dir.join("fixture.changes.json"),
            r#"{"db_name":"fixture.sqlite","notifiable_article_ids":[1]}"#,
        )
        .expect("valid change manifest should write");
        fs::write(push_state_dir.join("broken.changes.json"), "{")
            .expect("broken change manifest should write");

        let stats = get_admin_stats(&config).expect("admin stats should load");

        assert_eq!(stats.push.len(), 1);
        assert_eq!(stats.push[0].db_name, "runtime");
        assert_eq!(stats.push[0].status, "completed");
        assert_eq!(stats.push[0].delivered_count, Some(2));
        assert_eq!(stats.push[0].user_results, Some(1));
    }

    #[test]
    fn tracking_weekly_article_count_reads_only_needed_manifest_fields() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        let push_state_dir = config.project_root().join("data").join("push_state");
        fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
        fs::write(
            push_state_dir.join("fixture.changes.json"),
            r#"{"db_name":"fixture.sqlite","notifiable_article_ids":[10,10,"11",null],"backfill_article_ids":[12,"13"],"summary":{"issues":[{"added_article_ids":[10,11,12,13]}]}}"#,
        )
        .expect("manifest should write");
        fs::write(
            push_state_dir.join("missing-db.changes.json"),
            r#"{"notifiable_article_ids":[99],"summary":{"added_article_ids":[99]}}"#,
        )
        .expect("missing db manifest should write");

        let all_count =
            count_weekly_articles(&config, &[]).expect("weekly article count should load");
        let selected_count = count_weekly_articles(&config, &["fixture.sqlite".to_string()])
            .expect("selected weekly article count should load");
        let unselected_count = count_weekly_articles(&config, &["other.sqlite".to_string()])
            .expect("unselected weekly article count should load");

        assert_eq!(all_count, 2);
        assert_eq!(selected_count, 2);
        assert_eq!(unselected_count, 0);
    }
}
