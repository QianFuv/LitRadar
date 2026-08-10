//! Authentication repository operations for the existing auth database.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use litradar_domain::{
    is_valid_invite_code_policy, UserId, ACCESS_TOKEN_ACTIVE_LIMIT, ACCESS_TOKEN_LIMIT_DETAIL,
    ACCESS_TOKEN_RESERVED_NAME, DEFAULT_INVITE_CODE_MAX_USES, DEFAULT_INVITE_CODE_TTL_SECONDS,
};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

use crate::business::{
    insert_required_security_audit_event, SecurityAuditError, SecurityAuditEvent,
};
use crate::{migrate_auth_database, open_sqlite_connection, MigrationError};

const SESSION_REVOCATION_BUSY_TIMEOUT: Duration = Duration::from_millis(250);

/// Stored user row returned by auth repository queries.
#[derive(Clone, PartialEq)]
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

/// Authenticated access-token owner and the observed issuance generation.
#[derive(Clone, PartialEq)]
pub struct VerifiedAccessTokenRow {
    /// Authenticated user metadata.
    pub user: AuthUserRow,
    /// User generation that must still match before issuing another token.
    pub token_generation: i64,
}

impl fmt::Debug for VerifiedAccessTokenRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedAccessTokenRow")
            .field("user", &self.user)
            .field("token_generation", &self.token_generation)
            .finish()
    }
}

impl fmt::Debug for AuthUserRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthUserRow")
            .field("id", &self.id)
            .field("username", &"[REDACTED]")
            .field("is_admin", &self.is_admin)
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Stored user credential row.
#[derive(Clone, PartialEq)]
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
    /// Token issuance generation observed with the credential row.
    pub token_generation: i64,
}

impl fmt::Debug for UserCredentialRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserCredentialRow")
            .field("id", &self.id)
            .field("username", &"[REDACTED]")
            .field("password_hash", &"[REDACTED]")
            .field("salt", &"[REDACTED]")
            .field("is_admin", &self.is_admin)
            .field("created_at", &self.created_at)
            .field("token_generation", &self.token_generation)
            .finish()
    }
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
#[derive(Clone, PartialEq)]
pub struct InviteCodeRow {
    /// Invite row identifier.
    pub id: i64,
    /// Raw invite code.
    pub code: String,
    /// User that consumed the invite code.
    pub used_by: Option<UserId>,
    /// First consumption timestamp retained for compatibility.
    pub used_at: Option<f64>,
    /// Absolute expiration timestamp.
    pub expires_at: f64,
    /// Optional irreversible revocation timestamp.
    pub revoked_at: Option<f64>,
    /// Maximum permitted registrations.
    pub max_uses: i64,
    /// Number of committed registrations.
    pub use_count: i64,
    /// Creation timestamp.
    pub created_at: f64,
}

impl fmt::Debug for InviteCodeRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InviteCodeRow")
            .field("id", &self.id)
            .field("code", &"[REDACTED]")
            .field("used_by", &self.used_by)
            .field("used_at", &self.used_at)
            .field("expires_at", &self.expires_at)
            .field("revoked_at", &self.revoked_at)
            .field("max_uses", &self.max_uses)
            .field("use_count", &self.use_count)
            .field("created_at", &self.created_at)
            .finish()
    }
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
    /// The provided invite code cannot admit another registration.
    InvalidOrUsedInviteCode,
    /// The user already owns an unrevoked invite issuance.
    ActiveInviteCodeAlreadyExists,
    /// Invite expiry or use quota falls outside the managed policy.
    InvalidInvitePolicy,
    /// The username already exists.
    UsernameAlreadyExists,
    /// Local administrator bootstrap has already completed.
    AdministratorBootstrapAlreadyCompleted,
    /// The user already owns the maximum active personal access tokens.
    AccessTokenLimitReached,
    /// Operating-system cryptographic randomness was unavailable.
    EntropyUnavailable,
    /// A credential mutation violated its exact-row invariant.
    CredentialMutationInvariant,
    /// Token authorization changed before a dependent issuance committed.
    StaleAuthorization,
    /// An administrative actor no longer exists or has administrator privileges.
    AdministratorActorForbidden,
    /// A required durable security audit row could not be persisted.
    AuditPersistence(SecurityAuditError),
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
            Self::InvalidOrUsedInviteCode => {
                formatter.write_str("Invalid, expired, revoked, or exhausted invite code")
            }
            Self::ActiveInviteCodeAlreadyExists => {
                formatter.write_str("An unrevoked invite code already exists; rotate it instead")
            }
            Self::InvalidInvitePolicy => {
                formatter.write_str("Invite code policy is outside the allowed range")
            }
            Self::UsernameAlreadyExists => formatter.write_str("Username already exists"),
            Self::AdministratorBootstrapAlreadyCompleted => {
                formatter.write_str("Administrator bootstrap is already complete")
            }
            Self::AccessTokenLimitReached => formatter.write_str(ACCESS_TOKEN_LIMIT_DETAIL),
            Self::EntropyUnavailable => {
                formatter.write_str("Operating-system cryptographic randomness is unavailable")
            }
            Self::CredentialMutationInvariant => {
                formatter.write_str("Credential update affected an unexpected number of users")
            }
            Self::StaleAuthorization => {
                formatter.write_str("Token authorization changed before issuance")
            }
            Self::AdministratorActorForbidden => formatter.write_str("Admin access required"),
            Self::AuditPersistence(_) => formatter.write_str("Security audit persistence failed"),
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
            Self::AuditPersistence(error) => Some(error),
            _ => None,
        }
    }
}

