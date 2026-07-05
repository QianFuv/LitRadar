//! Authentication repository operations for the existing auth database.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use ps_domain::UserId;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

/// Stored user row returned by auth repository queries.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthUserRow {
    /// User identifier.
    pub id: UserId,
    /// Login username.
    pub username: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Stored user credential row.
#[derive(Debug, Clone, PartialEq)]
pub struct UserCredentialRow {
    /// User identifier.
    pub id: UserId,
    /// Login username.
    pub username: String,
    /// Stored password hash.
    pub password_hash: String,
    /// Stored password salt.
    pub salt: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Stored access token metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessTokenRow {
    /// Token row identifier.
    pub id: i64,
    /// Token display name.
    pub name: String,
    /// Expiration timestamp.
    pub expires_at: f64,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Stored invite code metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct InviteCodeRow {
    /// Invite row identifier.
    pub id: i64,
    /// Raw invite code.
    pub code: String,
    /// User that consumed the invite code.
    pub used_by: Option<UserId>,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Repository errors for auth database operations.
#[derive(Debug)]
pub enum AuthRepositoryError {
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// Filesystem setup failed.
    Io(std::io::Error),
    /// Registration requires an invite code.
    InviteCodeRequired,
    /// The provided invite code is missing or already used.
    InvalidOrUsedInviteCode,
    /// The user has already generated an invite code.
    UserHasAlreadyGeneratedInviteCode,
    /// The username already exists.
    UsernameAlreadyExists,
}

impl fmt::Display for AuthRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InviteCodeRequired => formatter.write_str("Invite code is required"),
            Self::InvalidOrUsedInviteCode => formatter.write_str("Invalid or used invite code"),
            Self::UserHasAlreadyGeneratedInviteCode => {
                formatter.write_str("User has already generated an invite code")
            }
            Self::UsernameAlreadyExists => formatter.write_str("Username already exists"),
        }
    }
}

