//! Administrative users, invites, announcements, and statistics.

use super::shared::*;
use super::*;

const ADMIN_INVITE_CODE_BYTES: usize = 8;

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
/// * `actor_id` - Administrator requesting the mutation.
/// * `user_id` - Target user identifier.
/// * `is_admin` - Replacement admin flag.
///
/// # Returns
///
/// Empty result when the actor, target, and administrator invariant permit the update.
pub fn set_user_admin(
    auth_db_path: impl AsRef<Path>,
    actor_id: UserId,
    user_id: UserId,
    is_admin: bool,
) -> Result<(), BusinessRepositoryError> {
    set_user_admin_with_audit(auth_db_path, actor_id, user_id, is_admin, None)
}

/// Update administrator status and persist a required audit event atomically.
pub fn set_user_admin_with_audit(
    auth_db_path: impl AsRef<Path>,
    actor_id: UserId,
    user_id: UserId,
    is_admin: bool,
    audit: Option<&SecurityAuditEvent>,
) -> Result<(), BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_administrator_actor(&transaction, actor_id)?;
    let Some(was_admin) = user_admin_flag(&transaction, user_id)? else {
        return Err(BusinessRepositoryError::AdministratorTargetNotFound);
    };
    if was_admin && !is_admin {
        require_another_administrator(&transaction)?;
    }
    let updated = transaction.execute(
        "UPDATE users SET is_admin = ?1, updated_at = ?2 WHERE id = ?3",
        params![is_admin as i64, now_seconds(), user_id.value()],
    )?;
    if updated != 1 {
        return Err(BusinessRepositoryError::AdministratorTargetNotFound);
    }
    if let Some(audit) = audit {
        insert_required_security_audit_event(
            &transaction,
            &audit
                .clone()
                .with_actor_id(actor_id.value())
                .with_target_id(user_id.value()),
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// Delete a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `actor_id` - Administrator requesting the mutation.
/// * `user_id` - Target user identifier.
///
/// # Returns
///
/// Empty result when the actor, target, and administrator invariant permit deletion.
pub fn delete_user(
    auth_db_path: impl AsRef<Path>,
    actor_id: UserId,
    user_id: UserId,
) -> Result<(), BusinessRepositoryError> {
    delete_user_with_audit(auth_db_path, actor_id, user_id, None)
}

/// Delete a user and persist a required audit event atomically.
pub fn delete_user_with_audit(
    auth_db_path: impl AsRef<Path>,
    actor_id: UserId,
    user_id: UserId,
    audit: Option<&SecurityAuditEvent>,
) -> Result<(), BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_administrator_actor(&transaction, actor_id)?;
    let Some(was_admin) = user_admin_flag(&transaction, user_id)? else {
        return Err(BusinessRepositoryError::AdministratorTargetNotFound);
    };
    if was_admin {
        require_another_administrator(&transaction)?;
    }
    let deleted = transaction.execute("DELETE FROM users WHERE id = ?1", [user_id.value()])?;
    if deleted != 1 {
        return Err(BusinessRepositoryError::AdministratorTargetNotFound);
    }
    if let Some(audit) = audit {
        insert_required_security_audit_event(
            &transaction,
            &audit
                .clone()
                .with_actor_id(actor_id.value())
                .with_target_id(user_id.value()),
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn require_administrator_actor(
    connection: &Connection,
    actor_id: UserId,
) -> Result<(), BusinessRepositoryError> {
    if user_admin_flag(connection, actor_id)? == Some(true) {
        Ok(())
    } else {
        Err(BusinessRepositoryError::AdministratorActorForbidden)
    }
}

fn user_admin_flag(
    connection: &Connection,
    user_id: UserId,
) -> Result<Option<bool>, BusinessRepositoryError> {
    connection
        .query_row(
            "SELECT is_admin FROM users WHERE id = ?1",
            [user_id.value()],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )
        .optional()
        .map_err(Into::into)
}

fn require_another_administrator(connection: &Connection) -> Result<(), BusinessRepositoryError> {
    let administrator_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM users WHERE is_admin = 1", [], |row| {
            row.get(0)
        })?;
    if administrator_count > 1 {
        Ok(())
    } else {
        Err(BusinessRepositoryError::AdministratorInvariantViolation)
    }
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
    admin_create_invite_code_with_audit(auth_db_path, None)
}

/// Create an administrator invite and persist a required audit event atomically.
pub fn admin_create_invite_code_with_audit(
    auth_db_path: impl AsRef<Path>,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AdminInviteCodeInfo, BusinessRepositoryError> {
    let code = random_hex(ADMIN_INVITE_CODE_BYTES)
        .map_err(|error| BusinessRepositoryError::Sqlite(error.into_sqlite_error()))?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_seconds();
    transaction.execute(
        "INSERT INTO invite_codes (code, created_by, created_at) VALUES (?1, NULL, ?2)",
        params![code, now],
    )?;
    let invite = AdminInviteCodeInfo {
        id: transaction.last_insert_rowid(),
        code,
        created_by: None,
        created_by_name: None,
        used_by: None,
        used_by_name: None,
        used_at: None,
        created_at: now,
    };
    if let Some(audit) = audit {
        insert_required_security_audit_event(
            &transaction,
            &audit.clone().with_target_id(invite.id),
        )?;
    }
    transaction.commit()?;
    Ok(invite)
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
    delete_invite_code_with_audit(auth_db_path, code_id, None)
}

/// Delete an unused invite and persist a required audit event atomically.
pub fn delete_invite_code_with_audit(
    auth_db_path: impl AsRef<Path>,
    code_id: i64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<bool, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count = transaction.execute(
        "DELETE FROM invite_codes WHERE id = ?1 AND used_by IS NULL",
        [code_id],
    )?;
    if count > 0 {
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &transaction,
                &audit.clone().with_target_id(code_id),
            )?;
        }
    }
    transaction.commit()?;
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
    create_announcement_with_audit(auth_db_path, title, message, priority, enabled, None)
}

/// Create an announcement and persist a required audit event atomically.
pub fn create_announcement_with_audit(
    auth_db_path: impl AsRef<Path>,
    title: &str,
    message: &str,
    priority: &str,
    enabled: bool,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AnnouncementInfo, BusinessRepositoryError> {
    let title = title.trim();
    let message = message.trim();
    let priority = priority.trim().to_ascii_lowercase();
    litradar_domain::validate_announcement_fields(Some(title), Some(message), Some(&priority))?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_seconds();
    transaction.execute(
        "INSERT INTO announcements (title, message, priority, enabled, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![title, message, priority, enabled as i64, now, now],
    )?;
    let announcement_id = transaction.last_insert_rowid();
    let announcement = get_announcement_from_connection(&transaction, announcement_id)?
        .ok_or_else(|| BusinessRepositoryError::from(rusqlite::Error::QueryReturnedNoRows))?;
    if let Some(audit) = audit {
        insert_required_security_audit_event(
            &transaction,
            &audit.clone().with_target_id(announcement_id),
        )?;
    }
    transaction.commit()?;
    Ok(announcement)
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
    update_announcement_with_audit(
        auth_db_path,
        announcement_id,
        title,
        message,
        priority,
        enabled,
        None,
    )
}

/// Update an announcement and persist a required audit event atomically.
pub fn update_announcement_with_audit(
    auth_db_path: impl AsRef<Path>,
    announcement_id: i64,
    title: Option<&str>,
    message: Option<&str>,
    priority: Option<&str>,
    enabled: Option<bool>,
    audit: Option<&SecurityAuditEvent>,
) -> Result<Option<AnnouncementInfo>, BusinessRepositoryError> {
    let title = title.map(str::trim);
    let message = message.map(str::trim);
    let priority = priority.map(|value| value.trim().to_ascii_lowercase());
    litradar_domain::validate_announcement_fields(title, message, priority.as_deref())?;
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(current) = get_announcement_from_connection(&transaction, announcement_id)? else {
        return Ok(None);
    };
    transaction.execute(
        "UPDATE announcements SET title = ?1, message = ?2, priority = ?3, enabled = ?4, \
         updated_at = ?5 WHERE id = ?6",
        params![
            title.unwrap_or(&current.title),
            message.unwrap_or(&current.message),
            priority.as_deref().unwrap_or(&current.priority),
            enabled.unwrap_or(current.enabled) as i64,
            now_seconds(),
            announcement_id
        ],
    )?;
    let announcement = get_announcement_from_connection(&transaction, announcement_id)?;
    if announcement.is_some() {
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &transaction,
                &audit.clone().with_target_id(announcement_id),
            )?;
        }
    }
    transaction.commit()?;
    Ok(announcement)
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
    delete_announcement_with_audit(auth_db_path, announcement_id, None)
}