impl AuthRepositoryError {
    /// Return whether a repository failure is transient SQLite lock contention.
    ///
    /// # Returns
    ///
    /// True only for SQLite busy or locked failures that are safe to retry once.
    pub fn is_transient_sqlite_contention(&self) -> bool {
        match self {
            Self::Sqlite(error) => is_transient_sqlite_contention(error),
            Self::AuditPersistence(SecurityAuditError::Sqlite(error)) => {
                is_transient_sqlite_contention(error)
            }
            _ => false,
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

impl From<SecurityAuditError> for AuthRepositoryError {
    /// Convert required audit persistence errors into fail-closed auth errors.
    fn from(error: SecurityAuditError) -> Self {
        Self::AuditPersistence(error)
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

/// Generate lowercase random hex using the operating-system CSPRNG.
///
/// # Arguments
///
/// * `byte_count` - Number of random bytes to generate.
///
/// # Returns
///
/// Lowercase random hex string.
pub fn random_hex(byte_count: usize) -> Result<String, AuthRepositoryError> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::fill(&mut bytes).map_err(|_| AuthRepositoryError::EntropyUnavailable)?;
    Ok(hex::encode(bytes))
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
    bootstrap_admin_with_audit(auth_db_path, username, password_hash, salt, now, None)
}

/// Create the first administrator and persist a required audit event atomically.
pub fn bootstrap_admin_with_audit(
    auth_db_path: impl AsRef<Path>,
    username: &str,
    password_hash: &str,
    salt: &str,
    now: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AuthUserRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = bootstrap_admin_in_transaction(&connection, username, password_hash, salt, now)
        .and_then(|user| {
            if let Some(audit) = audit {
                let audit = audit
                    .clone()
                    .with_actor_id(user.id.value())
                    .with_target_id(user.id.value());
                insert_required_security_audit_event(&connection, &audit)?;
            }
            Ok(user)
        });
    finish_immediate_transaction(&connection, result)
}

/// Register a user and persist a required audit event atomically.
pub fn register_user_with_invite_and_audit(
    auth_db_path: impl AsRef<Path>,
    username: &str,
    password_hash: &str,
    salt: &str,
    invite_code: Option<&str>,
    now: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AuthUserRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = register_user_in_transaction(
        &connection,
        username,
        password_hash,
        salt,
        invite_code,
        now,
        audit,
    )
    .and_then(|user| {
        if let Some(audit) = audit {
            let audit = audit
                .clone()
                .with_actor_id(user.id.value())
                .with_target_id(user.id.value());
            insert_required_security_audit_event(&connection, &audit)?;
        }
        Ok(user)
    });
    finish_immediate_transaction(&connection, result)
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
    register_user_with_invite_and_audit(
        auth_db_path,
        username,
        password_hash,
        salt,
        invite_code,
        now,
        None,
    )
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
            "SELECT id, username, password_hash, salt, is_admin, created_at, token_generation \
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
            "SELECT id, username, password_hash, salt, is_admin, created_at, token_generation \
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
    insert_personal_access_token_with_audit(
        auth_db_path,
        user_id,
        token_hash,
        name,
        expires_at,
        created_at,
        None,
    )
}

/// Insert a personal access token and required audit event atomically.
pub fn insert_personal_access_token_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_hash: &str,
    name: &str,
    expires_at: f64,
    created_at: f64,
    audit: Option<&SecurityAuditEvent>,
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
    )
    .and_then(|row| {
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(row.id),
            )?;
        }
        Ok(row)
    });
    finish_immediate_transaction(&connection, result)
}

/// Insert a personal access token only while the observed authorization is current.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `expected_token_generation` - Generation observed during authorization.
/// * `authorizing_token_hash` - Exact authorizing token hash, when the caller used a token.
/// * `token_hash` - SHA-256 hash of the token being issued.
/// * `name` - Token display name.
/// * `expires_at` - Expiration timestamp.
/// * `created_at` - Creation timestamp and authorization-token validity boundary.
/// * `audit` - Optional required completion audit.
///
/// # Returns
///
/// Inserted token metadata, or `StaleAuthorization` without a mutation.
#[allow(clippy::too_many_arguments)]
pub fn insert_personal_access_token_with_authorization_and_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    expected_token_generation: i64,
    authorizing_token_hash: Option<&str>,
    token_hash: &str,
    name: &str,
    expires_at: f64,
    created_at: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        ensure_token_generation(&connection, user_id, expected_token_generation)?;
        if let Some(authorizing_token_hash) = authorizing_token_hash {
            ensure_authorizing_token(&connection, user_id, authorizing_token_hash, created_at)?;
        }
        let row = insert_personal_access_token_in_transaction(
            &connection,
            user_id,
            token_hash,
            name,
            expires_at,
            created_at,
        )?;
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(row.id),
            )?;
        }
        Ok(row)
    })();
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
    replace_login_access_token_with_audit(
        auth_db_path,
        user_id,
        token_hash,
        expires_at,
        created_at,
        None,
    )
}

/// Replace a browser login token and required audit event atomically.
pub fn replace_login_access_token_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_hash: &str,
    expires_at: f64,
    created_at: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = replace_login_access_token_in_transaction(
        &connection,
        user_id,
        token_hash,
        expires_at,
        created_at,
    )
    .and_then(|row| {
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(row.id),
            )?;
        }
        Ok(row)
    });
    finish_immediate_transaction(&connection, result)
}

/// Replace a login token only while the observed user generation still matches.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `expected_token_generation` - Generation observed with verified credentials.
/// * `token_hash` - SHA-256 hash of the login token being issued.
/// * `expires_at` - Expiration timestamp.
/// * `created_at` - Creation timestamp.
/// * `audit` - Optional required completion audit.
///
/// # Returns
///
/// Inserted login token metadata, or `StaleAuthorization` without a mutation.
#[allow(clippy::too_many_arguments)]
pub fn replace_login_access_token_if_generation_matches_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    expected_token_generation: i64,
    token_hash: &str,
    expires_at: f64,
    created_at: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<AccessTokenRow, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        ensure_token_generation(&connection, user_id, expected_token_generation)?;
        let row = replace_login_access_token_in_transaction(
            &connection,
            user_id,
            token_hash,
            expires_at,
            created_at,
        )?;
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(row.id),
            )?;
        }
        Ok(row)
    })();
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
) -> Result<Option<VerifiedAccessTokenRow>, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    let row = connection
        .query_row(
            "SELECT t.user_id, t.expires_at, u.username, u.is_admin, u.created_at, \
                    u.token_generation \
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
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((user_id, expires_at, username, is_admin, created_at, token_generation)) = row else {
        return Ok(None);
    };
    if expires_at <= now {
        connection.execute(
            "DELETE FROM access_tokens WHERE token_hash = ?1",
            [token_hash],
        )?;
        return Ok(None);
    }
    Ok(Some(VerifiedAccessTokenRow {
        user: AuthUserRow {
            id: user_id,
            username,
            is_admin,
            created_at,
        },
        token_generation,
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
    delete_access_token_with_audit(auth_db_path, user_id, token_id, None)
}

/// Delete an access token and required audit event atomically.
pub fn delete_access_token_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    token_id: i64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let count = connection.execute(
            "DELETE FROM access_tokens WHERE id = ?1 AND user_id = ?2",
            params![token_id, user_id.value()],
        )?;
        if count > 0 {
            if let Some(audit) = audit {
                insert_required_security_audit_event(
                    &connection,
                    &audit.clone().with_target_id(token_id),
                )?;
            }
        }
        Ok(count > 0)
    })();
    finish_immediate_transaction(&connection, result)
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
    delete_access_token_by_hash_with_audit(auth_db_path, token_hash, None)
}