impl Error for AuthRepositoryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for AuthRepositoryError {
    /// Convert SQLite errors into repository errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for AuthRepositoryError {
    /// Convert filesystem errors into repository errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Create auth tables needed by the migrated auth routes.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
///
/// # Returns
///
/// Empty result on success.
pub fn initialize_auth_database(auth_db_path: impl AsRef<Path>) -> Result<(), AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS users (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
            password_hash TEXT    NOT NULL,
            salt          TEXT    NOT NULL,
            is_admin      INTEGER NOT NULL DEFAULT 0,
            created_at    REAL    NOT NULL,
            updated_at    REAL    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS access_tokens (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            token_hash  TEXT    NOT NULL UNIQUE,
            name        TEXT    NOT NULL DEFAULT '',
            expires_at  REAL    NOT NULL,
            created_at  REAL    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cnki_sessions (
            user_id          INTEGER PRIMARY KEY
                             REFERENCES users(id) ON DELETE CASCADE,
            session_json     TEXT    NOT NULL DEFAULT '{}',
            qr_uuid          TEXT    NOT NULL DEFAULT '',
            status           TEXT    NOT NULL DEFAULT 'empty',
            token_expires_at REAL,
            created_at       REAL    NOT NULL,
            updated_at       REAL    NOT NULL,
            last_used_at     REAL
        );

        CREATE TABLE IF NOT EXISTS folders (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name        TEXT    NOT NULL,
            is_tracking INTEGER NOT NULL DEFAULT 0,
            created_at  REAL    NOT NULL,
            updated_at  REAL    NOT NULL,
            UNIQUE(user_id, name)
        );

        CREATE TABLE IF NOT EXISTS favorites (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            folder_id   INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
            article_id  INTEGER NOT NULL,
            db_name     TEXT    NOT NULL DEFAULT '',
            note        TEXT    NOT NULL DEFAULT '',
            created_at  REAL    NOT NULL,
            UNIQUE(user_id, folder_id, article_id, db_name)
        );

        CREATE TABLE IF NOT EXISTS invite_codes (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            code        TEXT    NOT NULL UNIQUE,
            created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
            used_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
            used_at     REAL,
            created_at  REAL   NOT NULL
        );

        CREATE TABLE IF NOT EXISTS notification_settings (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id         INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
            keywords        TEXT    NOT NULL DEFAULT '[]',
            directions      TEXT    NOT NULL DEFAULT '[]',
            selected_databases TEXT NOT NULL DEFAULT '[]',
            delivery_method TEXT    NOT NULL DEFAULT 'folder',
            pushplus_token  TEXT    NOT NULL DEFAULT '',
            pushplus_template TEXT  NOT NULL DEFAULT 'markdown',
            pushplus_topic  TEXT    NOT NULL DEFAULT '',
            pushplus_channel TEXT   NOT NULL DEFAULT 'wechat',
            sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0,
            ai_base_url     TEXT    NOT NULL DEFAULT '',
            ai_api_key      TEXT    NOT NULL DEFAULT '',
            ai_model        TEXT    NOT NULL DEFAULT '',
            ai_system_prompt TEXT   NOT NULL DEFAULT '',
            ai_backup_base_url TEXT NOT NULL DEFAULT '',
            ai_backup_api_key TEXT NOT NULL DEFAULT '',
            ai_backup_model TEXT NOT NULL DEFAULT '',
            ai_backup_system_prompt TEXT NOT NULL DEFAULT '',
            ai_retry_attempts INTEGER NOT NULL DEFAULT 3,
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      REAL    NOT NULL,
            updated_at      REAL    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS scheduled_tasks (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            name            TEXT    NOT NULL,
            command         TEXT    NOT NULL,
            cron            TEXT    NOT NULL,
            enabled         INTEGER NOT NULL DEFAULT 1,
            last_run_at     REAL,
            last_status     TEXT    NOT NULL DEFAULT '',
            created_at      REAL    NOT NULL,
            updated_at      REAL    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS runtime_settings (
            key             TEXT PRIMARY KEY,
            value           TEXT NOT NULL DEFAULT '',
            updated_at      REAL NOT NULL
        );

        CREATE TABLE IF NOT EXISTS announcements (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            title           TEXT    NOT NULL,
            message         TEXT    NOT NULL,
            priority        TEXT    NOT NULL DEFAULT 'normal',
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      REAL    NOT NULL,
            updated_at      REAL    NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_access_tokens_user
            ON access_tokens(user_id);
        CREATE INDEX IF NOT EXISTS idx_folders_user
            ON folders(user_id);
        CREATE INDEX IF NOT EXISTS idx_favorites_folder
            ON favorites(folder_id);
        CREATE INDEX IF NOT EXISTS idx_favorites_user
            ON favorites(user_id);
        CREATE INDEX IF NOT EXISTS idx_invite_codes_code
            ON invite_codes(code);
        CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by
            ON invite_codes(created_by);
        CREATE INDEX IF NOT EXISTS idx_notification_settings_user
            ON notification_settings(user_id);
        CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_enabled
            ON scheduled_tasks(enabled);
        CREATE INDEX IF NOT EXISTS idx_announcements_enabled
            ON announcements(enabled);
        ",
    )?;
    let user_columns = table_columns(&connection, "users")?;
    if !user_columns.iter().any(|column| column == "is_admin") {
        connection.execute(
            "ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        connection.execute(
            "UPDATE users SET is_admin = 1 WHERE id = (SELECT MIN(id) FROM users)",
            [],
        )?;
    }
    let notification_columns = table_columns(&connection, "notification_settings")?;
    let notification_migrations = [
        (
            "selected_databases",
            "ALTER TABLE notification_settings ADD COLUMN selected_databases TEXT NOT NULL DEFAULT '[]'",
        ),
        (
            "ai_base_url",
            "ALTER TABLE notification_settings ADD COLUMN ai_base_url TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_api_key",
            "ALTER TABLE notification_settings ADD COLUMN ai_api_key TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_model",
            "ALTER TABLE notification_settings ADD COLUMN ai_model TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_system_prompt",
            "ALTER TABLE notification_settings ADD COLUMN ai_system_prompt TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_backup_base_url",
            "ALTER TABLE notification_settings ADD COLUMN ai_backup_base_url TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_backup_api_key",
            "ALTER TABLE notification_settings ADD COLUMN ai_backup_api_key TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_backup_model",
            "ALTER TABLE notification_settings ADD COLUMN ai_backup_model TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_backup_system_prompt",
            "ALTER TABLE notification_settings ADD COLUMN ai_backup_system_prompt TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ai_retry_attempts",
            "ALTER TABLE notification_settings ADD COLUMN ai_retry_attempts INTEGER NOT NULL DEFAULT 3",
        ),
        (
            "sync_to_tracking_folder",
            "ALTER TABLE notification_settings ADD COLUMN sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0",
        ),
    ];
    for (column, statement) in notification_migrations {
        if !notification_columns
            .iter()
            .any(|existing| existing == column)
        {
            connection.execute(statement, [])?;
        }
    }
    let announcement_columns = table_columns(&connection, "announcements")?;
    if !announcement_columns.is_empty()
        && !announcement_columns
            .iter()
            .any(|column| column == "priority")
    {
        connection.execute(
            "ALTER TABLE announcements ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal'",
            [],
        )?;
    }
    Ok(())
}

/// Generate lowercase random hex using SQLite `randomblob`.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `byte_count` - Number of random bytes to generate.
///
/// # Returns
///
/// Lowercase random hex string.
pub fn random_hex(
    auth_db_path: impl AsRef<Path>,
    byte_count: i64,
) -> Result<String, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    Ok(
        connection.query_row("SELECT lower(hex(randomblob(?1)))", [byte_count], |row| {
            row.get(0)
        })?,
    )
}

/// Count registered users.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
///
/// # Returns
///
/// Registered user count.
pub fn count_users(auth_db_path: impl AsRef<Path>) -> Result<i64, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    Ok(connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?)
}

/// Register a user while enforcing Python invite semantics.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `username` - Username to create.
/// * `password_hash` - Stored password hash.
/// * `salt` - Stored password salt.
/// * `invite_code` - Optional invite code.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Created user row.
pub fn register_user_with_invite(
    auth_db_path: impl AsRef<Path>,
    username: &str,
    password_hash: &str,
    salt: &str,
    invite_code: Option<&str>,
    now: f64,
) -> Result<AuthUserRow, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result =
        register_user_in_transaction(&connection, username, password_hash, salt, invite_code, now);
    match result {
        Ok(user) => {
            connection.execute("COMMIT", [])?;
            Ok(user)
        }
        Err(error) => {
            let _ = connection.execute("ROLLBACK", []);
            Err(error)
        }
    }
}

/// Find one user's stored credentials by username.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `username` - Username to find.
///
/// # Returns
///
/// Credential row or None.
pub fn find_user_credentials_by_username(
    auth_db_path: impl AsRef<Path>,
    username: &str,
) -> Result<Option<UserCredentialRow>, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT id, username, password_hash, salt, is_admin, created_at \
             FROM users WHERE username = ?1",
            [username],
            credential_from_row,
        )
        .optional()
        .map_err(AuthRepositoryError::from)
}

