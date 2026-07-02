//! Typed repositories for migrated auth database business routes.

use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::{
    AdminInviteCodeInfo, AdminStatsResponse, AdminUserInfo, AnnouncementInfo, AuthStats,
    FavoriteAdd, FavoriteArticleRef, FavoriteArticleResponse, FavoriteBatchCheckResponse,
    FavoriteCheckResponse, FavoriteResponse, FolderResponse, IndexDatabaseStats, IndexStats,
    NotificationSettingsResponse, NotificationSettingsUpdate, NotificationSubscriberInfo,
    PushStats, RuntimeSettingInfo, ScheduledTaskInfo, UserId,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use serde_json::Value;

use crate::{initialize_auth_database, random_hex, StorageConfig};

const ADMIN_INVITE_CODE_BYTES: i64 = 8;

#[derive(Debug, Clone, Copy)]
struct RuntimeConfigDefinition {
    field: &'static str,
    env_name: &'static str,
    label: &'static str,
    input_type: &'static str,
    is_secret: bool,
    description: &'static str,
    default_value: &'static str,
}

const RUNTIME_CONFIG_DEFINITIONS: [RuntimeConfigDefinition; 4] = [
    RuntimeConfigDefinition {
        field: "openalex_api_key_pool",
        env_name: "OPENALEX_API_KEY_POOL",
        label: "OpenAlex API key pool",
        input_type: "password",
        is_secret: true,
        description: "OpenAlex authenticated request key pool.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "proxy_pool",
        env_name: "PROXY_POOL",
        label: "Proxy pool",
        input_type: "password",
        is_secret: true,
        description: "Comma- or semicolon-separated proxy URLs for scholarly and CNKI requests.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "crossref_mailto_pool",
        env_name: "CROSSREF_MAILTO_POOL",
        label: "Crossref mailto pool",
        input_type: "text",
        is_secret: false,
        description: "Comma- or semicolon-separated Crossref contact emails.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "semantic_scholar_api_key_pool",
        env_name: "SEMANTIC_SCHOLAR_API_KEY_POOL",
        label: "Semantic Scholar API key pool",
        input_type: "password",
        is_secret: true,
        description: "Comma- or semicolon-separated Semantic Scholar REST API keys.",
        default_value: "",
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
}

impl fmt::Display for BusinessRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
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

/// Create a folder for a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
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
    user_id: UserId,
) -> Result<Option<NotificationSettingsResponse>, BusinessRepositoryError> {
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
            notification_settings_from_row,
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
///
/// # Returns
///
/// Enabled subscriber settings ordered by user id.
pub fn list_notification_subscribers(
    auth_db_path: impl AsRef<Path>,
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
    let rows = statement.query_map([], notification_subscriber_from_row)?;
    collect_rows(rows)
}

/// Create or update notification settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `settings` - Normalized notification settings.
///
/// # Returns
///
/// Updated notification settings.
pub fn upsert_notification_settings(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    settings: &NotificationSettingsUpdate,
) -> Result<NotificationSettingsResponse, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let keywords = serde_json::to_string(&settings.keywords)?;
    let directions = serde_json::to_string(&settings.directions)?;
    let selected_databases = serde_json::to_string(&settings.selected_databases)?;
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
            settings.pushplus_token,
            settings.pushplus_template,
            settings.pushplus_topic,
            settings.pushplus_channel,
            settings.sync_to_tracking_folder as i64,
            settings.ai_base_url,
            settings.ai_api_key,
            settings.ai_model,
            settings.ai_system_prompt,
            settings.ai_backup_base_url,
            settings.ai_backup_api_key,
            settings.ai_backup_model,
            settings.ai_backup_system_prompt,
            settings.ai_retry_attempts,
            settings.enabled as i64,
            now,
            now
        ],
    )?;
    get_notification_settings(auth_db_path, user_id)?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
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
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement =
        connection.prepare("SELECT key, value, updated_at FROM runtime_settings")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    let rows: HashMap<String, (String, f64)> = collect_rows(rows)?
        .into_iter()
        .map(|(key, value, updated_at)| (key, (value, updated_at)))
        .collect();
    Ok(RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            runtime_setting_from_definition(definition, rows.get(definition.env_name))
        })
        .collect())
}

/// Upsert managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `values` - Values keyed by API field name.
///
/// # Returns
///
/// Updated runtime setting payloads.
pub fn upsert_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    values: &HashMap<String, String>,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    {
        let mut statement = connection.prepare(
            "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?;
        for (field, raw_value) in values {
            let definition = runtime_definition_by_field(field)
                .ok_or_else(|| BusinessRepositoryError::UnknownRuntimeSetting(field.clone()))?;
            let mut value = raw_value.trim().to_string();
            if definition.input_type == "boolean" {
                value = runtime_bool_to_text(&value, true)?;
            }
            statement.execute(params![definition.env_name, value, now])?;
        }
    }
    list_runtime_settings(auth_db_path)
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
        "SELECT id, name, command, cron, enabled, last_run_at, last_status, created_at, updated_at \
         FROM scheduled_tasks ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map([], scheduled_task_from_row)?;
    collect_rows(rows)
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
/// * `name` - Task name.
/// * `command` - Shell command.
/// * `cron` - Five-field cron expression.
/// * `enabled` - Whether the task is enabled.
///
/// # Returns
///
/// Created task payload.
pub fn create_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    name: &str,
    command: &str,
    cron: &str,
    enabled: bool,
) -> Result<ScheduledTaskInfo, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let now = now_seconds();
    connection.execute(
        "INSERT INTO scheduled_tasks \
         (name, command, cron, enabled, last_run_at, last_status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, NULL, '', ?5, ?6)",
        params![name, command, cron, enabled as i64, now, now],
    )?;
    get_scheduled_task_from_connection(&connection, connection.last_insert_rowid())?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