/// Delete an access token by hash and persist a required audit event atomically.
pub fn delete_access_token_by_hash_with_audit(
    auth_db_path: impl AsRef<Path>,
    token_hash: &str,
    audit: Option<&SecurityAuditEvent>,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.busy_timeout(SESSION_REVOCATION_BUSY_TIMEOUT)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let token_id = connection
            .query_row(
                "SELECT id FROM access_tokens WHERE token_hash = ?1",
                [token_hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let count = connection.execute(
            "DELETE FROM access_tokens WHERE token_hash = ?1",
            [token_hash],
        )?;
        if let Some(audit) = audit {
            let audit =
                token_id.map_or_else(|| audit.clone(), |id| audit.clone().with_target_id(id));
            insert_required_security_audit_event(&connection, &audit)?;
        }
        Ok(count > 0)
    })();
    finish_immediate_transaction(&connection, result)
}

/// Delete every access token owned by a user and persist a required audit event atomically.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User whose login and personal access tokens must be revoked.
/// * `audit` - Required terminal security audit event.
///
/// # Returns
///
/// Number of revoked access tokens.
pub fn delete_all_access_tokens_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    audit: &SecurityAuditEvent,
) -> Result<usize, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.busy_timeout(SESSION_REVOCATION_BUSY_TIMEOUT)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let updated = connection.execute(
            "UPDATE users SET token_generation = token_generation + 1 WHERE id = ?1",
            [user_id.value()],
        )?;
        if updated != 1 {
            return Err(AuthRepositoryError::CredentialMutationInvariant);
        }
        let count = connection.execute(
            "DELETE FROM access_tokens WHERE user_id = ?1",
            [user_id.value()],
        )?;
        insert_required_security_audit_event(&connection, audit)?;
        Ok(count)
    })();
    finish_immediate_transaction(&connection, result)
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
/// True when the target user existed and the credential rotation committed.
pub fn update_user_password_and_delete_tokens(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    password_hash: &str,
    salt: &str,
    now: f64,
) -> Result<bool, AuthRepositoryError> {
    update_user_password_and_delete_tokens_with_audit(
        auth_db_path,
        user_id,
        password_hash,
        salt,
        now,
        None,
    )
}

/// Rotate credentials, revoke tokens, and persist a required audit event atomically.
pub fn update_user_password_and_delete_tokens_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    password_hash: &str,
    salt: &str,
    now: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = update_user_password_and_delete_tokens_in_transaction(
        &connection,
        user_id,
        password_hash,
        salt,
        now,
    )
    .and_then(|did_update| {
        if did_update {
            if let Some(audit) = audit {
                insert_required_security_audit_event(&connection, audit)?;
            }
        }
        Ok(did_update)
    });
    finish_immediate_transaction(&connection, result)
}

/// Reset a user's credentials only while the requesting actor remains an administrator.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `actor_id` - Administrator requesting the reset.
/// * `user_id` - Target user identifier.
/// * `password_hash` - Replacement password hash.
/// * `salt` - Replacement salt.
/// * `now` - Current Unix timestamp.
/// * `audit` - Required completion audit event.
///
/// # Returns
///
/// True when the actor remained authorized and the target credential rotation committed.
pub fn update_user_password_as_administrator_with_audit(
    auth_db_path: impl AsRef<Path>,
    actor_id: UserId,
    user_id: UserId,
    password_hash: &str,
    salt: &str,
    now: f64,
    audit: &SecurityAuditEvent,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = require_administrator_actor(&connection, actor_id)
        .and_then(|()| {
            update_user_password_and_delete_tokens_in_transaction(
                &connection,
                user_id,
                password_hash,
                salt,
                now,
            )
        })
        .and_then(|did_update| {
            if did_update {
                insert_required_security_audit_event(
                    &connection,
                    &audit
                        .clone()
                        .with_actor_id(actor_id.value())
                        .with_target_id(user_id.value()),
                )?;
            }
            Ok(did_update)
        });
    finish_immediate_transaction(&connection, result)
}

fn require_administrator_actor(
    connection: &Connection,
    actor_id: UserId,
) -> Result<(), AuthRepositoryError> {
    let is_administrator = connection
        .query_row(
            "SELECT is_admin FROM users WHERE id = ?1",
            [actor_id.value()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some_and(|value| value != 0);
    if is_administrator {
        Ok(())
    } else {
        Err(AuthRepositoryError::AdministratorActorForbidden)
    }
}

/// Replace an observed password row, revoke tokens, and audit the change atomically.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `expected_hash` - Password hash observed before old-password verification.
/// * `expected_salt` - Password salt observed before old-password verification.
/// * `replacement_hash` - Replacement password hash.
/// * `replacement_salt` - Replacement password salt.
/// * `now` - Current Unix timestamp.
/// * `audit` - Required audit event written only for a successful replacement.
///
/// # Returns
///
/// True when the exact observed credential row was replaced.
#[allow(clippy::too_many_arguments)]
pub fn compare_and_swap_user_password_and_delete_tokens_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    expected_hash: &str,
    expected_salt: &str,
    replacement_hash: &str,
    replacement_salt: &str,
    now: f64,
    audit: &SecurityAuditEvent,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let updated = connection.execute(
            "UPDATE users
             SET password_hash = ?1, salt = ?2, updated_at = ?3,
                 token_generation = token_generation + 1
             WHERE id = ?4 AND password_hash = ?5 AND salt = ?6",
            params![
                replacement_hash,
                replacement_salt,
                now,
                user_id.value(),
                expected_hash,
                expected_salt
            ],
        )?;
        if updated > 1 {
            return Err(AuthRepositoryError::CredentialMutationInvariant);
        }
        if updated == 0 {
            return Ok(false);
        }
        connection.execute(
            "DELETE FROM access_tokens WHERE user_id = ?1",
            [user_id.value()],
        )?;
        insert_required_security_audit_event(&connection, audit)?;
        Ok(true)
    })();
    finish_immediate_transaction(&connection, result)
}