/// Find one user's stored credentials by id.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Credential row or None.
pub fn find_user_credentials_by_id(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<Option<UserCredentialRow>, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT id, username, password_hash, salt, is_admin, created_at \
             FROM users WHERE id = ?1",
            [user_id.value()],
            credential_from_row,
        )
        .optional()
        .map_err(AuthRepositoryError::from)
}

/// Delete access tokens by display name.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `name` - Token display name.
///
/// # Returns
///
/// Number of deleted rows.
pub fn delete_access_tokens_by_name(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    name: &str,
) -> Result<usize, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM access_tokens WHERE user_id = ?1 AND name = ?2",
        params![user_id.value(), name],
    )?;
    Ok(count)
}

/// Insert a hashed access token.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `token_hash` - SHA-256 token hash.
/// * `name` - Token display name.
/// * `expires_at` - Expiration timestamp.
/// * `created_at` - Creation timestamp.
///
/// # Returns
///
/// Inserted token metadata.
pub fn insert_access_token(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_hash: &str,
    name: &str,
    expires_at: f64,
    created_at: f64,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    purge_expired_access_tokens(&connection, created_at)?;
    connection.execute(
        "INSERT INTO access_tokens \
         (user_id, token_hash, name, expires_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user_id.value(), token_hash, name, expires_at, created_at],
    )?;
    Ok(AccessTokenRow {
        id: connection.last_insert_rowid(),
        name: name.to_string(),
        expires_at,
        created_at,
    })
}

