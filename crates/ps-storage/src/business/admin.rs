//! Administrative users, invites, announcements, and statistics.

use super::shared::*;
use super::*;

const ADMIN_INVITE_CODE_BYTES: i64 = 8;

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

fn filename_string(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string()
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
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::{migrate_auth_database, StorageConfig};

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
}