/// Replace one matching legacy password row with an Argon2id PHC string.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User identifier.
/// * `expected_hash` - Legacy password hash observed before verification.
/// * `expected_salt` - Legacy salt observed before verification.
/// * `replacement_hash` - Argon2id PHC string to store.
/// * `now` - Current Unix timestamp.
///
/// # Returns
///
/// True when the exact legacy row was upgraded. Existing access tokens are unchanged.
pub fn compare_and_swap_legacy_password_hash(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    expected_hash: &str,
    expected_salt: &str,
    replacement_hash: &str,
    now: f64,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    let updated = connection.execute(
        "UPDATE users SET password_hash = ?1, salt = '', updated_at = ?2 \
         WHERE id = ?3 AND password_hash = ?4 AND salt = ?5",
        params![
            replacement_hash,
            now,
            user_id.value(),
            expected_hash,
            expected_salt
        ],
    )?;
    if updated > 1 {
        return Err(AuthRepositoryError::CredentialMutationInvariant);
    }
    Ok(updated == 1)
}

fn update_user_password_and_delete_tokens_in_transaction(
    connection: &Connection,
    user_id: UserId,
    password_hash: &str,
    salt: &str,
    now: f64,
) -> Result<bool, AuthRepositoryError> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM users WHERE id = ?1",
            [user_id.value()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(false);
    }
    let updated = connection.execute(
        "UPDATE users
         SET password_hash = ?1, salt = ?2, updated_at = ?3,
             token_generation = token_generation + 1
         WHERE id = ?4",
        params![password_hash, salt, now, user_id.value()],
    )?;
    if updated != 1 {
        return Err(AuthRepositoryError::CredentialMutationInvariant);
    }
    connection.execute(
        "DELETE FROM access_tokens WHERE user_id = ?1",
        [user_id.value()],
    )?;
    Ok(true)
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
    create_invite_code_with_audit(auth_db_path, user_id, code, now, None)
}

/// Create an invite code and persist a required audit event atomically.
pub fn create_invite_code_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    code: &str,
    now: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<InviteCodeRow, AuthRepositoryError> {
    issue_invite_code_with_audit(
        auth_db_path,
        user_id,
        code,
        now,
        now + DEFAULT_INVITE_CODE_TTL_SECONDS as f64,
        DEFAULT_INVITE_CODE_MAX_USES,
        audit,
    )
}

/// Issue an invite code with an explicit bounded lifecycle policy and atomic audit event.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - User issuing the invite.
/// * `code` - Raw operating-system-random code.
/// * `now` - Current Unix timestamp.
/// * `expires_at` - Absolute expiration timestamp.
/// * `max_uses` - Maximum permitted registrations.
/// * `audit` - Optional required completion audit event.
///
/// # Returns
///
/// Newly issued invite row.
pub fn issue_invite_code_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    code: &str,
    now: f64,
    expires_at: f64,
    max_uses: i64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<InviteCodeRow, AuthRepositoryError> {
    validate_invite_policy(now, expires_at, max_uses)?;
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let existing = connection
            .query_row(
                "SELECT id FROM invite_codes WHERE created_by = ?1 AND revoked_at IS NULL",
                [user_id.value()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(AuthRepositoryError::ActiveInviteCodeAlreadyExists);
        }
        let row = insert_invite_code_in_transaction(
            &connection,
            code,
            Some(user_id),
            now,
            expires_at,
            max_uses,
        )?;
        if let Some(audit) = audit {
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(row.id),
            )?;
        }
        Ok(row)
    })();
    finish_immediate_transaction(&connection, result)
}

/// Revoke the current user's unrevoked invite code and audit the lifecycle change atomically.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - Invite creator.
/// * `now` - Revocation timestamp.
/// * `audit` - Optional required completion audit event.
///
/// # Returns
///
/// True when an unrevoked invite was changed.
pub fn revoke_user_invite_code_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    now: f64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<bool, AuthRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let invite_id = connection
            .query_row(
                "UPDATE invite_codes SET revoked_at = MAX(?1, created_at)
                 WHERE created_by = ?2 AND revoked_at IS NULL
                 RETURNING id",
                params![now, user_id.value()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let (Some(invite_id), Some(audit)) = (invite_id, audit) {
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(invite_id),
            )?;
        }
        Ok(invite_id.is_some())
    })();
    finish_immediate_transaction(&connection, result)
}