/// Verify an access token hash and return the owning user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `token_hash` - SHA-256 token hash.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Authenticated user row or None.
pub fn verify_access_token_hash(
    auth_db_path: impl AsRef<Path>,
    token_hash: &str,
    now: f64,
) -> Result<Option<AuthUserRow>, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    let row = connection
        .query_row(
            "SELECT t.user_id, t.expires_at, u.username, u.is_admin, u.created_at \
             FROM access_tokens t JOIN users u ON t.user_id = u.id \
             WHERE t.token_hash = ?1",
            [token_hash],
            |row| {
                Ok((
                    UserId(row.get::<_, i64>(0)?),
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, f64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((user_id, expires_at, username, is_admin, created_at)) = row else {
        return Ok(None);
    };
    if expires_at < now {
        connection.execute(
            "DELETE FROM access_tokens WHERE token_hash = ?1",
            [token_hash],
        )?;
        return Ok(None);
    }
    Ok(Some(AuthUserRow {
        id: user_id,
        username,
        is_admin,
        created_at,
    }))
}

/// List active non-login access tokens for a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Token metadata rows.
pub fn list_access_tokens(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    now: f64,
) -> Result<Vec<AccessTokenRow>, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    purge_expired_access_tokens(&connection, now)?;
    let mut statement = connection.prepare(
        "SELECT id, name, expires_at, created_at FROM access_tokens \
         WHERE user_id = ?1 AND expires_at > ?2 AND name != 'login' \
         ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map(params![user_id.value(), now], token_from_row)?;
    collect_rows(rows)
}

/// Delete an access token row by id.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `token_id` - Token row identifier.
///
/// # Returns
///
/// True when a token was deleted.
pub fn delete_access_token(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_id: i64,
) -> Result<bool, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM access_tokens WHERE id = ?1 AND user_id = ?2",
        params![token_id, user_id.value()],
    )?;
    Ok(count > 0)
}

/// Delete an access token row by token hash.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `token_hash` - SHA-256 token hash.
///
/// # Returns
///
/// True when a token was deleted.
pub fn delete_access_token_by_hash(
    auth_db_path: impl AsRef<Path>,
    token_hash: &str,
) -> Result<bool, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM access_tokens WHERE token_hash = ?1",
        [token_hash],
    )?;
    Ok(count > 0)
}

/// Update a user's password and revoke all existing tokens.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `password_hash` - Replacement password hash.
/// * `salt` - Replacement salt.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Empty result on success.
pub fn update_user_password_and_delete_tokens(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    password_hash: &str,
    salt: &str,
    now: f64,
) -> Result<(), AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute(
        "UPDATE users SET password_hash = ?1, salt = ?2, updated_at = ?3 WHERE id = ?4",
        params![password_hash, salt, now, user_id.value()],
    )?;
    connection.execute(
        "DELETE FROM access_tokens WHERE user_id = ?1",
        [user_id.value()],
    )?;
    Ok(())
}

