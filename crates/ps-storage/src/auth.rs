//! Authentication repository operations for the existing auth database.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use ps_domain::{
    UserId, ACCESS_TOKEN_ACTIVE_LIMIT, ACCESS_TOKEN_LIMIT_DETAIL, ACCESS_TOKEN_RESERVED_NAME,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

use crate::{migrate_auth_database, open_sqlite_connection, MigrationError};

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
    /// Database migration failed.
    Migration(MigrationError),
    /// Registration requires an invite code.
    InviteCodeRequired,
    /// Local administrator bootstrap must create the first user.
    AdministratorBootstrapRequired,
    /// The provided invite code is missing or already used.
    InvalidOrUsedInviteCode,
    /// The user has already generated an invite code.
    UserHasAlreadyGeneratedInviteCode,
    /// The username already exists.
    UsernameAlreadyExists,
    /// Local administrator bootstrap has already completed.
    AdministratorBootstrapAlreadyCompleted,
    /// The user already owns the maximum active personal access tokens.
    AccessTokenLimitReached,
}

impl fmt::Display for AuthRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Migration(error) => write!(formatter, "{error}"),
            Self::InviteCodeRequired => formatter.write_str("Invite code is required"),
            Self::AdministratorBootstrapRequired => {
                formatter.write_str("Administrator bootstrap is required")
            }
            Self::InvalidOrUsedInviteCode => formatter.write_str("Invalid or used invite code"),
            Self::UserHasAlreadyGeneratedInviteCode => {
                formatter.write_str("User has already generated an invite code")
            }
            Self::UsernameAlreadyExists => formatter.write_str("Username already exists"),
            Self::AdministratorBootstrapAlreadyCompleted => {
                formatter.write_str("Administrator bootstrap is already complete")
            }
            Self::AccessTokenLimitReached => formatter.write_str(ACCESS_TOKEN_LIMIT_DETAIL),
        }
    }
}

impl Error for AuthRepositoryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Migration(error) => Some(error),
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

impl From<MigrationError> for AuthRepositoryError {
    /// Convert migration errors into repository errors.
    fn from(error: MigrationError) -> Self {
        Self::Migration(error)
    }
}

/// Migrate the auth database to the current schema version.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
///
/// # Returns
///
/// Empty result on success.
pub fn initialize_auth_database(auth_db_path: impl AsRef<Path>) -> Result<(), AuthRepositoryError> {
    migrate_auth_database(auth_db_path).map_err(AuthRepositoryError::from)
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
    let connection = open_auth_connection(auth_db_path)?;
    Ok(connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?)
}

/// Create the first administrator through an explicit local bootstrap transaction.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `username` - Administrator username to create.
/// * `password_hash` - Stored password hash.
/// * `salt` - Stored password salt.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// Created administrator row, or an error when any user already exists.
pub fn bootstrap_admin(
    auth_db_path: impl AsRef<Path>,
    username: &str,
    password_hash: &str,
    salt: &str,
    now: f64,
) -> Result<AuthUserRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = bootstrap_admin_in_transaction(&connection, username, password_hash, salt, now);
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

/// Register a non-administrator using a required one-time invite code.
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

/// Insert a personal access token under the active per-user quota.
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
/// Inserted token metadata, or a typed quota error.
pub fn insert_personal_access_token(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_hash: &str,
    name: &str,
    expires_at: f64,
    created_at: f64,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = insert_personal_access_token_in_transaction(
        &connection,
        user_id,
        token_hash,
        name,
        expires_at,
        created_at,
    );
    finish_immediate_transaction(&connection, result)
}