/// Update a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
/// * `name` - Optional replacement name.
/// * `command` - Optional replacement command.
/// * `cron` - Optional replacement cron expression.
/// * `enabled` - Optional replacement enabled flag.
///
/// # Returns
///
/// Updated task payload or None.
pub fn update_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
    name: Option<&str>,
    command: Option<&str>,
    cron: Option<&str>,
    enabled: Option<bool>,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let Some(current) = get_scheduled_task_from_connection(&connection, task_id)? else {
        return Ok(None);
    };
    connection.execute(
        "UPDATE scheduled_tasks SET name = ?1, command = ?2, cron = ?3, enabled = ?4, \
         updated_at = ?5 WHERE id = ?6",
        params![
            name.unwrap_or(&current.name),
            command.unwrap_or(&current.command),
            cron.unwrap_or(&current.cron),
            enabled.unwrap_or(current.enabled) as i64,
            now_seconds(),
            task_id
        ],
    )?;
    get_scheduled_task_from_connection(&connection, task_id)
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
/// * `ran_at` - Unix timestamp when the command started.
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
        let value = read_json_file(&path)?;
        let Some(db_name) = value
            .get("db_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !is_database_selected(selected_databases, db_name) {
            continue;
        }
        for key in ["notifiable_article_ids", "backfill_article_ids"] {
            let Some(items) = value.get(key).and_then(Value::as_array) else {
                continue;
            };
            for article_id in items.iter().filter_map(Value::as_i64) {
                seen.insert((db_name.to_string(), article_id));
            }
        }
    }
    Ok(seen.len())
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
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
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
    initialize_auth_database(path.as_ref())
        .map_err(|error| BusinessRepositoryError::Io(std::io::Error::other(error)))?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;
        ",
    )?;
    Ok(connection)
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
) -> rusqlite::Result<Result<NotificationSettingsResponse, BusinessRepositoryError>> {
    Ok((|| {
        Ok(NotificationSettingsResponse {
            id: row.get(0)?,
            user_id: UserId(row.get(1)?),
            keywords: parse_string_list(row.get::<_, String>(2)?),
            directions: parse_string_list(row.get::<_, String>(3)?),
            selected_databases: parse_string_list(row.get::<_, String>(4)?),
            delivery_method: row.get(5)?,
            pushplus_token: row.get(6)?,
            pushplus_template: row.get(7)?,
            pushplus_topic: row.get(8)?,
            pushplus_channel: row.get(9)?,
            sync_to_tracking_folder: row.get::<_, i64>(10)? != 0,
            ai_base_url: row.get(11)?,
            ai_api_key: row.get(12)?,
            ai_model: row.get(13)?,
            ai_system_prompt: row.get(14)?,
            ai_backup_base_url: row.get(15)?,
            ai_backup_api_key: row.get(16)?,
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
) -> rusqlite::Result<NotificationSubscriberInfo> {
    let user_id = row.get::<_, i64>(0)?;
    Ok(NotificationSubscriberInfo {
        subscriber_id: user_id.to_string(),
        user_id,
        name: row.get(1)?,
        keywords: parse_string_list(row.get::<_, String>(2)?),
        directions: parse_string_list(row.get::<_, String>(3)?),
        selected_databases: parse_string_list(row.get::<_, String>(4)?),
        delivery_method: row.get(5)?,
        pushplus_token: row.get(6)?,
        template: optional_trimmed(row.get::<_, String>(7)?),
        topic: optional_trimmed(row.get::<_, String>(8)?),
        channel: optional_trimmed(row.get::<_, String>(9)?),
        sync_to_tracking_folder: row.get::<_, i64>(10)? != 0,
        ai_base_url: optional_trimmed(row.get::<_, String>(11)?),
        ai_api_key: optional_trimmed(row.get::<_, String>(12)?),
        ai_model: optional_trimmed(row.get::<_, String>(13)?),
        ai_system_prompt: optional_trimmed(row.get::<_, String>(14)?),
        ai_backup_base_url: optional_trimmed(row.get::<_, String>(15)?),
        ai_backup_api_key: optional_trimmed(row.get::<_, String>(16)?),
        ai_backup_model: optional_trimmed(row.get::<_, String>(17)?),
        ai_backup_system_prompt: optional_trimmed(row.get::<_, String>(18)?),
        ai_retry_attempts: row.get::<_, i64>(19)?.max(1),
        tracking_folder_id: row.get(20)?,
    })
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
    Ok(ScheduledTaskInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        command: row.get(2)?,
        cron: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        last_run_at: row.get(5)?,
        last_status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
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
            "SELECT id, name, command, cron, enabled, last_run_at, last_status, created_at, updated_at \
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

fn runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
) -> RuntimeSettingInfo {
    let (value, source, updated_at) = if let Some((value, updated_at)) = row {
        (value.clone(), "database".to_string(), Some(*updated_at))
    } else if let Ok(value) = env::var(definition.env_name) {
        (value, "environment".to_string(), None)
    } else {
        (
            definition.default_value.to_string(),
            "default".to_string(),
            None,
        )
    };
    RuntimeSettingInfo {
        field: definition.field.to_string(),
        key: definition.env_name.to_string(),
        label: definition.label.to_string(),
        description: definition.description.to_string(),
        input_type: definition.input_type.to_string(),
        is_secret: definition.is_secret,
        value,
        source,
        updated_at,
    }
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