/// Revoke any prior issuance and create one replacement invite in the same transaction.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the auth SQLite database.
/// * `user_id` - Invite creator.
/// * `code` - Raw operating-system-random replacement code.
/// * `now` - Rotation timestamp.
/// * `expires_at` - Absolute replacement expiration timestamp.
/// * `max_uses` - Replacement registration quota.
/// * `audit` - Optional required completion audit event.
///
/// # Returns
///
/// Newly issued replacement invite row.
pub fn rotate_user_invite_code_with_audit(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
    code: &str,
    now: f64,
    expires_at: f64,
    max_uses: i64,
    audit: Option<&SecurityAuditEvent>,
) -> Result<InviteCodeRow, AuthRepositoryError> {
    validate_invite_policy(now, expires_at, max_uses)?;
    let connection = open_auth_connection(auth_db_path)?;
    connection.execute("BEGIN IMMEDIATE", [])?;
    let result = (|| {
        let previous_id = connection
            .query_row(
                "UPDATE invite_codes SET revoked_at = MAX(?1, created_at)
                 WHERE created_by = ?2 AND revoked_at IS NULL
                 RETURNING id",
                params![now, user_id.value()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let row = insert_invite_code_in_transaction(
            &connection,
            code,
            Some(user_id),
            now,
            expires_at,
            max_uses,
        )?;
        if let Some(audit) = audit {
            if let Some(previous_id) = previous_id {
                insert_required_security_audit_event(
                    &connection,
                    &audit.clone().with_target_id(previous_id),
                )?;
            }
            insert_required_security_audit_event(
                &connection,
                &audit.clone().with_target_id(row.id),
            )?;
        }
        Ok(row)
    })();
    finish_immediate_transaction(&connection, result)
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
            "SELECT id, code, used_by, used_at, expires_at, revoked_at, max_uses, use_count,
                    created_at
             FROM invite_codes WHERE created_by = ?1
             ORDER BY created_at DESC, id DESC LIMIT 1",
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
    audit: Option<&SecurityAuditEvent>,
) -> Result<AuthUserRow, AuthRepositoryError> {
    let user_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    if user_count == 0 {
        return Err(AuthRepositoryError::AdministratorBootstrapRequired);
    }
    let invite_code = invite_code.ok_or(AuthRepositoryError::InviteCodeRequired)?;
    let user = insert_user_in_transaction(connection, username, password_hash, salt, false, now)?;
    let invite_id = connection
        .query_row(
            "UPDATE invite_codes
             SET used_by = CASE WHEN use_count = 0 THEN ?1 ELSE used_by END,
                 used_at = CASE WHEN use_count = 0 THEN ?2 ELSE used_at END,
                 use_count = use_count + 1
             WHERE code = ?3
               AND revoked_at IS NULL
               AND expires_at > ?2
               AND use_count < max_uses
             RETURNING id",
            params![user.id.value(), now, invite_code],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(AuthRepositoryError::InvalidOrUsedInviteCode)?;
    connection.execute(
        "INSERT INTO invite_code_uses (invite_code_id, user_id, used_at)
         VALUES (?1, ?2, ?3)",
        params![invite_id, user.id.value(), now],
    )?;
    let mut redemption_audit = SecurityAuditEvent::new("invite_redeem", "completed")
        .with_actor_id(user.id.value())
        .with_target_id(invite_id);
    redemption_audit.occurred_at = now;
    if let Some(audit) = audit {
        redemption_audit.request_id.clone_from(&audit.request_id);
    }
    insert_required_security_audit_event(connection, &redemption_audit)?;
    create_default_folder(connection, user.id, now)?;
    Ok(user)
}

fn validate_invite_policy(
    now: f64,
    expires_at: f64,
    max_uses: i64,
) -> Result<(), AuthRepositoryError> {
    if !is_valid_invite_code_policy(now, expires_at, max_uses) {
        return Err(AuthRepositoryError::InvalidInvitePolicy);
    }
    Ok(())
}

fn insert_invite_code_in_transaction(
    connection: &Connection,
    code: &str,
    created_by: Option<UserId>,
    now: f64,
    expires_at: f64,
    max_uses: i64,
) -> Result<InviteCodeRow, AuthRepositoryError> {
    connection.execute(
        "INSERT INTO invite_codes
             (code, created_by, created_at, expires_at, max_uses, use_count)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![
            code,
            created_by.map(UserId::value),
            now,
            expires_at,
            max_uses
        ],
    )?;
    Ok(InviteCodeRow {
        id: connection.last_insert_rowid(),
        code: code.to_string(),
        used_by: None,
        used_at: None,
        expires_at,
        revoked_at: None,
        max_uses,
        use_count: 0,
        created_at: now,
    })
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

fn ensure_token_generation(
    connection: &Connection,
    user_id: UserId,
    expected_token_generation: i64,
) -> Result<(), AuthRepositoryError> {
    let is_current = connection
        .query_row(
            "SELECT token_generation = ?2 FROM users WHERE id = ?1",
            params![user_id.value(), expected_token_generation],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    if is_current {
        Ok(())
    } else {
        Err(AuthRepositoryError::StaleAuthorization)
    }
}

fn ensure_authorizing_token(
    connection: &Connection,
    user_id: UserId,
    token_hash: &str,
    now: f64,
) -> Result<(), AuthRepositoryError> {
    let is_current = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM access_tokens
             WHERE user_id = ?1 AND token_hash = ?2 AND expires_at > ?3
         )",
        params![user_id.value(), token_hash, now],
        |row| row.get::<_, bool>(0),
    )?;
    if is_current {
        Ok(())
    } else {
        Err(AuthRepositoryError::StaleAuthorization)
    }
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
        token_generation: row.get(6)?,
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
        used_at: row.get(3)?,
        expires_at: row.get(4)?,
        revoked_at: row.get(5)?,
        max_uses: row.get(6)?,
        use_count: row.get(7)?,
        created_at: row.get(8)?,
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

fn is_transient_sqlite_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(failure.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    use litradar_domain::{UserId, ACCESS_TOKEN_ACTIVE_LIMIT, ACCESS_TOKEN_RESERVED_NAME};
    use rusqlite::params;
    use tempfile::{tempdir, TempDir};

    use super::{
        bootstrap_admin, compare_and_swap_legacy_password_hash,
        compare_and_swap_user_password_and_delete_tokens_with_audit, create_invite_code,
        delete_access_token, delete_all_access_tokens_with_audit, find_user_credentials_by_id,
        get_user_invite_code, initialize_auth_database, insert_personal_access_token,
        insert_personal_access_token_with_authorization_and_audit, issue_invite_code_with_audit,
        list_access_tokens, open_auth_connection, random_hex, register_user_with_invite,
        replace_login_access_token, replace_login_access_token_if_generation_matches_with_audit,
        revoke_user_invite_code_with_audit, rotate_user_invite_code_with_audit,
        update_user_password_and_delete_tokens, update_user_password_and_delete_tokens_with_audit,
        update_user_password_as_administrator_with_audit, verify_access_token_hash,
        AuthRepositoryError, AuthUserRow, InviteCodeRow, UserCredentialRow,
    };
    use crate::{list_security_audit_events, SecurityAuditEvent};

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
        assert_eq!(verified.user.id, user_id);
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

    #[test]
    fn access_token_expiring_at_now_is_rejected_and_removed() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "boundary-token-hash",
            "boundary",
            100.0,
            2.0,
        );

        let verified = verify_access_token_hash(&auth_db_path, "boundary-token-hash", 100.0)
            .expect("boundary token verification should run");

        assert_eq!(verified, None);
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "boundary-token-hash"),
            0
        );
    }

    #[test]
    fn token_issuance_rejects_authorization_observed_before_global_revocation() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "authorizing-token-hash",
            "authorizing",
            4_000_000_000.0,
            2.0,
        );
        let observed = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist");
        delete_all_access_tokens_with_audit(
            &auth_db_path,
            user_id,
            &SecurityAuditEvent::new("logout_all", "completed").with_actor_id(user_id.value()),
        )
        .expect("global revocation should commit");

        let personal_error = insert_personal_access_token_with_authorization_and_audit(
            &auth_db_path,
            user_id,
            observed.token_generation,
            Some("authorizing-token-hash"),
            "stale-personal-hash",
            "stale-personal",
            4_000_000_000.0,
            3.0,
            Some(
                &SecurityAuditEvent::new("token_create", "completed")
                    .with_actor_id(user_id.value()),
            ),
        )
        .expect_err("stale PAT authorization must not survive logout-all");
        let login_error = replace_login_access_token_if_generation_matches_with_audit(
            &auth_db_path,
            user_id,
            observed.token_generation,
            "stale-login-hash",
            4_000_000_000.0,
            3.0,
            Some(&SecurityAuditEvent::new("login", "completed").with_actor_id(user_id.value())),
        )
        .expect_err("stale password authorization must not survive logout-all");
        let current = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should reload")
            .expect("fixture user should remain");
        let audits = list_security_audit_events(&auth_db_path).expect("audits should load");

        assert!(matches!(
            personal_error,
            AuthRepositoryError::StaleAuthorization
        ));
        assert!(matches!(
            login_error,
            AuthRepositoryError::StaleAuthorization
        ));
        assert_eq!(current.token_generation, observed.token_generation + 1);
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "stale-personal-hash"),
            0
        );
        assert_eq!(count_tokens_by_hash(&auth_db_path, "stale-login-hash"), 0);
        assert_eq!(
            audits
                .iter()
                .filter(|event| event.action == "logout_all")
                .count(),
            1
        );
        assert!(!audits
            .iter()
            .any(|event| matches!(event.action.as_str(), "token_create" | "login")));
    }

    #[test]
    fn personal_token_issuance_rechecks_the_exact_authorizing_token() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        let token_id = insert_raw_access_token(
            &auth_db_path,
            user_id,
            "single-authorizing-hash",
            "authorizing",
            4_000_000_000.0,
            2.0,
        );
        let observed = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist");
        assert!(delete_access_token(&auth_db_path, user_id, token_id)
            .expect("authorizing token should be revoked"));

        let error = insert_personal_access_token_with_authorization_and_audit(
            &auth_db_path,
            user_id,
            observed.token_generation,
            Some("single-authorizing-hash"),
            "successor-hash",
            "successor",
            4_000_000_000.0,
            3.0,
            None,
        )
        .expect_err("a revoked authorizing token must not mint a successor");

        assert!(matches!(error, AuthRepositoryError::StaleAuthorization));
        assert_eq!(count_tokens_by_hash(&auth_db_path, "successor-hash"), 0);
    }

    #[test]
    fn credential_rotation_rolls_back_when_token_revocation_fails() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "rotation-token-hash",
            "integration",
            4_000_000_000.0,
            2.0,
        );
        let original = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist");
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER fail_credential_token_revoke \
                 BEFORE DELETE ON access_tokens \
                 WHEN OLD.user_id = {} \
                 BEGIN SELECT RAISE(ABORT, 'injected token revoke failure'); END;",
                user_id.value()
            ))
            .expect("fault trigger should install");
        drop(connection);

        let error = update_user_password_and_delete_tokens(
            &auth_db_path,
            user_id,
            "replacement-password-hash",
            "replacement-salt",
            3.0,
        )
        .expect_err("injected token deletion failure should abort rotation");
        let after_failure = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should reload")
            .expect("fixture user should remain");

        assert!(matches!(error, AuthRepositoryError::Sqlite(_)));
        assert_eq!(after_failure, original);
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "rotation-token-hash"),
            1
        );

        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        connection
            .execute("DROP TRIGGER fail_credential_token_revoke", [])
            .expect("fault trigger should drop");
        drop(connection);
        assert!(update_user_password_and_delete_tokens(
            &auth_db_path,
            user_id,
            "replacement-password-hash",
            "replacement-salt",
            4.0,
        )
        .expect("credential rotation should commit"));
        let after_success = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should reload")
            .expect("fixture user should remain");
        assert_eq!(after_success.password_hash, "replacement-password-hash");
        assert_eq!(after_success.salt, "replacement-salt");
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "rotation-token-hash"),
            0
        );
        assert!(!update_user_password_and_delete_tokens(
            &auth_db_path,
            UserId(i64::MAX),
            "unused-password-hash",
            "unused-salt",
            5.0,
        )
        .expect("missing-user rotation should be a committed no-op"));
    }

    #[test]
    fn credential_rotation_rolls_back_when_required_security_audit_insert_fails() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "audit-rollback-token-hash",
            "integration",
            4_000_000_000.0,
            2.0,
        );
        let original = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist");
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_required_security_audit \
                 BEFORE INSERT ON security_audit_events \
                 BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
            )
            .expect("audit fault trigger should install");
        drop(connection);

        let error = update_user_password_and_delete_tokens_with_audit(
            &auth_db_path,
            user_id,
            "replacement-password-hash",
            "replacement-salt",
            3.0,
            Some(
                &SecurityAuditEvent::new("password_change", "completed")
                    .with_actor_id(user_id.value()),
            ),
        )
        .expect_err("required audit failure should abort credential rotation");
        let after_failure = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should reload")
            .expect("fixture user should remain");

        assert!(matches!(error, AuthRepositoryError::AuditPersistence(_)));
        assert_eq!(after_failure, original);
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "audit-rollback-token-hash"),
            1
        );
        assert!(list_security_audit_events(&auth_db_path)
            .expect("audit rows should remain readable")
            .is_empty());
    }

    #[test]
    fn password_change_cas_has_one_winner_and_no_stale_side_effects() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "pre-race-token-hash",
            "integration",
            4_000_000_000.0,
            2.0,
        );
        let original = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist");
        let barrier = Arc::new(Barrier::new(3));
        let handles = [
            ("replacement-password-a", 3.0),
            ("replacement-password-b", 4.0),
        ]
        .into_iter()
        .map(|(replacement_hash, now)| {
            let auth_db_path = auth_db_path.clone();
            let expected_hash = original.password_hash.clone();
            let expected_salt = original.salt.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let audit = SecurityAuditEvent::new("password_change", "completed")
                    .with_actor_id(user_id.value())
                    .with_target_id(user_id.value());
                let did_update = compare_and_swap_user_password_and_delete_tokens_with_audit(
                    &auth_db_path,
                    user_id,
                    &expected_hash,
                    &expected_salt,
                    replacement_hash,
                    "",
                    now,
                    &audit,
                )
                .expect("password CAS should complete");
                (replacement_hash, did_update)
            })
        })
        .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("password CAS thread should finish"))
            .collect::<Vec<_>>();
        let winner = results
            .iter()
            .find_map(|(replacement_hash, did_update)| did_update.then_some(*replacement_hash))
            .expect("one password CAS should win");

        assert_eq!(
            results.iter().filter(|(_, did_update)| *did_update).count(),
            1
        );
        let current = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should reload")
            .expect("fixture user should remain");
        assert_eq!(current.password_hash, winner);
        assert!(current.salt.is_empty());
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "pre-race-token-hash"),
            0
        );
        assert_eq!(
            list_security_audit_events(&auth_db_path)
                .expect("audit rows should load")
                .len(),
            1
        );

        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "post-race-token-hash",
            "post-race",
            4_000_000_000.0,
            5.0,
        );
        let stale_audit = SecurityAuditEvent::new("password_change", "completed")
            .with_actor_id(user_id.value())
            .with_target_id(user_id.value());
        assert!(
            !compare_and_swap_user_password_and_delete_tokens_with_audit(
                &auth_db_path,
                user_id,
                &original.password_hash,
                &original.salt,
                "stale-replacement-password",
                "",
                6.0,
                &stale_audit,
            )
            .expect("stale password CAS should be a committed no-op")
        );
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "post-race-token-hash"),
            1
        );
        assert_eq!(
            list_security_audit_events(&auth_db_path)
                .expect("audit rows should reload")
                .len(),
            1
        );
        assert_eq!(
            find_user_credentials_by_id(&auth_db_path, user_id)
                .expect("credentials should reload")
                .expect("fixture user should remain"),
            current
        );
    }

    #[test]
    fn invite_registration_rejects_expired_revoked_and_exhausted_codes() {
        let (_temp_dir, auth_db_path, issuer_id) = access_token_fixture();
        issue_invite_code_with_audit(
            &auth_db_path,
            issuer_id,
            "expired-code",
            10.0,
            20.0,
            1,
            None,
        )
        .expect("expiring invite should be issued");
        let expired = register_user_with_invite(
            &auth_db_path,
            "expired_user",
            "password-hash",
            "salt",
            Some("expired-code"),
            20.0,
        )
        .expect_err("invite expiring at registration time should be rejected");
        assert!(matches!(
            expired,
            AuthRepositoryError::InvalidOrUsedInviteCode
        ));

        rotate_user_invite_code_with_audit(
            &auth_db_path,
            issuer_id,
            "revoked-code",
            30.0,
            100.0,
            1,
            None,
        )
        .expect("replacement invite should be issued");
        assert!(
            revoke_user_invite_code_with_audit(&auth_db_path, issuer_id, 40.0, None,)
                .expect("invite revocation should commit")
        );
        let revoked = register_user_with_invite(
            &auth_db_path,
            "revoked_user",
            "password-hash",
            "salt",
            Some("revoked-code"),
            50.0,
        )
        .expect_err("revoked invite should be rejected");
        assert!(matches!(
            revoked,
            AuthRepositoryError::InvalidOrUsedInviteCode
        ));

        issue_invite_code_with_audit(
            &auth_db_path,
            issuer_id,
            "exhausted-code",
            60.0,
            100.0,
            1,
            None,
        )
        .expect("quota fixture invite should be issued");
        register_user_with_invite(
            &auth_db_path,
            "first_redeemer",
            "password-hash",
            "salt",
            Some("exhausted-code"),
            70.0,
        )
        .expect("first redemption should commit");
        let exhausted = register_user_with_invite(
            &auth_db_path,
            "second_redeemer",
            "password-hash",
            "salt",
            Some("exhausted-code"),
            80.0,
        )
        .expect_err("exhausted invite should be rejected");
        assert!(matches!(
            exhausted,
            AuthRepositoryError::InvalidOrUsedInviteCode
        ));
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))
                .expect("user count should load"),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM invite_code_uses", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("redemption count should load"),
            1
        );
    }

    #[test]
    fn invite_rotation_irreversibly_replaces_the_prior_code() {
        let (_temp_dir, auth_db_path, issuer_id) = access_token_fixture();
        create_invite_code(&auth_db_path, issuer_id, "prior-code", 10.0)
            .expect("prior invite should be issued");
        let replacement = rotate_user_invite_code_with_audit(
            &auth_db_path,
            issuer_id,
            "replacement-code",
            20.0,
            100.0,
            1,
            Some(
                &SecurityAuditEvent::new("invite_rotate", "completed")
                    .with_actor_id(issuer_id.value()),
            ),
        )
        .expect("invite rotation should commit");

        let prior_result = register_user_with_invite(
            &auth_db_path,
            "prior_redeemer",
            "password-hash",
            "salt",
            Some("prior-code"),
            30.0,
        );
        assert!(matches!(
            prior_result,
            Err(AuthRepositoryError::InvalidOrUsedInviteCode)
        ));
        register_user_with_invite(
            &auth_db_path,
            "replacement_redeemer",
            "password-hash",
            "salt",
            Some("replacement-code"),
            30.0,
        )
        .expect("replacement invite should remain usable");
        let current = get_user_invite_code(&auth_db_path, issuer_id)
            .expect("current invite should load")
            .expect("replacement invite should exist");
        assert_eq!(current.id, replacement.id);
        assert_eq!(current.code, "replacement-code");
        assert_eq!(current.use_count, 1);
        let audits =
            list_security_audit_events(&auth_db_path).expect("invite lifecycle audits should load");
        assert_eq!(
            audits
                .iter()
                .filter(|event| event.action == "invite_rotate")
                .count(),
            2
        );
        assert_eq!(
            audits
                .iter()
                .filter(|event| event.action == "invite_redeem")
                .count(),
            1
        );
    }

    #[test]
    fn invite_redemption_and_rotation_roll_back_when_required_audit_fails() {
        let (_temp_dir, auth_db_path, issuer_id) = access_token_fixture();
        create_invite_code(&auth_db_path, issuer_id, "rollback-prior-code", 10.0)
            .expect("prior invite should be issued");
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_invite_lifecycle_audit
                 BEFORE INSERT ON security_audit_events
                 WHEN NEW.action IN ('invite_redeem', 'invite_rotate')
                 BEGIN SELECT RAISE(ABORT, 'injected invite audit failure'); END;",
            )
            .expect("invite audit fault trigger should install");
        drop(connection);

        let redemption_error = register_user_with_invite(
            &auth_db_path,
            "rollback_redeemer",
            "password-hash",
            "salt",
            Some("rollback-prior-code"),
            20.0,
        )
        .expect_err("required redemption audit failure should abort registration");
        assert!(matches!(
            redemption_error,
            AuthRepositoryError::AuditPersistence(_)
        ));
        let rotation_error = rotate_user_invite_code_with_audit(
            &auth_db_path,
            issuer_id,
            "rollback-replacement-code",
            30.0,
            100.0,
            1,
            Some(
                &SecurityAuditEvent::new("invite_rotate", "completed")
                    .with_actor_id(issuer_id.value()),
            ),
        )
        .expect_err("required rotation audit failure should abort replacement");
        assert!(matches!(
            rotation_error,
            AuthRepositoryError::AuditPersistence(_)
        ));

        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        let prior_state = connection
            .query_row(
                "SELECT use_count, revoked_at FROM invite_codes WHERE code = 'rollback-prior-code'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<f64>>(1)?)),
            )
            .expect("prior invite state should load");
        assert_eq!(prior_state, (0, None));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM invite_codes WHERE code = 'rollback-replacement-code'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("replacement count should load"),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM users WHERE username = 'rollback_redeemer'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("rolled-back user count should load"),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM invite_code_uses", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("rolled-back redemption count should load"),
            0
        );
    }

    #[test]
    fn concurrent_invite_issuance_admits_only_one_unrevoked_code() {
        let (_temp_dir, auth_db_path, issuer_id) = access_token_fixture();
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let auth_db_path = auth_db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    create_invite_code(
                        auth_db_path,
                        issuer_id,
                        &format!("concurrent-code-{index}"),
                        10.0 + index as f64,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("issuance thread should finish"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(
                    result,
                    Err(AuthRepositoryError::ActiveInviteCodeAlreadyExists)
                ))
                .count(),
            1
        );
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM invite_codes
                     WHERE created_by = ?1 AND revoked_at IS NULL",
                    [issuer_id.value()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("active invite count should load"),
            1
        );
    }

    #[test]
    fn concurrent_final_invite_redemption_commits_exactly_once() {
        let (_temp_dir, auth_db_path, issuer_id) = access_token_fixture();
        issue_invite_code_with_audit(
            &auth_db_path,
            issuer_id,
            "last-use-code",
            10.0,
            100.0,
            1,
            None,
        )
        .expect("single-use invite should be issued");
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let auth_db_path = auth_db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    register_user_with_invite(
                        auth_db_path,
                        &format!("concurrent_redeemer_{index}"),
                        "password-hash",
                        "salt",
                        Some("last-use-code"),
                        20.0,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("redemption thread should finish"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(
                    result,
                    Err(AuthRepositoryError::InvalidOrUsedInviteCode)
                ))
                .count(),
            1
        );
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        let state = connection
            .query_row(
                "SELECT use_count,
                        (SELECT COUNT(*) FROM invite_code_uses WHERE invite_code_id = ic.id)
                 FROM invite_codes ic WHERE code = 'last-use-code'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("invite quota state should load");
        assert_eq!(state, (1, 1));
    }

    #[test]
    fn os_random_hex_and_auth_row_debug_keep_secret_boundaries() {
        let first = random_hex(32).expect("OS randomness should be available");
        let second = random_hex(32).expect("OS randomness should remain available");
        assert_eq!(first.len(), 64);
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_ne!(first, second);

        let debug = format!(
            "{:?}",
            (
                AuthUserRow {
                    id: UserId(1),
                    username: "auth-user-name-sentinel".to_string(),
                    is_admin: true,
                    created_at: 1.0,
                },
                UserCredentialRow {
                    id: UserId(1),
                    username: "credential-name-sentinel".to_string(),
                    password_hash: "password-hash-sentinel".to_string(),
                    salt: "password-salt-sentinel".to_string(),
                    is_admin: true,
                    created_at: 1.0,
                    token_generation: 0,
                },
                InviteCodeRow {
                    id: 2,
                    code: "invite-code-sentinel".to_string(),
                    used_by: None,
                    used_at: None,
                    expires_at: 3.0,
                    revoked_at: None,
                    max_uses: 1,
                    use_count: 0,
                    created_at: 2.0,
                },
            )
        );
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("auth-user-name-sentinel"));
        assert!(!debug.contains("credential-name-sentinel"));
        assert!(!debug.contains("password-hash-sentinel"));
        assert!(!debug.contains("password-salt-sentinel"));
        assert!(!debug.contains("invite-code-sentinel"));
    }

    #[test]
    fn legacy_password_compare_and_swap_updates_once_without_revoking_tokens() {
        let (_temp_dir, auth_db_path, user_id) = access_token_fixture();
        insert_raw_access_token(
            &auth_db_path,
            user_id,
            "legacy-upgrade-token",
            "integration",
            4_000_000_000.0,
            2.0,
        );
        let original = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist");
        let barrier = Arc::new(Barrier::new(2));
        let handles = ["$argon2id$fixture-one", "$argon2id$fixture-two"]
            .into_iter()
            .map(|replacement| {
                let auth_db_path = auth_db_path.clone();
                let original = original.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    compare_and_swap_legacy_password_hash(
                        auth_db_path,
                        user_id,
                        &original.password_hash,
                        &original.salt,
                        replacement,
                        3.0,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("CAS thread should finish")
                    .expect("CAS operation should run")
            })
            .collect::<Vec<_>>();
        let upgraded = find_user_credentials_by_id(&auth_db_path, user_id)
            .expect("credentials should reload")
            .expect("fixture user should remain");

        assert_eq!(results.iter().filter(|result| **result).count(), 1);
        assert!(matches!(
            upgraded.password_hash.as_str(),
            "$argon2id$fixture-one" | "$argon2id$fixture-two"
        ));
        assert_eq!(upgraded.salt, "");
        assert_eq!(
            count_tokens_by_hash(&auth_db_path, "legacy-upgrade-token"),
            1
        );
    }

    #[test]
    fn administrator_password_reset_rejects_actor_demoted_after_authorization() {
        let (_temp_dir, auth_db_path, actor_id) = access_token_fixture();
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        connection
            .execute(
                "INSERT INTO users \
                 (username, password_hash, salt, is_admin, created_at, updated_at) \
                 VALUES ('reset_target', 'original-hash', 'original-salt', 1, 1.0, 1.0)",
                [],
            )
            .expect("reset target should insert");
        let target_id = UserId(connection.last_insert_rowid());
        drop(connection);
        insert_raw_access_token(
            &auth_db_path,
            target_id,
            "reset-target-token",
            "integration",
            4_000_000_000.0,
            2.0,
        );
        let original = find_user_credentials_by_id(&auth_db_path, target_id)
            .expect("target credentials should load")
            .expect("reset target should exist");
        let audit = SecurityAuditEvent::new("user_password_reset", "completed")
            .with_actor_id(actor_id.value())
            .with_target_id(target_id.value());
        let connection = open_auth_connection(&auth_db_path).expect("auth connection should open");
        connection
            .execute(
                "UPDATE users SET is_admin = 0 WHERE id = ?1",
                [actor_id.value()],
            )
            .expect("actor demotion should commit");
        drop(connection);

        let result = update_user_password_as_administrator_with_audit(
            &auth_db_path,
            actor_id,
            target_id,
            "replacement-hash",
            "",
            3.0,
            &audit,
        );

        assert!(matches!(
            result,
            Err(AuthRepositoryError::AdministratorActorForbidden)
        ));
        assert_eq!(
            find_user_credentials_by_id(&auth_db_path, target_id)
                .expect("target credentials should reload")
                .expect("reset target should remain"),
            original
        );
        assert_eq!(count_tokens_by_hash(&auth_db_path, "reset-target-token"), 1);
        assert!(list_security_audit_events(&auth_db_path)
            .expect("audit rows should load")
            .is_empty());
    }
}