/// Delete an announcement and persist a required audit event atomically.
pub fn delete_announcement_with_audit(
    auth_db_path: impl AsRef<Path>,
    announcement_id: i64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<bool, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count =
        transaction.execute("DELETE FROM announcements WHERE id = ?1", [announcement_id])?;
    if count > 0 {
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &transaction,
                &audit.clone().with_target_id(announcement_id),
            )?;
        }
    }
    transaction.commit()?;
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
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::{migrate_auth_database, StorageConfig};

    fn insert_user(auth_db_path: &Path, username: &str, is_admin: bool) -> UserId {
        let connection = Connection::open(auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users \
                 (username, password_hash, salt, is_admin, created_at, updated_at) \
                 VALUES (?1, 'fixture-hash', 'fixture-salt', ?2, 1.0, 1.0)",
                params![username, is_admin as i64],
            )
            .expect("fixture user should insert");
        UserId(connection.last_insert_rowid())
    }

    fn administrator_count(auth_db_path: &Path) -> i64 {
        Connection::open(auth_db_path)
            .expect("auth database should open")
            .query_row("SELECT COUNT(*) FROM users WHERE is_admin = 1", [], |row| {
                row.get(0)
            })
            .expect("administrator count should load")
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
    fn announcement_storage_rejects_shared_character_limits_atomically() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");

        let create_error = create_announcement(
            &auth_db_path,
            &"文".repeat(litradar_domain::MAX_ANNOUNCEMENT_TITLE_CHARS + 1),
            "message",
            "normal",
            true,
        )
        .expect_err("oversized title should be rejected");
        assert!(matches!(
            create_error,
            BusinessRepositoryError::InvalidInput(_)
        ));
        let announcement =
            create_announcement(&auth_db_path, "Title", "Original message", "normal", true)
                .expect("valid announcement should be created");

        let update_error = update_announcement(
            &auth_db_path,
            announcement.id,
            None,
            Some(&"文".repeat(litradar_domain::MAX_ANNOUNCEMENT_MESSAGE_CHARS + 1)),
            None,
            None,
        )
        .expect_err("oversized message should be rejected");
        assert!(matches!(
            update_error,
            BusinessRepositoryError::InvalidInput(_)
        ));
        let stored = get_announcement(&auth_db_path, announcement.id)
            .expect("announcement should load")
            .expect("announcement should remain present");
        assert_eq!(stored.message, "Original message");
        assert_eq!(
            Connection::open(&auth_db_path)
                .expect("auth database should open")
                .query_row("SELECT COUNT(*) FROM announcements", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("announcement count should load"),
            1
        );
    }

    #[test]
    fn administrator_invariant_serializes_concurrent_cross_demotions() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let first = insert_user(&auth_db_path, "first_admin", true);
        let second = insert_user(&auth_db_path, "second_admin", true);
        let barrier = Arc::new(Barrier::new(2));
        let handles = [(first, second), (second, first)]
            .into_iter()
            .map(|(actor_id, target_id)| {
                let auth_db_path = auth_db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    set_user_admin(auth_db_path, actor_id, target_id, false)
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("demotion thread should finish"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(BusinessRepositoryError::AdministratorActorForbidden)
                    )
                })
                .count(),
            1
        );
        assert_eq!(administrator_count(&auth_db_path), 1);
    }

    #[test]
    fn administrator_invariant_serializes_concurrent_cross_deletions() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let first = insert_user(&auth_db_path, "first_admin", true);
        let second = insert_user(&auth_db_path, "second_admin", true);
        let barrier = Arc::new(Barrier::new(2));
        let handles = [(first, second), (second, first)]
            .into_iter()
            .map(|(actor_id, target_id)| {
                let auth_db_path = auth_db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    delete_user(auth_db_path, actor_id, target_id)
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("deletion thread should finish"))
            .collect::<Vec<_>>();
        let user_count: i64 = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .expect("user count should load");

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(BusinessRepositoryError::AdministratorActorForbidden)
                    )
                })
                .count(),
            1
        );
        assert_eq!(user_count, 1);
        assert_eq!(administrator_count(&auth_db_path), 1);
    }

    #[test]
    fn administrator_invariant_rejects_stale_actors_and_distinguishes_errors() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let first = insert_user(&auth_db_path, "first_admin", true);
        let second = insert_user(&auth_db_path, "second_admin", true);
        let member = insert_user(&auth_db_path, "member", false);
        let stale_authorization =
            Connection::open(&auth_db_path).expect("stale authorization connection should open");
        assert_eq!(
            user_admin_flag(&stale_authorization, second)
                .expect("stale actor flag should load before demotion"),
            Some(true)
        );
        drop(stale_authorization);

        set_user_admin(&auth_db_path, first, second, false)
            .expect("first administrator should demote the second");
        assert!(matches!(
            set_user_admin(&auth_db_path, second, member, true),
            Err(BusinessRepositoryError::AdministratorActorForbidden)
        ));
        assert!(matches!(
            set_user_admin(&auth_db_path, first, UserId(999_999), true),
            Err(BusinessRepositoryError::AdministratorTargetNotFound)
        ));
        assert!(matches!(
            set_user_admin(&auth_db_path, first, first, false),
            Err(BusinessRepositoryError::AdministratorInvariantViolation)
        ));
        assert!(matches!(
            delete_user(&auth_db_path, first, first),
            Err(BusinessRepositoryError::AdministratorInvariantViolation)
        ));
        assert_eq!(administrator_count(&auth_db_path), 1);
        assert_eq!(
            user_admin_flag(
                &Connection::open(&auth_db_path).expect("auth database should open"),
                member
            )
            .expect("member flag should load"),
            Some(false)
        );
    }
}
