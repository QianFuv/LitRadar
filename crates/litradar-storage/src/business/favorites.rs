//! Favorite folders and article membership repositories.

use super::shared::*;
use super::*;

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
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.publication_year, \
         a.date, a.authors_json, a.abstract_text, a.doi, a.open_access, a.in_press, \
         j.title AS journal_title, j.issn, j.eissn, i.volume, i.number \
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
                article_id: litradar_domain::ArticleId(article_id),
                db_name: db_name.to_string(),
                note: String::new(),
                created_at: 0.0,
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
    collect_rows(rows)
        .map(|items: Vec<((String, i64), FavoriteArticleResponse)>| items.into_iter().collect())
}

fn json_string_vec_from_business_row(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Vec<String>> {
    let payload = row.get::<_, String>(index)?;
    serde_json::from_str(&payload).map_err(|error| {
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