/// Atomically replace the internal browser login access token.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `token_hash` - SHA-256 token hash.
/// * `expires_at` - Expiration timestamp.
/// * `created_at` - Creation timestamp.
///
/// # Returns
///
/// Inserted login token metadata.
pub fn replace_login_access_token(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_hash: &str,
    expires_at: f64,
    created_at: f64,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = replace_login_access_token_in_transaction(
        &connection,
        user_id,
        token_hash,
        expires_at,
        created_at,
    );
    finish_immediate_transaction(&connection, result)
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
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    open_sqlite_connection(path).map_err(AuthRepositoryError::from)
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
    if user_count == 0 {
        return Err(AuthRepositoryError::AdministratorBootstrapRequired);
    }
    let invite_code = invite_code.ok_or(AuthRepositoryError::InviteCodeRequired)?;
    let user = insert_user_in_transaction(connection, username, password_hash, salt, false, now)?;
    let count = connection.execute(
        "UPDATE invite_codes SET used_by = ?1, used_at = ?2 \
         WHERE code = ?3 AND used_by IS NULL",
        params![user.id.value(), now, invite_code],
    )?;
    if count == 0 {
        return Err(AuthRepositoryError::InvalidOrUsedInviteCode);
    }
    create_default_folder(connection, user.id, now)?;
    Ok(user)
}

fn bootstrap_admin_in_transaction(
    connection: &Connection,
    username: &str,
    password_hash: &str,
    salt: &str,
    now: f64,
) -> Result<AuthUserRow, AuthRepositoryError> {
    let user_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    if user_count != 0 {
        return Err(AuthRepositoryError::AdministratorBootstrapAlreadyCompleted);
    }
    let user = insert_user_in_transaction(connection, username, password_hash, salt, true, now)?;
    create_default_folder(connection, user.id, now)?;
    Ok(user)
}

fn insert_user_in_transaction(
    connection: &Connection,
    username: &str,
    password_hash: &str,
    salt: &str,
    is_admin: bool,
    now: f64,
) -> Result<AuthUserRow, AuthRepositoryError> {
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
    Ok(user)
}

fn create_default_folder(
    connection: &Connection,
    user_id: UserId,
    now: f64,
) -> Result<(), AuthRepositoryError> {
    connection.execute(
        "INSERT INTO folders (user_id, name, is_tracking, created_at, updated_at) \
         VALUES (?1, ?2, 1, ?3, ?4)",
        params![user_id.value(), "默认收藏", now, now],
    )?;
    Ok(())
}

fn purge_expired_access_tokens(
    connection: &Connection,
    now: f64,
) -> Result<usize, AuthRepositoryError> {
    Ok(connection.execute("DELETE FROM access_tokens WHERE expires_at <= ?1", [now])?)
}

fn insert_personal_access_token_in_transaction(
    connection: &Connection,
    user_id: UserId,
    token_hash: &str,
    name: &str,
    expires_at: f64,
    created_at: f64,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    purge_expired_access_tokens(connection, created_at)?;
    let active_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM access_tokens \
         WHERE user_id = ?1 AND expires_at > ?2 AND name != ?3",
        params![user_id.value(), created_at, ACCESS_TOKEN_RESERVED_NAME],
        |row| row.get(0),
    )?;
    if active_count >= ACCESS_TOKEN_ACTIVE_LIMIT {
        return Err(AuthRepositoryError::AccessTokenLimitReached);
    }
    insert_access_token_row(
        connection, user_id, token_hash, name, expires_at, created_at,
    )
}

fn replace_login_access_token_in_transaction(
    connection: &Connection,
    user_id: UserId,
    token_hash: &str,
    expires_at: f64,
    created_at: f64,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    purge_expired_access_tokens(connection, created_at)?;
    connection.execute(
        "DELETE FROM access_tokens WHERE user_id = ?1 AND name = ?2",
        params![user_id.value(), ACCESS_TOKEN_RESERVED_NAME],
    )?;
    insert_access_token_row(
        connection,
        user_id,
        token_hash,
        ACCESS_TOKEN_RESERVED_NAME,
        expires_at,
        created_at,
    )
}

