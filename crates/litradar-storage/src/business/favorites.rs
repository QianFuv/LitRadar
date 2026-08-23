//! Favorite folders and article membership repositories.

use super::shared::*;
use super::*;
use litradar_domain::{
    validate_characters, validate_favorite_add, validate_favorite_article_ref,
    validate_folder_name, validate_item_count, validate_positive_id, ArticleId,
    FavoriteMetadataStatus, MAX_BATCH_ARTICLE_IDS, MAX_DATABASE_NAME_CHARS,
    SQLITE_IN_QUERY_CHUNK_SIZE,
};

use crate::DatabaseResolutionError;

/// Lean favorite reference used by citation exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteCitationReference {
    /// Canonical article identifier in the selected index database.
    pub article_id: ArticleId,
    /// Stored index database selection.
    pub db_name: String,
}

/// Owned-folder citation snapshot with exact oversize detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteCitationSnapshot {
    /// Folder name used to derive the download filename.
    pub folder_name: String,
    /// Favorite references in deterministic export order.
    pub references: Vec<FavoriteCitationReference>,
    /// Whether one limit-plus-one sentinel row existed.
    pub has_more: bool,
}

/// Citation-only article metadata in favorite order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteCitationRecord {
    /// Canonical article identifier in the selected index database.
    pub article_id: ArticleId,
    /// Stored index database selection.
    pub db_name: String,
    /// Article title when the referenced metadata exists.
    pub title: Option<String>,
    /// Decoded display author names.
    pub authors: Vec<String>,
    /// Journal title when the referenced metadata exists.
    pub journal_title: Option<String>,
    /// Canonical article date when present.
    pub date: Option<String>,
    /// Digital object identifier when present.
    pub doi: Option<String>,
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
    let name = name.trim();
    validate_folder_name(name)?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_seconds();
    if is_tracking {
        transaction.execute(
            "UPDATE folders SET is_tracking = 0, updated_at = ?1 WHERE user_id = ?2",
            params![now, user_id.value()],
        )?;
    }
    match transaction.execute(
        "INSERT INTO folders (user_id, name, is_tracking, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user_id.value(), name, is_tracking as i64, now, now],
    ) {
        Ok(_) => {
            let folder = FolderResponse {
                id: transaction.last_insert_rowid(),
                name: name.to_string(),
                is_tracking,
                article_count: 0,
                created_at: now,
            };
            transaction.commit()?;
            Ok(folder)
        }
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
    let name = name.trim();
    validate_folder_name(name)?;
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
    validate_positive_id("folder_id", folder_id)?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let is_tracking = transaction
        .query_row(
            "SELECT is_tracking FROM folders WHERE id = ?1 AND user_id = ?2",
            params![folder_id, user_id.value()],
            |row| row.get::<_, i64>(0).map(|value| value != 0),
        )
        .optional()?;
    let Some(is_tracking) = is_tracking else {
        return Ok(false);
    };
    if is_tracking {
        let notification_dependencies = transaction
            .query_row(
                "SELECT delivery_method, pushplus_token <> '', sync_to_tracking_folder <> 0
                 FROM notification_settings WHERE user_id = ?1",
                [user_id.value()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((delivery_method, has_pushplus_token, sync_to_tracking_folder)) =
            notification_dependencies
        {
            litradar_domain::validate_notification_dependencies(
                &delivery_method,
                has_pushplus_token,
                sync_to_tracking_folder,
                false,
            )?;
        }
    }
    let count = transaction.execute(
        "DELETE FROM folders WHERE id = ?1 AND user_id = ?2",
        params![folder_id, user_id.value()],
    )?;
    transaction.commit()?;
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
    validate_positive_id("folder_id", folder_id)?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let target = transaction
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
    transaction.execute(
        "UPDATE folders SET is_tracking = 0, updated_at = ?1 WHERE user_id = ?2",
        params![now, user_id.value()],
    )?;
    let updated = transaction.execute(
        "UPDATE folders SET is_tracking = 1, updated_at = ?1 WHERE id = ?2 AND user_id = ?3",
        params![now, folder_id, user_id.value()],
    )?;
    if updated != 1 {
        return Ok(false);
    }
    transaction.commit()?;
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
    validate_positive_id("folder_id", folder_id)?;
    validate_favorite_add(favorite)?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_folder_exists(
        &transaction,
        user_id,
        folder_id,
        BusinessRepositoryError::FolderNotFound,
    )?;
    let now = now_seconds();
    let inserted = transaction
        .query_row(
            "INSERT INTO favorites \
         (user_id, folder_id, article_id, db_name, note, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(user_id, folder_id, article_id, db_name) DO NOTHING \
         RETURNING id, folder_id, article_id, db_name, note, created_at",
            params![
                user_id.value(),
                folder_id,
                favorite.article_id.value(),
                favorite.db_name,
                favorite.note,
                now
            ],
            favorite_from_row,
        )
        .optional()?;
    let stored = if let Some(inserted) = inserted {
        inserted
    } else {
        transaction.query_row(
            "SELECT id, folder_id, article_id, db_name, note, created_at \
             FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
             AND article_id = ?3 AND db_name = ?4",
            params![
                user_id.value(),
                folder_id,
                favorite.article_id.value(),
                favorite.db_name
            ],
            favorite_from_row,
        )?
    };
    transaction.commit()?;
    Ok(stored)
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
    validate_positive_id("folder_id", folder_id)?;
    validate_positive_id("article_id", article_id)?;
    validate_characters("db_name", db_name, MAX_DATABASE_NAME_CHARS)?;
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
         AND article_id = ?3 AND db_name = ?4",
        params![user_id.value(), folder_id, article_id, db_name],
    )?;
    Ok(count > 0)
}

/// Load one owned folder and a bounded citation reference snapshot.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
/// * `folder_id` - Folder row identifier.
/// * `limit` - Maximum references retained in the snapshot.
///
/// # Returns
///
/// Folder name, deterministic references, and exact limit-plus-one state.
pub fn load_favorite_citation_snapshot(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    folder_id: i64,
    limit: usize,
) -> Result<FavoriteCitationSnapshot, BusinessRepositoryError> {
    validate_positive_id("folder_id", folder_id)?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction()?;
    let folder_name = transaction
        .query_row(
            "SELECT name FROM folders WHERE id = ?1 AND user_id = ?2",
            params![folder_id, user_id.value()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(BusinessRepositoryError::FolderNotFound)?;
    let query_limit = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
    let mut statement = transaction.prepare(
        "SELECT article_id, db_name FROM favorites \
         WHERE user_id = ?1 AND folder_id = ?2 \
         ORDER BY created_at DESC, id DESC LIMIT ?3",
    )?;
    let rows = statement.query_map(params![user_id.value(), folder_id, query_limit], |row| {
        Ok(FavoriteCitationReference {
            article_id: ArticleId(row.get(0)?),
            db_name: row.get(1)?,
        })
    })?;
    let mut references = collect_rows(rows)?;
    drop(statement);
    transaction.commit()?;
    let has_more = references.len() > limit;
    references.truncate(limit);
    Ok(FavoriteCitationSnapshot {
        folder_name,
        references,
        has_more,
    })
}

/// Resolve citation-only metadata for a caller-bounded reference batch.
///
/// # Arguments
///
/// * `config` - Storage paths used to resolve index databases.
/// * `references` - Favorite references in caller-defined batch order.
///
/// # Returns
///
/// Citation records in the same order, with blank fields for expected missing content.
pub fn load_favorite_citation_records(
    config: &StorageConfig,
    references: &[FavoriteCitationReference],
) -> Result<Vec<FavoriteCitationRecord>, BusinessRepositoryError> {
    let mut by_db: HashMap<String, Vec<i64>> = HashMap::new();
    for reference in references {
        by_db
            .entry(reference.db_name.clone())
            .or_default()
            .push(reference.article_id.value());
    }
    let mut metadata = HashMap::new();
    for (db_name, article_ids) in by_db {
        let db_path =
            match config.resolve_index_db_path((!db_name.is_empty()).then_some(db_name.as_str())) {
                Ok(db_path) => db_path,
                Err(
                    DatabaseResolutionError::NoSqliteDatabasesFound
                    | DatabaseResolutionError::DatabaseNotFound
                    | DatabaseResolutionError::MultipleDatabasesFound
                    | DatabaseResolutionError::InvalidDatabaseName,
                ) => continue,
                Err(DatabaseResolutionError::Io(error)) => return Err(error.into()),
            };
        metadata.extend(load_citation_metadata_from_index(
            &db_path,
            &db_name,
            &article_ids,
        )?);
    }
    Ok(references
        .iter()
        .map(|reference| {
            metadata
                .get(&(reference.db_name.clone(), reference.article_id.value()))
                .cloned()
                .unwrap_or_else(|| FavoriteCitationRecord {
                    article_id: reference.article_id,
                    db_name: reference.db_name.clone(),
                    title: None,
                    authors: Vec::new(),
                    journal_title: None,
                    date: None,
                    doi: None,
                })
        })
        .collect())
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
    let (metadata, unavailable_databases) = load_favorite_metadata(config, &favorites);
    Ok(favorites
        .into_iter()
        .map(|favorite| {
            let key = (favorite.db_name.clone(), favorite.article_id.value());
            let mut response = FavoriteArticleResponse::from(favorite);
            if unavailable_databases.contains(&response.db_name) {
                response.metadata_status = FavoriteMetadataStatus::Unavailable;
            }
            if let Some(article_metadata) = metadata.get(&key) {
                response.metadata_status = FavoriteMetadataStatus::Available;
                response.journal_id = article_metadata.journal_id;
                response.issue_id = article_metadata.issue_id;
                response.title = article_metadata.title.clone();
                response.publication_year = article_metadata.publication_year;
                response.date = article_metadata.date.clone();
                response.authors = article_metadata.authors.clone();
                response.abstract_text = article_metadata.abstract_text.clone();
                response.doi = article_metadata.doi.clone();
                response.journal_title = article_metadata.journal_title.clone();
                response.open_access = article_metadata.open_access;
                response.in_press = article_metadata.in_press;
                response.volume = article_metadata.volume.clone();
                response.number = article_metadata.number.clone();
                response.issn = article_metadata.issn.clone();
                response.eissn = article_metadata.eissn.clone();
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
    validate_positive_id("article_id", article_id)?;
    validate_characters("db_name", db_name, MAX_DATABASE_NAME_CHARS)?;
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
    validate_item_count("article_ids", article_ids.len(), MAX_BATCH_ARTICLE_IDS)?;
    validate_characters("db_name", db_name, MAX_DATABASE_NAME_CHARS)?;
    let article_ids = normalize_article_ids(article_ids);
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let connection = open_business_connection(auth_db_path)?;
    let mut by_article: HashMap<i64, Vec<FavoriteCheckResponse>> = article_ids
        .iter()
        .copied()
        .map(|id| (id, Vec::new()))
        .collect();
    for chunk in article_ids.chunks(SQLITE_IN_QUERY_CHUNK_SIZE) {
        let placeholders = repeat_placeholders(chunk.len(), 3);
        let sql = format!(
            "SELECT fav.article_id, fav.folder_id, f.name AS folder_name \
             FROM favorites fav JOIN folders f ON fav.folder_id = f.id \
             WHERE fav.user_id = ?1 AND fav.db_name = ?2 \
             AND fav.article_id IN ({placeholders}) \
             ORDER BY fav.article_id, fav.created_at"
        );
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 2);
        values.push(&user_id.0);
        values.push(&db_name);
        for article_id in chunk {
            values.push(article_id);
        }
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(values.as_slice())?;
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
    }
    Ok(article_ids
        .into_iter()
        .map(|article_id| FavoriteBatchCheckResponse {
            article_id: litradar_domain::ArticleId(article_id),
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
    validate_positive_id("folder_id", folder_id)?;
    validate_item_count("articles", articles.len(), MAX_BATCH_ARTICLE_IDS)?;
    for article in articles {
        validate_favorite_add(article)?;
    }
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_folder_exists(
        &transaction,
        user_id,
        folder_id,
        BusinessRepositoryError::FolderNotFound,
    )?;
    let now = now_seconds();
    let mut added = 0_i64;
    {
        let mut statement = transaction.prepare(
            "INSERT OR IGNORE INTO favorites \
             (user_id, folder_id, article_id, db_name, note, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for article in articles {
            added += statement.execute(params![
                user_id.value(),
                folder_id,
                article.article_id.value(),
                article.db_name,
                article.note,
                now
            ])? as i64;
        }
    }
    transaction.commit()?;
    Ok(added)
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
    validate_positive_id("folder_id", folder_id)?;
    validate_item_count("articles", articles.len(), MAX_BATCH_ARTICLE_IDS)?;
    for article in articles {
        validate_favorite_article_ref(article)?;
    }
    let normalized = normalize_favorite_articles(articles);
    if normalized.is_empty() {
        return Ok(0);
    }
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_folder_exists(
        &transaction,
        user_id,
        folder_id,
        BusinessRepositoryError::FolderNotFound,
    )?;
    let mut removed = 0_i64;
    {
        let mut statement = transaction.prepare(
            "DELETE FROM favorites WHERE user_id = ?1 AND folder_id = ?2 \
             AND article_id = ?3 AND db_name = ?4",
        )?;
        for (article_id, db_name) in normalized {
            removed +=
                statement.execute(params![user_id.value(), folder_id, article_id, db_name])? as i64;
        }
    }
    transaction.commit()?;
    Ok(removed)
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
    validate_positive_id("source_folder_id", source_folder_id)?;
    validate_positive_id("target_folder_id", target_folder_id)?;
    if source_folder_id == target_folder_id {
        return Err(BusinessRepositoryError::SourceAndTargetFoldersSame);
    }
    validate_item_count("articles", articles.len(), MAX_BATCH_ARTICLE_IDS)?;
    for article in articles {
        validate_favorite_article_ref(article)?;
    }
    let normalized = normalize_favorite_articles(articles);
    if normalized.is_empty() {
        return Ok(0);
    }
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_folder_exists(
        &transaction,
        user_id,
        source_folder_id,
        BusinessRepositoryError::SourceFolderNotFound,
    )?;
    ensure_folder_exists(
        &transaction,
        user_id,
        target_folder_id,
        BusinessRepositoryError::TargetFolderNotFound,
    )?;
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

fn load_favorite_metadata(
    config: &StorageConfig,
    favorites: &[FavoriteResponse],
) -> (
    HashMap<(String, i64), FavoriteArticleResponse>,
    HashSet<String>,
) {
    let mut by_db: HashMap<String, Vec<i64>> = HashMap::new();
    for favorite in favorites {
        by_db
            .entry(favorite.db_name.clone())
            .or_default()
            .push(favorite.article_id.value());
    }
    let mut result = HashMap::new();
    let mut unavailable_databases = HashSet::new();
    for (db_name, article_ids) in by_db {
        let db_path =
            match config.resolve_index_db_path((!db_name.is_empty()).then_some(db_name.as_str())) {
                Ok(db_path) => db_path,
                Err(
                    DatabaseResolutionError::NoSqliteDatabasesFound
                    | DatabaseResolutionError::DatabaseNotFound
                    | DatabaseResolutionError::InvalidDatabaseName,
                ) => continue,
                Err(DatabaseResolutionError::MultipleDatabasesFound) => {
                    log_favorite_metadata_unavailable(&db_name, "ambiguous_database");
                    unavailable_databases.insert(db_name);
                    continue;
                }
                Err(DatabaseResolutionError::Io(_)) => {
                    log_favorite_metadata_unavailable(&db_name, "filesystem");
                    unavailable_databases.insert(db_name);
                    continue;
                }
            };
        match load_metadata_from_index(&db_path, &db_name, &article_ids) {
            Ok(items) => result.extend(items),
            Err(error) => {
                log_favorite_metadata_unavailable(
                    &db_name,
                    favorite_metadata_error_category(&error),
                );
                unavailable_databases.insert(db_name);
            }
        }
    }
    (result, unavailable_databases)
}

fn favorite_metadata_error_category(error: &BusinessRepositoryError) -> &'static str {
    match error {
        BusinessRepositoryError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
            _,
            _,
            source,
        )) if source.is::<serde_json::Error>() => "json",
        BusinessRepositoryError::Sqlite(_) => "sqlite",
        BusinessRepositoryError::Io(_) => "filesystem",
        BusinessRepositoryError::Json(_) => "json",
        _ => "storage",
    }
}

fn log_favorite_metadata_unavailable(db_name: &str, error_category: &'static str) {
    tracing::warn!(
        event = "favorites.metadata_unavailable",
        database = safe_favorite_database_identifier(db_name),
        error_category,
        "Favorite metadata lookup is unavailable"
    );
}

fn safe_favorite_database_identifier(db_name: &str) -> &str {
    let candidate = db_name.trim();
    if candidate.is_empty() {
        return "default";
    }
    let is_safe = candidate.len() <= MAX_DATABASE_NAME_CHARS
        && candidate.is_ascii()
        && candidate
            .bytes()
            .enumerate()
            .all(|(index, byte)| match byte {
                b'a'..=b'z' | b'0'..=b'9' => true,
                b'.' | b'_' | b'-' => index > 0,
                _ => false,
            });
    if is_safe {
        candidate
    } else {
        "invalid"
    }
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
    let connection = Connection::open(db_path)?;
    let mut result = HashMap::new();
    for chunk in unique_ids.chunks(SQLITE_IN_QUERY_CHUNK_SIZE) {
        let placeholders = repeat_placeholders(chunk.len(), 1);
        let sql = format!(
            "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.publication_year, \
             a.date, a.authors_json, a.abstract_text, a.doi, a.open_access, a.in_press, \
             j.title AS journal_title, j.issn, j.eissn, i.volume, i.number \
             FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
             JOIN journals j ON j.journal_id = a.journal_id \
             WHERE a.article_id IN ({placeholders})"
        );
        let values = chunk
            .iter()
            .map(|article_id| article_id as &dyn rusqlite::ToSql)
            .collect::<Vec<_>>();
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(values.as_slice(), |row| {
            let article_id = row.get::<_, i64>(0)?;
            Ok((
                (db_name.to_string(), article_id),
                FavoriteArticleResponse {
                    id: 0,
                    folder_id: 0,
                    article_id: litradar_domain::ArticleId(article_id),
                    db_name: db_name.to_string(),
                    note: String::new(),
                    created_at: 0.0,
                    metadata_status: FavoriteMetadataStatus::Available,
                    journal_id: row
                        .get::<_, Option<i64>>(1)?
                        .map(litradar_domain::JournalId),
                    issue_id: row.get(2)?,
                    title: row.get(3)?,
                    publication_year: row.get(4)?,
                    date: row.get(5)?,
                    authors: Some(json_string_vec_from_business_row(row, 6)?),
                    abstract_text: row.get(7)?,
                    doi: row.get(8)?,
                    open_access: row.get::<_, Option<i64>>(9)?.map(|value| value != 0),
                    in_press: row.get::<_, Option<i64>>(10)?.map(|value| value != 0),
                    journal_title: row.get(11)?,
                    issn: row.get(12)?,
                    eissn: row.get(13)?,
                    volume: row.get(14)?,
                    number: row.get(15)?,
                },
            ))
        })?;
        result.extend(collect_rows(rows)?);
    }
    Ok(result)
}

fn load_citation_metadata_from_index(
    db_path: &Path,
    db_name: &str,
    article_ids: &[i64],
) -> Result<HashMap<(String, i64), FavoriteCitationRecord>, BusinessRepositoryError> {
    let unique_ids = normalize_article_ids(article_ids);
    if unique_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let connection = Connection::open(db_path)?;
    let placeholders = repeat_placeholders(unique_ids.len(), 1);
    let sql = format!(
        "SELECT a.article_id, a.title, a.authors_json, j.title, a.date, a.doi \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.article_id IN ({placeholders})"
    );
    let values = unique_ids
        .iter()
        .map(|article_id| article_id as &dyn rusqlite::ToSql)
        .collect::<Vec<_>>();
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(values.as_slice(), |row| {
        let article_id = row.get::<_, i64>(0)?;
        Ok((
            (db_name.to_string(), article_id),
            FavoriteCitationRecord {
                article_id: ArticleId(article_id),
                db_name: db_name.to_string(),
                title: row.get(1)?,
                authors: json_string_vec_from_business_row(row, 2)?,
                journal_title: row.get(3)?,
                date: row.get(4)?,
                doi: row.get(5)?,
            },
        ))
    })?;
    Ok(collect_rows(rows)?.into_iter().collect())
}

fn json_string_vec_from_business_row(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Vec<String>> {
    let payload = row.get::<_, String>(index)?;
    crate::article_authors::decode_article_author_names(&payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
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
        article_id: litradar_domain::ArticleId(row.get(2)?),
        db_name: row.get(3)?,
        note: row.get(4)?,
        created_at: row.get(5)?,
    })
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
fn is_constraint_error(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == ErrorCode::ConstraintViolation
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use litradar_domain::{ArticleId, FavoriteMetadataStatus, NotificationSettingsUpdate};
    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::migrations::test_support::CapturedLogs;
    use crate::{migrate_auth_database, SecretCodec};

    #[test]
    fn favorites_tracking_mutations_preserve_the_previous_selection_on_failure() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let original = create_folder(&auth_db_path, user_id, "Original", true)
            .expect("original tracking folder should be created");

        assert!(matches!(
            create_folder(&auth_db_path, user_id, "Original", true),
            Err(BusinessRepositoryError::DuplicateFolderName)
        ));
        assert_eq!(
            get_tracking_folder(&auth_db_path, user_id)
                .expect("tracking folder should load")
                .expect("tracking folder should remain")
                .id,
            original.id
        );

        let replacement = create_folder(&auth_db_path, user_id, "Replacement", false)
            .expect("replacement folder should be created");
        assert!(!set_tracking_folder(&auth_db_path, user_id, 99_999)
            .expect("missing tracking target should be reported"));
        assert_eq!(
            get_tracking_folder(&auth_db_path, user_id)
                .expect("tracking folder should load")
                .expect("tracking folder should remain")
                .id,
            original.id
        );

        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER fail_tracking_selection
                 BEFORE UPDATE OF is_tracking ON folders
                 WHEN NEW.id = {} AND NEW.is_tracking = 1
                 BEGIN SELECT RAISE(ABORT, 'injected tracking selection failure'); END;",
                replacement.id
            ))
            .expect("tracking fault trigger should be created");
        drop(connection);
        assert!(set_tracking_folder(&auth_db_path, user_id, replacement.id).is_err());
        assert_eq!(
            get_tracking_folder(&auth_db_path, user_id)
                .expect("tracking folder should load after failure")
                .expect("tracking folder should survive failure")
                .id,
            original.id
        );
        Connection::open(&auth_db_path)
            .expect("auth database should reopen")
            .execute_batch("DROP TRIGGER fail_tracking_selection;")
            .expect("tracking fault trigger should be removed");

        assert!(set_tracking_folder(&auth_db_path, user_id, replacement.id)
            .expect("replacement tracking folder should be selected"));
        let folders = list_folders(&auth_db_path, user_id).expect("folders should load");
        let tracked = folders
            .iter()
            .filter(|folder| folder.is_tracking)
            .collect::<Vec<_>>();
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].id, replacement.id);
    }

    #[test]
    fn tracking_folder_and_pushplus_sync_are_serialized_in_both_commit_orders() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let tracking = create_folder(&auth_db_path, user_id, "Tracking", true)
            .expect("tracking folder should be created");
        let codec = SecretCodec::from_key([53_u8; 32]);
        let settings = pushplus_sync_settings();
        crate::upsert_notification_settings(&auth_db_path, &codec, user_id, &settings)
            .expect("PushPlus sync should persist while the folder exists");

        let delete_error = delete_folder(&auth_db_path, user_id, tracking.id)
            .expect_err("a committed PushPlus dependency should block folder deletion");
        assert!(matches!(
            delete_error,
            BusinessRepositoryError::InvalidInput(_)
        ));
        assert_eq!(
            delete_error.to_string(),
            "A tracking folder is required before enabling PushPlus sync to tracking"
        );
        assert_eq!(
            get_tracking_folder(&auth_db_path, user_id)
                .expect("tracking folder should load")
                .expect("blocked deletion should retain the folder")
                .id,
            tracking.id
        );

        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let tracking = create_folder(&auth_db_path, user_id, "Tracking", true)
            .expect("second tracking folder should be created");
        let codec = SecretCodec::from_key([59_u8; 32]);
        assert!(delete_folder(&auth_db_path, user_id, tracking.id)
            .expect("folder deletion should commit before settings exist"));
        let settings_error =
            crate::upsert_notification_settings(&auth_db_path, &codec, user_id, &settings)
                .expect_err("PushPlus sync should observe the committed folder deletion");
        assert!(matches!(
            settings_error,
            BusinessRepositoryError::InvalidInput(_)
        ));
        assert_eq!(
            settings_error.to_string(),
            "A tracking folder is required before enabling PushPlus sync to tracking"
        );
        assert!(get_tracking_folder(&auth_db_path, user_id)
            .expect("tracking folder lookup should succeed")
            .is_none());
        assert!(
            crate::get_notification_settings(&auth_db_path, &codec, user_id)
                .expect("notification settings lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn tracking_folder_and_folder_delivery_are_serialized_in_both_commit_orders() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let tracking = create_folder(&auth_db_path, user_id, "Tracking", true)
            .expect("tracking folder should be created");
        let codec = SecretCodec::from_key([61_u8; 32]);
        let settings = serde_json::from_str::<NotificationSettingsUpdate>("{}")
            .expect("folder settings should deserialize");
        crate::upsert_notification_settings(&auth_db_path, &codec, user_id, &settings)
            .expect("folder delivery should persist while the folder exists");

        let delete_error = delete_folder(&auth_db_path, user_id, tracking.id)
            .expect_err("folder delivery should block tracking folder deletion");
        assert!(matches!(
            delete_error,
            BusinessRepositoryError::InvalidInput(_)
        ));
        assert_eq!(
            delete_error.to_string(),
            "A tracking folder is required when delivery_method is 'folder'"
        );
        assert_eq!(
            get_tracking_folder(&auth_db_path, user_id)
                .expect("tracking folder should load")
                .expect("blocked deletion should retain the folder")
                .id,
            tracking.id
        );

        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let tracking = create_folder(&auth_db_path, user_id, "Tracking", true)
            .expect("second tracking folder should be created");
        let codec = SecretCodec::from_key([67_u8; 32]);
        assert!(delete_folder(&auth_db_path, user_id, tracking.id)
            .expect("tracking folder deletion should commit before settings exist"));
        let settings_error =
            crate::upsert_notification_settings(&auth_db_path, &codec, user_id, &settings)
                .expect_err("folder settings should observe the committed deletion");
        assert!(matches!(
            settings_error,
            BusinessRepositoryError::InvalidInput(_)
        ));
        assert_eq!(
            settings_error.to_string(),
            "A tracking folder is required when delivery_method is 'folder'"
        );
        assert!(
            crate::get_notification_settings(&auth_db_path, &codec, user_id)
                .expect("notification settings lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn favorites_repeated_add_returns_the_exact_existing_row() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let folder = create_folder(&auth_db_path, user_id, "Reading", false)
            .expect("folder should be created");
        let first_payload = FavoriteAdd {
            article_id: ArticleId(41),
            db_name: "fixture.sqlite".to_string(),
            note: "first note".to_string(),
        };
        let repeated_payload = FavoriteAdd {
            note: "replacement note".to_string(),
            ..first_payload.clone()
        };

        let first = add_favorite(&auth_db_path, user_id, folder.id, &first_payload)
            .expect("favorite should be inserted");
        let repeated = add_favorite(&auth_db_path, user_id, folder.id, &repeated_payload)
            .expect("duplicate favorite should return the stored row");

        assert_eq!(repeated.id, first.id);
        assert_eq!(repeated.created_at, first.created_at);
        assert_eq!(repeated.note, "first note");
    }

    #[test]
    fn favorites_bulk_add_and_remove_roll_back_after_injected_failures() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let folder = create_folder(&auth_db_path, user_id, "Atomic", false)
            .expect("folder should be created");
        let additions = (1..=3)
            .map(|article_id| FavoriteAdd {
                article_id: ArticleId(article_id),
                db_name: "fixture.sqlite".to_string(),
                note: String::new(),
            })
            .collect::<Vec<_>>();
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_favorite_insert
                 BEFORE INSERT ON favorites WHEN NEW.article_id = 2
                 BEGIN SELECT RAISE(ABORT, 'injected favorite insert failure'); END;",
            )
            .expect("insert fault trigger should be created");
        drop(connection);

        assert!(bulk_add_favorites(&auth_db_path, user_id, folder.id, &additions).is_err());
        assert_eq!(
            count_favorites(&auth_db_path, user_id, Some(folder.id))
                .expect("favorite count should load"),
            0
        );

        let connection = Connection::open(&auth_db_path).expect("auth database should reopen");
        connection
            .execute_batch(
                "DROP TRIGGER fail_favorite_insert;
                 CREATE TRIGGER fail_favorite_delete
                 BEFORE DELETE ON favorites WHEN OLD.article_id = 2
                 BEGIN SELECT RAISE(ABORT, 'injected favorite delete failure'); END;",
            )
            .expect("delete fault trigger should be created");
        drop(connection);
        assert_eq!(
            bulk_add_favorites(&auth_db_path, user_id, folder.id, &additions)
                .expect("favorites should be inserted"),
            3
        );
        let removals = additions
            .iter()
            .map(|favorite| FavoriteArticleRef {
                article_id: favorite.article_id,
                db_name: favorite.db_name.clone(),
            })
            .collect::<Vec<_>>();

        assert!(bulk_remove_favorites(&auth_db_path, user_id, folder.id, &removals).is_err());
        assert_eq!(
            count_favorites(&auth_db_path, user_id, Some(folder.id))
                .expect("favorite count should load"),
            3
        );
    }

    #[test]
    fn favorites_batch_boundary_accepts_five_hundred_and_rejects_one_more() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let folder = create_folder(&auth_db_path, user_id, "Boundary", false)
            .expect("folder should be created");
        let additions = (1..=MAX_BATCH_ARTICLE_IDS as i64)
            .map(|article_id| FavoriteAdd {
                article_id: ArticleId(article_id),
                db_name: "fixture.sqlite".to_string(),
                note: String::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bulk_add_favorites(&auth_db_path, user_id, folder.id, &additions)
                .expect("boundary favorites should be inserted"),
            MAX_BATCH_ARTICLE_IDS as i64
        );
        let mut oversized_additions = additions.clone();
        oversized_additions.push(FavoriteAdd {
            article_id: ArticleId(MAX_BATCH_ARTICLE_IDS as i64 + 1),
            db_name: "fixture.sqlite".to_string(),
            note: String::new(),
        });
        assert!(matches!(
            bulk_add_favorites(&auth_db_path, user_id, folder.id, &oversized_additions),
            Err(BusinessRepositoryError::InvalidInput(_))
        ));
        assert_eq!(
            count_favorites(&auth_db_path, user_id, Some(folder.id))
                .expect("favorite count should remain readable"),
            MAX_BATCH_ARTICLE_IDS as i64
        );
        let article_ids = additions
            .iter()
            .map(|favorite| favorite.article_id.value())
            .collect::<Vec<_>>();

        let checked = batch_is_favorited(&auth_db_path, user_id, &article_ids, "fixture.sqlite")
            .expect("boundary batch should be checked");
        assert_eq!(checked.len(), MAX_BATCH_ARTICLE_IDS);
        assert!(checked.iter().all(|item| item.folders.len() == 1));

        let mut oversized = article_ids;
        oversized.push(MAX_BATCH_ARTICLE_IDS as i64 + 1);
        assert!(matches!(
            batch_is_favorited(&auth_db_path, user_id, &oversized, "fixture.sqlite"),
            Err(BusinessRepositoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn favorites_metadata_queries_chunk_more_than_five_hundred_ids() {
        let temp_dir = tempdir().expect("temp directory should be created");
        let index_db_path = temp_dir.path().join("fixture.sqlite");
        let mut connection = Connection::open(&index_db_path).expect("index database should open");
        connection
            .execute_batch(
                "CREATE TABLE journals (
                     journal_id INTEGER PRIMARY KEY,
                     title TEXT NOT NULL,
                     issn TEXT,
                     eissn TEXT
                 );
                 CREATE TABLE issues (
                     issue_id INTEGER PRIMARY KEY,
                     volume TEXT,
                     number TEXT
                 );
                 CREATE TABLE articles (
                     article_id INTEGER PRIMARY KEY,
                     journal_id INTEGER NOT NULL,
                     issue_id INTEGER,
                     title TEXT,
                     publication_year INTEGER,
                     date TEXT,
                     authors_json TEXT NOT NULL,
                     abstract_text TEXT,
                     doi TEXT,
                     open_access INTEGER,
                     in_press INTEGER
                 );
                 INSERT INTO journals (journal_id, title) VALUES (1, 'Fixture Journal');",
            )
            .expect("minimal index schema should be created");
        let transaction = connection
            .transaction()
            .expect("article fixture transaction should start");
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO articles
                     (article_id, journal_id, title, publication_year, authors_json)
                     VALUES (?1, 1, ?2, 2026, '[]')",
                )
                .expect("article insert should prepare");
            for article_id in 1..=SQLITE_IN_QUERY_CHUNK_SIZE as i64 + 1 {
                statement
                    .execute(params![article_id, format!("Article {article_id}")])
                    .expect("article fixture should insert");
            }
        }
        transaction
            .commit()
            .expect("article fixture transaction should commit");
        let article_ids = (1..=SQLITE_IN_QUERY_CHUNK_SIZE as i64 + 1).collect::<Vec<_>>();

        let metadata = load_metadata_from_index(&index_db_path, "fixture.sqlite", &article_ids)
            .expect("chunked metadata query should load");

        assert_eq!(metadata.len(), SQLITE_IN_QUERY_CHUNK_SIZE + 1);
        assert!(metadata
            .values()
            .all(|item| item.metadata_status == FavoriteMetadataStatus::Available));
    }

    #[test]
    fn favorite_metadata_status_distinguishes_missing_and_operational_failures_safely() {
        const NOTE_SENTINEL: &str = "note-sentinel-never-log";
        const DATABASE_SENTINEL: &str = "credential-sentinel-never-log";

        let (temp_dir, auth_db_path, user_id) = favorite_test_database();
        let config = StorageConfig::from_project_root(temp_dir.path())
            .with_auth_db_path(auth_db_path.clone());
        fs::create_dir_all(config.index_dir()).expect("index directory should be created");
        create_metadata_index(
            &config.index_dir().join("available.sqlite"),
            "Available Article",
            r#"[{"display_name":"Ada Lovelace"}]"#,
        );
        create_metadata_index(
            &config.index_dir().join("json.sqlite"),
            "Invalid Authors",
            "{broken-json",
        );
        drop(
            Connection::open(config.index_dir().join("broken.sqlite"))
                .expect("broken index database should be created"),
        );
        let folder = create_folder(&auth_db_path, user_id, "Metadata Status", false)
            .expect("metadata status folder should be created");
        let unavailable_db_name = format!("{DATABASE_SENTINEL}/broken.sqlite");
        bulk_add_favorites(
            &auth_db_path,
            user_id,
            folder.id,
            &[
                FavoriteAdd {
                    article_id: ArticleId(1),
                    db_name: "available.sqlite".to_string(),
                    note: NOTE_SENTINEL.to_string(),
                },
                FavoriteAdd {
                    article_id: ArticleId(999),
                    db_name: "available.sqlite".to_string(),
                    note: String::new(),
                },
                FavoriteAdd {
                    article_id: ArticleId(1),
                    db_name: "missing.sqlite".to_string(),
                    note: String::new(),
                },
                FavoriteAdd {
                    article_id: ArticleId(1),
                    db_name: "json.sqlite".to_string(),
                    note: String::new(),
                },
                FavoriteAdd {
                    article_id: ArticleId(1),
                    db_name: unavailable_db_name.clone(),
                    note: String::new(),
                },
            ],
        )
        .expect("metadata status favorites should be inserted");
        let captured_logs = CapturedLogs::default();

        let favorites = captured_logs
            .capture(|| list_favorite_articles(&config, user_id, Some(folder.id), 50, 0))
            .expect("favorite rows should remain available");

        assert_eq!(favorites.len(), 5);
        let find_status = |article_id: i64, db_name: &str| {
            favorites
                .iter()
                .find(|favorite| {
                    favorite.article_id == ArticleId(article_id) && favorite.db_name == db_name
                })
                .expect("favorite status fixture should exist")
                .metadata_status
        };
        assert_eq!(
            find_status(1, "available.sqlite"),
            FavoriteMetadataStatus::Available
        );
        assert_eq!(
            find_status(999, "available.sqlite"),
            FavoriteMetadataStatus::Missing
        );
        assert_eq!(
            find_status(1, "missing.sqlite"),
            FavoriteMetadataStatus::Missing
        );
        assert_eq!(
            find_status(1, "json.sqlite"),
            FavoriteMetadataStatus::Unavailable
        );
        assert_eq!(
            find_status(1, &unavailable_db_name),
            FavoriteMetadataStatus::Unavailable
        );
        let available = favorites
            .iter()
            .find(|favorite| favorite.metadata_status == FavoriteMetadataStatus::Available)
            .expect("available metadata should be retained");
        assert_eq!(available.title.as_deref(), Some("Available Article"));

        let events = captured_logs
            .events()
            .into_iter()
            .filter(|event| event["event"] == "favorites.metadata_unavailable")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| {
            event["database"] == "json.sqlite" && event["error_category"] == "json"
        }));
        assert!(events.iter().any(|event| {
            event["database"] == "invalid" && event["error_category"] == "sqlite"
        }));
        let log_text = captured_logs.text();
        assert!(!log_text.contains(NOTE_SENTINEL));
        assert!(!log_text.contains(DATABASE_SENTINEL));
        assert!(!log_text.contains("no such table"));
        assert!(!log_text.contains("broken-json"));
        assert!(!log_text.contains(
            temp_dir
                .path()
                .to_str()
                .expect("temporary directory path should be UTF-8")
        ));
    }

    #[test]
    fn favorite_citation_snapshot_checks_ownership_order_and_sentinel() {
        let (_temp_dir, auth_db_path, user_id) = favorite_test_database();
        let folder = create_folder(&auth_db_path, user_id, "Citation Snapshot", false)
            .expect("citation folder should be created");
        let additions = (1..=3)
            .map(|article_id| FavoriteAdd {
                article_id: ArticleId(article_id),
                db_name: "fixture.sqlite".to_string(),
                note: format!("note {article_id} must not enter the snapshot"),
            })
            .collect::<Vec<_>>();
        bulk_add_favorites(&auth_db_path, user_id, folder.id, &additions)
            .expect("citation favorites should be inserted");

        let snapshot = load_favorite_citation_snapshot(&auth_db_path, user_id, folder.id, 2)
            .expect("citation snapshot should load");

        assert_eq!(snapshot.folder_name, "Citation Snapshot");
        assert_eq!(
            snapshot
                .references
                .iter()
                .map(|reference| reference.article_id.value())
                .collect::<Vec<_>>(),
            [3, 2]
        );
        assert!(snapshot.has_more);
        let complete = load_favorite_citation_snapshot(&auth_db_path, user_id, folder.id, 3)
            .expect("complete citation snapshot should load");
        assert_eq!(complete.references.len(), 3);
        assert!(!complete.has_more);

        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users
                 (username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES ('other-citation-user', 'hash', 'salt', 0, 1.0, 1.0)",
                [],
            )
            .expect("second user should insert");
        let other_user_id = UserId(connection.last_insert_rowid());
        assert!(matches!(
            load_favorite_citation_snapshot(&auth_db_path, other_user_id, folder.id, 2),
            Err(BusinessRepositoryError::FolderNotFound)
        ));
    }

    #[test]
    fn favorite_citation_metadata_preserves_batches_missing_rows_and_unicode_authors() {
        let temp_dir = tempdir().expect("temp directory should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        fs::create_dir_all(config.index_dir()).expect("index directory should be created");
        let index_db_path = config.index_dir().join("fixture.sqlite");
        let connection = Connection::open(&index_db_path).expect("index database should open");
        connection
            .execute_batch(
                "CREATE TABLE journals (
                     journal_id INTEGER PRIMARY KEY,
                     title TEXT NOT NULL
                 );
                 CREATE TABLE articles (
                     article_id INTEGER PRIMARY KEY,
                     journal_id INTEGER NOT NULL,
                     title TEXT,
                     authors_json TEXT NOT NULL,
                     date TEXT,
                     doi TEXT
                 );
                 INSERT INTO journals (journal_id, title) VALUES (1, 'Citation Journal');",
            )
            .expect("minimal citation schema should be created");
        connection
            .execute(
                "INSERT INTO articles
                 (article_id, journal_id, title, authors_json, date, doi)
                 VALUES (1, 1, 'Unicode Citation', ?1, '2026-08-23', '10.1000/unicode')",
                [r#"[{"display_name":"张三"},{"display_name":"Ada Lovelace"}]"#],
            )
            .expect("citation article should insert");
        let mut references = vec![
            FavoriteCitationReference {
                article_id: ArticleId(1),
                db_name: "fixture.sqlite".to_string(),
            },
            FavoriteCitationReference {
                article_id: ArticleId(999),
                db_name: "fixture.sqlite".to_string(),
            },
            FavoriteCitationReference {
                article_id: ArticleId(1),
                db_name: "missing.sqlite".to_string(),
            },
        ];
        references.extend((references.len()..250).map(|_| FavoriteCitationReference {
            article_id: ArticleId(1),
            db_name: "fixture.sqlite".to_string(),
        }));

        let records = load_favorite_citation_records(&config, &references)
            .expect("caller-bounded citation batch should load");

        assert_eq!(records.len(), 250);
        assert_eq!(records[0].title.as_deref(), Some("Unicode Citation"));
        assert_eq!(records[0].authors, ["张三", "Ada Lovelace"]);
        assert_eq!(
            records[0].journal_title.as_deref(),
            Some("Citation Journal")
        );
        assert_eq!(records[0].date.as_deref(), Some("2026-08-23"));
        assert_eq!(records[0].doi.as_deref(), Some("10.1000/unicode"));
        assert!(records[1].title.is_none());
        assert!(records[1].authors.is_empty());
        assert!(records[2].title.is_none());
        assert!(records[2].authors.is_empty());
        assert_eq!(records[249].article_id, ArticleId(1));
        assert_eq!(records[249].title.as_deref(), Some("Unicode Citation"));
        assert!(!config.index_dir().join("missing.sqlite").exists());

        connection
            .execute(
                "UPDATE articles SET authors_json = '{broken-json' WHERE article_id = 1",
                [],
            )
            .expect("invalid author fixture should update");
        assert!(matches!(
            load_favorite_citation_records(&config, &references[..1]),
            Err(BusinessRepositoryError::Sqlite(
                rusqlite::Error::FromSqlConversionFailure(..)
            ))
        ));
    }

    fn create_metadata_index(path: &Path, title: &str, authors_json: &str) {
        let connection = Connection::open(path).expect("metadata index database should open");
        connection
            .execute_batch(
                "CREATE TABLE journals (
                     journal_id INTEGER PRIMARY KEY,
                     title TEXT NOT NULL,
                     issn TEXT,
                     eissn TEXT
                 );
                 CREATE TABLE issues (
                     issue_id INTEGER PRIMARY KEY,
                     volume TEXT,
                     number TEXT
                 );
                 CREATE TABLE articles (
                     article_id INTEGER PRIMARY KEY,
                     journal_id INTEGER NOT NULL,
                     issue_id INTEGER,
                     title TEXT,
                     publication_year INTEGER,
                     date TEXT,
                     authors_json TEXT NOT NULL,
                     abstract_text TEXT,
                     doi TEXT,
                     open_access INTEGER,
                     in_press INTEGER
                 );
                 INSERT INTO journals (journal_id, title) VALUES (1, 'Fixture Journal');",
            )
            .expect("metadata index schema should be created");
        connection
            .execute(
                "INSERT INTO articles
                 (article_id, journal_id, title, publication_year, authors_json)
                 VALUES (1, 1, ?1, 2026, ?2)",
                params![title, authors_json],
            )
            .expect("metadata article should insert");
    }

    fn favorite_test_database() -> (TempDir, PathBuf, UserId) {
        let temp_dir = tempdir().expect("temp directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users
                 (username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES ('favorite-owner', 'hash', 'salt', 1, 1.0, 1.0)",
                [],
            )
            .expect("favorite owner should insert");
        let user_id = UserId(connection.last_insert_rowid());
        (temp_dir, auth_db_path, user_id)
    }

    fn pushplus_sync_settings() -> NotificationSettingsUpdate {
        let mut settings = serde_json::from_str::<NotificationSettingsUpdate>("{}")
            .expect("default notification settings should deserialize");
        settings.delivery_method = "pushplus".to_string();
        settings.pushplus_token = Some(Some("pushplus-token".to_string()));
        settings.sync_to_tracking_folder = true;
        settings
    }
}