/// Create an invite code for a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `code` - Raw invite code.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Created invite code row.
pub fn create_invite_code(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    code: &str,
    now: f64,
) -> Result<InviteCodeRow, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    let existing = connection
        .query_row(
            "SELECT id FROM invite_codes WHERE created_by = ?1",
            [user_id.value()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if existing.is_some() {
        return Err(AuthRepositoryError::UserHasAlreadyGeneratedInviteCode);
    }
    connection.execute(
        "INSERT INTO invite_codes (code, created_by, created_at) VALUES (?1, ?2, ?3)",
        params![code, user_id.value(), now],
    )?;
    Ok(InviteCodeRow {
        id: connection.last_insert_rowid(),
        code: code.to_string(),
        used_by: None,
        created_at: now,
    })
}

/// Return the invite code created by a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Invite code row or None.
pub fn get_user_invite_code(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<Option<InviteCodeRow>, AuthRepositoryError> {
    initialize_auth_database(auth_db_path.as_ref())?;
    let connection = open_auth_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT id, code, used_by, created_at FROM invite_codes WHERE created_by = ?1",
            [user_id.value()],
            invite_from_row,
        )
        .optional()
        .map_err(AuthRepositoryError::from)
}

pub(crate) fn open_auth_connection(
    path: impl AsRef<Path>,
) -> Result<Connection, AuthRepositoryError> {
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

fn register_user_in_transaction(
    connection: &Connection,
    username: &str,
    password_hash: &str,
    salt: &str,
    invite_code: Option<&str>,
    now: f64,
) -> Result<AuthUserRow, AuthRepositoryError> {
    let user_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    let needs_invite = user_count > 0;
    if needs_invite && invite_code.is_none() {
        return Err(AuthRepositoryError::InviteCodeRequired);
    }
    let is_admin = !needs_invite;

    match connection.execute(
        "INSERT INTO users \
         (username, password_hash, salt, is_admin, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![username, password_hash, salt, is_admin as i64, now, now],
    ) {
        Ok(_) => {}
        Err(error) if is_constraint_error(&error) => {
            return Err(AuthRepositoryError::UsernameAlreadyExists);
        }
        Err(error) => return Err(error.into()),
    }
    let user = connection.query_row(
        "SELECT id, username, is_admin, created_at FROM users WHERE username = ?1",
        [username],
        user_from_row,
    )?;

    if needs_invite {
        let count = connection.execute(
            "UPDATE invite_codes SET used_by = ?1, used_at = ?2 \
             WHERE code = ?3 AND used_by IS NULL",
            params![
                user.id.value(),
                now,
                invite_code.expect("invite code should exist")
            ],
        )?;
        if count == 0 {
            return Err(AuthRepositoryError::InvalidOrUsedInviteCode);
        }
    }

    connection.execute(
        "INSERT INTO folders (user_id, name, is_tracking, created_at, updated_at) \
         VALUES (?1, ?2, 1, ?3, ?4)",
        params![user.id.value(), "默认收藏", now, now],
    )?;
    Ok(user)
}

fn purge_expired_access_tokens(
    connection: &Connection,
    now: f64,
) -> Result<usize, AuthRepositoryError> {
    Ok(connection.execute("DELETE FROM access_tokens WHERE expires_at <= ?1", [now])?)
}

fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuthUserRow> {
    Ok(AuthUserRow {
        id: UserId(row.get(0)?),
        username: row.get(1)?,
        is_admin: row.get::<_, i64>(2)? != 0,
        created_at: row.get(3)?,
    })
}

fn credential_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserCredentialRow> {
    Ok(UserCredentialRow {
        id: UserId(row.get(0)?),
        username: row.get(1)?,
        password_hash: row.get(2)?,
        salt: row.get(3)?,
        is_admin: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
    })
}

fn token_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccessTokenRow> {
    Ok(AccessTokenRow {
        id: row.get(0)?,
        name: row.get(1)?,
        expires_at: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn invite_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InviteCodeRow> {
    Ok(InviteCodeRow {
        id: row.get(0)?,
        code: row.get(1)?,
        used_by: row.get::<_, Option<i64>>(2)?.map(UserId),
        created_at: row.get(3)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, AuthRepositoryError> {
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

fn table_columns(
    connection: &Connection,
    table_name: &str,
) -> Result<Vec<String>, AuthRepositoryError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    collect_rows(rows)
}