fn insert_access_token_row(
    connection: &Connection,
    user_id: UserId,
    token_hash: &str,
    name: &str,
    expires_at: f64,
    created_at: f64,
) -> Result<AccessTokenRow, AuthRepositoryError> {
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

fn finish_immediate_transaction<Output>(
    connection: &Connection,
    result: Result<Output, AuthRepositoryError>,
) -> Result<Output, AuthRepositoryError> {
    match result {
        Ok(output) => match connection.execute("COMMIT", []) {
            Ok(_) => Ok(output),
            Err(error) => {
                let _ = connection.execute("ROLLBACK", []);
                Err(error.into())
            }
        },
        Err(error) => {
            let _ = connection.execute("ROLLBACK", []);
            Err(error)
        }
    }
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    use ps_domain::{UserId, ACCESS_TOKEN_ACTIVE_LIMIT, ACCESS_TOKEN_RESERVED_NAME};
    use rusqlite::params;
    use tempfile::{tempdir, TempDir};

    use super::{
        bootstrap_admin, delete_access_token, initialize_auth_database,
        insert_personal_access_token, list_access_tokens, open_auth_connection,
        replace_login_access_token, verify_access_token_hash, AuthRepositoryError,
    };

    fn access_token_fixture() -> (TempDir, PathBuf, UserId) {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let user = bootstrap_admin(&auth_db_path, "token_admin", "password-hash", "salt", 1.0)
            .expect("fixture administrator should be created");
        (temp_dir, auth_db_path, user.id)
    }

    fn insert_raw_access_token(
        auth_db_path: &Path,
        user_id: UserId,
        token_hash: &str,
        name: &str,
        expires_at: f64,
        created_at: f64,
    ) -> i64 {
        let connection = open_auth_connection(auth_db_path).expect("auth connection should open");
        connection
            .execute(
                "INSERT INTO access_tokens \
                 (user_id, token_hash, name, expires_at, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![user_id.value(), token_hash, name, expires_at, created_at],
            )
            .expect("raw fixture token should insert");
        connection.last_insert_rowid()
    }

    fn count_tokens_by_hash(auth_db_path: &Path, token_hash: &str) -> i64 {
        let connection = open_auth_connection(auth_db_path).expect("auth connection should open");
        connection
            .query_row(
                "SELECT COUNT(*) FROM access_tokens WHERE token_hash = ?1",
                [token_hash],
                |row| row.get(0),
            )
            .expect("token hash count should load")
    }

    fn login_token_hashes(auth_db_path: &Path, user_id: UserId) -> Vec<String> {
        let connection = open_auth_connection(auth_db_path).expect("auth connection should open");
        let mut statement = connection
            .prepare(
                "SELECT token_hash FROM access_tokens \
                 WHERE user_id = ?1 AND name = ?2 ORDER BY id",
            )
            .expect("login token query should prepare");
        statement
            .query_map(
                params![user_id.value(), ACCESS_TOKEN_RESERVED_NAME],
                |row| row.get(0),
            )
            .expect("login token query should run")
            .map(|row| row.expect("login token hash should load"))
            .collect()
    }

    #[test]
    fn access_token_concurrent_admission_is_bounded() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        for index in 0..(ACCESS_TOKEN_ACTIVE_LIMIT - 1) {
            insert_personal_access_token(
                &auth_db_path,
                user_id,
                &format!("existing-hash-{index}"),
                &format!("existing-{index}"),
                4_000_000_000.0,
                2.0 + index as f64,
            )
            .expect("existing token should be inserted");
        }
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let auth_db_path = auth_db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    insert_personal_access_token(
                        auth_db_path,
                        user_id,
                        &format!("concurrent-hash-{index}"),
                        &format!("concurrent-{index}"),
                        4_000_000_000.0,
                        100.0 + index as f64,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("admission thread should finish"))
            .collect::<Vec<_>>();
        let active =
            list_access_tokens(&auth_db_path, user_id, 200.0).expect("active tokens should list");

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(
                    result,
                    Err(AuthRepositoryError::AccessTokenLimitReached)
                ))
                .count(),
            1
        );
        assert_eq!(active.len() as i64, ACCESS_TOKEN_ACTIVE_LIMIT);
    }

    #[test]
    fn access_token_admission_ignores_expired_and_login_rows_without_rewriting_legacy_rows() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        let mut first_active_id = None;
        for index in 0..(ACCESS_TOKEN_ACTIVE_LIMIT - 1) {
            let token_id = insert_raw_access_token(
                &auth_db_path,
                user_id,
                &format!("active-hash-{index}"),
                &format!("active-{index}"),
                4_000_000_000.0,
                2.0 + index as f64,
            );
            first_active_id.get_or_insert(token_id);
        }
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "login-hash",
            ACCESS_TOKEN_RESERVED_NAME,
            4_000_000_000.0,
            60.0,
        );
        insert_raw_access_token(&auth_db_path, user_id, "expired-hash", "expired", 50.0, 3.0);

        insert_personal_access_token(
            &auth_db_path,
            user_id,
            "fiftieth-hash",
            "fiftieth",
            4_000_000_000.0,
            100.0,
        )
        .expect("expired and login rows should not consume personal quota");
        assert_eq!(count_tokens_by_hash(&auth_db_path, "expired-hash"), 0);
        let legacy_over_limit_id = insert_raw_access_token(
            &auth_db_path,
            user_id,
            "legacy-over-limit-hash",
            "legacy-over-limit",
            4_000_000_000.0,
            101.0,
        );

        let error = insert_personal_access_token(
            &auth_db_path,
            user_id,
            "rejected-hash",
            "rejected",
            4_000_000_000.0,
            102.0,
        )
        .expect_err("legacy over-limit rows should block only new admission");
        let listed = list_access_tokens(&auth_db_path, user_id, 200.0)
            .expect("legacy personal tokens should remain listable");
        let verified = verify_access_token_hash(&auth_db_path, "legacy-over-limit-hash", 200.0)
            .expect("legacy token verification should run")
            .expect("legacy over-limit token should remain usable");

        assert!(matches!(
            error,
            AuthRepositoryError::AccessTokenLimitReached
        ));
        assert_eq!(listed.len() as i64, ACCESS_TOKEN_ACTIVE_LIMIT + 1);
        assert_eq!(verified.id, user_id);
        assert_eq!(login_token_hashes(&auth_db_path, user_id), ["login-hash"]);
        assert_eq!(count_tokens_by_hash(&auth_db_path, "rejected-hash"), 0);
        assert!(
            delete_access_token(&auth_db_path, user_id, legacy_over_limit_id)
                .expect("legacy over-limit token should be revocable")
        );
        assert!(delete_access_token(
            &auth_db_path,
            user_id,
            first_active_id.expect("one active fixture should exist")
        )
        .expect("second legacy token should be revocable"));
        insert_personal_access_token(
            &auth_db_path,
            user_id,
            "recovered-hash",
            "recovered",
            4_000_000_000.0,
            103.0,
        )
        .expect("admission should recover after active count drops below the limit");
        assert_eq!(
            list_access_tokens(&auth_db_path, user_id, 200.0)
                .expect("recovered token should list")
                .len() as i64,
            ACCESS_TOKEN_ACTIVE_LIMIT
        );
    }

    #[test]
    fn access_token_transactions_roll_back_failures_and_serialize_login_replacement() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "duplicate-hash",
            "existing",
            4_000_000_000.0,
            2.0,
        );
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "old-login-hash",
            ACCESS_TOKEN_RESERVED_NAME,
            4_000_000_000.0,
            3.0,
        );
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "rollback-expired-hash",
            "expired",
            50.0,
            4.0,
        );

        let personal_error = insert_personal_access_token(
            &auth_db_path,
            user_id,
            "duplicate-hash",
            "new-personal",
            4_000_000_000.0,
            100.0,
        )
        .expect_err("duplicate personal hash should fail");
        let login_error = replace_login_access_token(
            &auth_db_path,
            user_id,
            "duplicate-hash",
            4_000_000_000.0,
            101.0,
        )
        .expect_err("duplicate login hash should fail");

        assert!(matches!(personal_error, AuthRepositoryError::Sqlite(_)));
        assert!(matches!(login_error, AuthRepositoryError::Sqlite(_)));
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "rollback-expired-hash"),
            1
        );
        assert_eq!(
            login_token_hashes(&auth_db_path, user_id),
            ["old-login-hash"]
        );

        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let auth_db_path = auth_db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    replace_login_access_token(
                        auth_db_path,
                        user_id,
                        &format!("concurrent-login-hash-{index}"),
                        4_000_000_000.0,
                        200.0 + index as f64,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("login thread should finish"))
            .collect::<Vec<_>>();
        let hashes = login_token_hashes(&auth_db_path, user_id);

        assert!(results.iter().all(Result::is_ok));
        assert_eq!(hashes.len(), 1);
        assert!(matches!(
            hashes[0].as_str(),
            "concurrent-login-hash-0" | "concurrent-login-hash-1"
        ));
    }
}
