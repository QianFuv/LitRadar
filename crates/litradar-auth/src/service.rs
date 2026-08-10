//! Authentication service operations built on storage repositories.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_domain::{
    InviteCodeResponse, InviteCodeStatus, TokenCreateResponse, TokenInfo, UserId, UserResponse,
    DEFAULT_INVITE_CODE_MAX_USES, DEFAULT_INVITE_CODE_TTL_SECONDS,
};
use litradar_storage::{
    bootstrap_admin_with_audit, compare_and_swap_legacy_password_hash,
    compare_and_swap_user_password_and_delete_tokens_with_audit, count_users,
    create_invite_code_with_audit, delete_access_token_by_hash_with_audit,
    delete_access_token_with_audit, delete_all_access_tokens_with_audit,
    find_user_credentials_by_id, find_user_credentials_by_username, get_user_invite_code,
    initialize_auth_database, insert_personal_access_token_with_authorization_and_audit,
    list_access_tokens, random_hex, register_user_with_invite_and_audit,
    replace_login_access_token_if_generation_matches_with_audit,
    revoke_user_invite_code_with_audit, rotate_user_invite_code_with_audit,
    update_user_password_and_delete_tokens_with_audit,
    update_user_password_as_administrator_with_audit, verify_access_token_hash,
    AuthRepositoryError, AuthUserRow, InviteCodeRow, SecurityAuditEvent, UserCredentialRow,
};

use crate::password::verify_dummy_password;
use crate::{
    hash_password, hash_token, is_valid_new_password, verify_password, PasswordError,
    PasswordVerification, ACCESS_TOKEN_NAME_LENGTH_DETAIL, ACCESS_TOKEN_NAME_MAX_CODE_POINTS,
    ACCESS_TOKEN_RESERVED_NAME, ACCESS_TOKEN_RESERVED_NAME_DETAIL, ACCESS_TOKEN_TTL_DETAIL,
    ACCESS_TOKEN_TTL_MAX_SECONDS, ACCESS_TOKEN_TTL_MIN_SECONDS, MIN_PASSWORD_LENGTH,
};

/// Python-compatible default access token TTL in seconds.
pub const ACCESS_TOKEN_DEFAULT_TTL: i64 = 7 * 24 * 3600;

const ACCESS_TOKEN_BYTES: usize = 32;
const INVITE_CODE_BYTES: usize = 8;
const MIN_USERNAME_LENGTH: usize = 3;
const MAX_USERNAME_LENGTH: usize = 32;

/// Authentication service error.
#[derive(Debug)]
pub enum AuthServiceError {
    /// Repository operation failed.
    Repository(AuthRepositoryError),
    /// Password hashing failed.
    Password(PasswordError),
    /// Credentials did not match a stored user.
    InvalidCredentials,
    /// Username does not satisfy the public account naming policy.
    InvalidUsername,
    /// A newly created or replaced password is too short.
    PasswordTooShort,
    /// The untrimmed personal access-token name exceeds the code-point limit.
    AccessTokenNameTooLong,
    /// The normalized personal access-token name is reserved for browser login.
    AccessTokenNameReserved,
    /// The requested personal access-token TTL is outside the accepted range.
    AccessTokenTtlOutOfRange,
}

impl fmt::Display for AuthServiceError {
    /// Format the service error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::Password(error) => write!(formatter, "{error}"),
            Self::InvalidCredentials => formatter.write_str("Invalid username or password"),
            Self::InvalidUsername => {
                formatter.write_str("Username must be 3-32 alphanumeric or underscore characters")
            }
            Self::PasswordTooShort => write!(
                formatter,
                "Password must be at least {MIN_PASSWORD_LENGTH} characters"
            ),
            Self::AccessTokenNameTooLong => formatter.write_str(ACCESS_TOKEN_NAME_LENGTH_DETAIL),
            Self::AccessTokenNameReserved => formatter.write_str(ACCESS_TOKEN_RESERVED_NAME_DETAIL),
            Self::AccessTokenTtlOutOfRange => formatter.write_str(ACCESS_TOKEN_TTL_DETAIL),
        }
    }
}

impl Error for AuthServiceError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::Password(error) => Some(error),
            Self::InvalidCredentials
            | Self::InvalidUsername
            | Self::PasswordTooShort
            | Self::AccessTokenNameTooLong
            | Self::AccessTokenNameReserved
            | Self::AccessTokenTtlOutOfRange => None,
        }
    }
}

impl AuthServiceError {
    /// Return whether an authentication operation hit transient SQLite lock contention.
    ///
    /// # Returns
    ///
    /// True only when one bounded retry is appropriate.
    pub fn is_transient_sqlite_contention(&self) -> bool {
        matches!(self, Self::Repository(error) if error.is_transient_sqlite_contention())
    }
}

impl From<AuthRepositoryError> for AuthServiceError {
    /// Convert repository errors into service errors.
    fn from(error: AuthRepositoryError) -> Self {
        Self::Repository(error)
    }
}

impl From<PasswordError> for AuthServiceError {
    /// Convert password hashing failures into service errors.
    fn from(error: PasswordError) -> Self {
        Self::Password(error)
    }
}

/// Created login session with the raw token kept out of JSON responses.
#[derive(Clone, PartialEq)]
pub struct LoginSession {
    /// Authenticated user.
    pub user: UserResponse,
    /// Raw token to set in the browser cookie.
    pub token: String,
    /// Token expiration timestamp.
    pub expires_at: f64,
}

impl fmt::Debug for LoginSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginSession")
            .field("user", &self.user)
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Authenticated access-token owner plus the generation observed during verification.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessTokenAuthorization {
    /// Authenticated user.
    pub user: UserResponse,
    /// Generation that dependent token issuance must compare atomically.
    pub token_generation: i64,
}

struct PasswordAuthorization {
    user: UserResponse,
    token_generation: i64,
}

/// Authentication service bound to one auth database.
#[derive(Debug, Clone)]
pub struct AuthService {
    auth_db_path: PathBuf,
}

impl AuthService {
    /// Build an auth service for an auth database path.
    ///
    /// # Arguments
    ///
    /// * `auth_db_path` - Path to `auth.sqlite`.
    ///
    /// # Returns
    ///
    /// Auth service instance.
    pub fn new(auth_db_path: impl AsRef<Path>) -> Self {
        Self {
            auth_db_path: auth_db_path.as_ref().to_path_buf(),
        }
    }

    /// Ensure auth database tables exist.
    ///
    /// # Returns
    ///
    /// Empty result on success.
    pub fn initialize(&self) -> Result<(), AuthServiceError> {
        initialize_auth_database(&self.auth_db_path)?;
        Ok(())
    }

    /// Register a user with Python-compatible invite behavior.
    ///
    /// # Arguments
    ///
    /// * `username` - Trimmed username.
    /// * `password` - Plain text password.
    /// * `invite_code` - Optional invite code.
    ///
    /// # Returns
    ///
    /// Created user response.
    pub fn register(
        &self,
        username: &str,
        password: &str,
        invite_code: Option<&str>,
    ) -> Result<UserResponse, AuthServiceError> {
        self.register_with_audit(
            username,
            password,
            invite_code,
            SecurityAuditEvent::new("register", "completed"),
        )
    }

    /// Register a user and persist the supplied completion audit atomically.
    pub fn register_with_audit(
        &self,
        username: &str,
        password: &str,
        invite_code: Option<&str>,
        audit: SecurityAuditEvent,
    ) -> Result<UserResponse, AuthServiceError> {
        validate_new_credentials(username, password)?;
        let password_hash = hash_password(password)?;
        let legacy_salt = String::new();
        let user = register_user_with_invite_and_audit(
            &self.auth_db_path,
            username,
            &password_hash,
            &legacy_salt,
            invite_code,
            now_seconds(),
            Some(&audit),
        )?;
        Ok(user_response(user))
    }

    /// Create the first administrator through the local bootstrap path.
    ///
    /// # Arguments
    ///
    /// * `username` - Administrator username.
    /// * `password` - Plain-text password read from standard input by the caller.
    ///
    /// # Returns
    ///
    /// Created administrator response.
    pub fn bootstrap_admin(
        &self,
        username: &str,
        password: &str,
    ) -> Result<UserResponse, AuthServiceError> {
        self.bootstrap_admin_with_audit(
            username,
            password,
            SecurityAuditEvent::new("admin_bootstrap", "completed"),
        )
    }

    /// Create the first administrator with an atomic completion audit.
    pub fn bootstrap_admin_with_audit(
        &self,
        username: &str,
        password: &str,
        audit: SecurityAuditEvent,
    ) -> Result<UserResponse, AuthServiceError> {
        validate_new_credentials(username, password)?;
        let password_hash = hash_password(password)?;
        let legacy_salt = String::new();
        let user = bootstrap_admin_with_audit(
            &self.auth_db_path,
            username,
            &password_hash,
            &legacy_salt,
            now_seconds(),
            Some(&audit),
        )?;
        Ok(user_response(user))
    }

    /// Verify username and password credentials.
    ///
    /// # Arguments
    ///
    /// * `username` - Trimmed username.
    /// * `password` - Plain text password.
    ///
    /// # Returns
    ///
    /// User response when credentials are valid.
    pub fn verify_user(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<UserResponse>, AuthServiceError> {
        Ok(self
            .verify_user_authorization(username, password)?
            .map(|authorization| authorization.user))
    }

    fn verify_user_authorization(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<PasswordAuthorization>, AuthServiceError> {
        let Some(row) = find_user_credentials_by_username(&self.auth_db_path, username)? else {
            verify_dummy_password(password);
            return Ok(None);
        };
        let token_generation = match verify_password(password, &row.salt, &row.password_hash) {
            PasswordVerification::Invalid => return Ok(None),
            PasswordVerification::ValidCurrent => row.token_generation,
            PasswordVerification::ValidLegacy => {
                let Some(token_generation) = self.upgrade_legacy_password(&row, password)? else {
                    return Ok(None);
                };
                token_generation
            }
        };
        Ok(Some(PasswordAuthorization {
            user: UserResponse {
                id: row.id,
                username: row.username,
                is_admin: row.is_admin,
            },
            token_generation,
        }))
    }

    fn upgrade_legacy_password(
        &self,
        legacy_row: &UserCredentialRow,
        password: &str,
    ) -> Result<Option<i64>, AuthServiceError> {
        let replacement_hash = hash_password(password)?;
        if compare_and_swap_legacy_password_hash(
            &self.auth_db_path,
            legacy_row.id,
            &legacy_row.password_hash,
            &legacy_row.salt,
            &replacement_hash,
            now_seconds(),
        )? {
            return Ok(Some(legacy_row.token_generation));
        }
        let Some(current) = find_user_credentials_by_id(&self.auth_db_path, legacy_row.id)? else {
            return Ok(None);
        };
        if verify_password(password, &current.salt, &current.password_hash)
            == PasswordVerification::Invalid
        {
            return Ok(None);
        }
        Ok(Some(current.token_generation))
    }

    /// Authenticate credentials and create a login session token.
    ///
    /// # Arguments
    ///
    /// * `username` - Trimmed username.
    /// * `password` - Plain text password.
    ///
    /// # Returns
    ///
    /// Created login session.
    pub fn login(&self, username: &str, password: &str) -> Result<LoginSession, AuthServiceError> {
        self.login_with_audit(
            username,
            password,
            SecurityAuditEvent::new("login", "completed"),
        )
    }

    /// Authenticate credentials and atomically audit the login token mutation.
    pub fn login_with_audit(
        &self,
        username: &str,
        password: &str,
        audit: SecurityAuditEvent,
    ) -> Result<LoginSession, AuthServiceError> {
        let authorization = self
            .verify_user_authorization(username, password)?
            .ok_or(AuthServiceError::InvalidCredentials)?;
        self.create_login_session(authorization, audit)
    }

    fn create_login_session(
        &self,
        authorization: PasswordAuthorization,
        audit: SecurityAuditEvent,
    ) -> Result<LoginSession, AuthServiceError> {
        let token = random_hex(ACCESS_TOKEN_BYTES)?;
        let token_hash = hash_token(&token);
        let created_at = now_seconds();
        let expires_at = created_at + ACCESS_TOKEN_DEFAULT_TTL as f64;
        let audit = audit.with_actor_id(authorization.user.id.value());
        let row = replace_login_access_token_if_generation_matches_with_audit(
            &self.auth_db_path,
            authorization.user.id,
            authorization.token_generation,
            &token_hash,
            expires_at,
            created_at,
            Some(&audit),
        )
        .map_err(|error| match error {
            AuthRepositoryError::StaleAuthorization => AuthServiceError::InvalidCredentials,
            error => AuthServiceError::Repository(error),
        })?;
        Ok(LoginSession {
            user: authorization.user,
            token,
            expires_at: row.expires_at,
        })
    }

    /// Create a raw access token and store only its hash.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Owner user identifier.
    /// * `name` - Untrimmed token display name.
    /// * `ttl` - Token TTL in seconds.
    ///
    /// # Returns
    ///
    /// Created token response including the raw token.
    pub fn create_access_token(
        &self,
        user_id: UserId,
        name: &str,
        ttl: i64,
    ) -> Result<TokenCreateResponse, AuthServiceError> {
        self.create_access_token_with_audit(
            user_id,
            name,
            ttl,
            SecurityAuditEvent::new("token_create", "completed").with_actor_id(user_id.value()),
        )
    }

    /// Create a personal access token with an atomic completion audit.
    pub fn create_access_token_with_audit(
        &self,
        user_id: UserId,
        name: &str,
        ttl: i64,
        audit: SecurityAuditEvent,
    ) -> Result<TokenCreateResponse, AuthServiceError> {
        let name = validate_access_token_request(name, ttl)?;
        let token_generation = find_user_credentials_by_id(&self.auth_db_path, user_id)?
            .ok_or(AuthRepositoryError::StaleAuthorization)?
            .token_generation;
        self.create_authorized_access_token(user_id, token_generation, None, name, ttl, audit)
    }

    /// Create a personal token only while the request's access-token authorization is current.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Owner user identifier.
    /// * `token_generation` - Generation captured while authenticating the request.
    /// * `authorizing_token` - Raw token that authenticated the request.
    /// * `name` - Untrimmed token display name.
    /// * `ttl` - Token TTL in seconds.
    /// * `audit` - Completion audit persisted with issuance.
    ///
    /// # Returns
    ///
    /// Created token, or a stale-authorization error without a mutation.
    #[allow(clippy::too_many_arguments)]
    pub fn create_access_token_with_authorization_and_audit(
        &self,
        user_id: UserId,
        token_generation: i64,
        authorizing_token: &str,
        name: &str,
        ttl: i64,
        audit: SecurityAuditEvent,
    ) -> Result<TokenCreateResponse, AuthServiceError> {
        let name = validate_access_token_request(name, ttl)?;
        let authorizing_token_hash = hash_token(authorizing_token);
        self.create_authorized_access_token(
            user_id,
            token_generation,
            Some(&authorizing_token_hash),
            name,
            ttl,
            audit,
        )
    }

    fn create_authorized_access_token(
        &self,
        user_id: UserId,
        token_generation: i64,
        authorizing_token_hash: Option<&str>,
        name: &str,
        ttl: i64,
        audit: SecurityAuditEvent,
    ) -> Result<TokenCreateResponse, AuthServiceError> {
        let token = random_hex(ACCESS_TOKEN_BYTES)?;
        let token_hash = hash_token(&token);
        let created_at = now_seconds();
        let expires_at = created_at + ttl as f64;
        let row = insert_personal_access_token_with_authorization_and_audit(
            &self.auth_db_path,
            user_id,
            token_generation,
            authorizing_token_hash,
            &token_hash,
            name,
            expires_at,
            created_at,
            Some(&audit),
        )?;
        Ok(TokenCreateResponse {
            id: row.id,
            token,
            name: row.name,
            expires_at: row.expires_at,
        })
    }

    /// Verify a raw access token.
    ///
    /// # Arguments
    ///
    /// * `token` - Raw bearer or cookie token.
    ///
    /// # Returns
    ///
    /// User response when the token is valid.
    pub fn verify_access_token(
        &self,
        token: &str,
    ) -> Result<Option<UserResponse>, AuthServiceError> {
        Ok(self
            .verify_access_token_authorization(token)?
            .map(|authorization| authorization.user))
    }

    /// Verify a raw access token and capture its issuance generation.
    ///
    /// # Arguments
    ///
    /// * `token` - Raw bearer or cookie token.
    ///
    /// # Returns
    ///
    /// Authorization context when the token is valid.
    pub fn verify_access_token_authorization(
        &self,
        token: &str,
    ) -> Result<Option<AccessTokenAuthorization>, AuthServiceError> {
        let token_hash = hash_token(token);
        let authorization =
            verify_access_token_hash(&self.auth_db_path, &token_hash, now_seconds())?;
        Ok(authorization.map(|authorization| AccessTokenAuthorization {
            user: user_response(authorization.user),
            token_generation: authorization.token_generation,
        }))
    }

    /// List active non-login access tokens.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Owner user identifier.
    ///
    /// # Returns
    ///
    /// Token metadata responses.
    pub fn list_access_tokens(&self, user_id: UserId) -> Result<Vec<TokenInfo>, AuthServiceError> {
        let rows = list_access_tokens(&self.auth_db_path, user_id, now_seconds())?;
        Ok(rows
            .into_iter()
            .map(|row| TokenInfo {
                id: row.id,
                name: row.name,
                expires_at: row.expires_at,
                created_at: row.created_at,
            })
            .collect())
    }

    /// Revoke one token by row id.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Owner user identifier.
    /// * `token_id` - Token row identifier.
    ///
    /// # Returns
    ///
    /// True when a token was revoked.
    pub fn revoke_access_token(
        &self,
        user_id: UserId,
        token_id: i64,
    ) -> Result<bool, AuthServiceError> {
        self.revoke_access_token_with_audit(
            user_id,
            token_id,
            SecurityAuditEvent::new("token_revoke", "completed")
                .with_actor_id(user_id.value())
                .with_target_id(token_id),
        )
    }

    /// Revoke a personal token with an atomic completion audit.
    pub fn revoke_access_token_with_audit(
        &self,
        user_id: UserId,
        token_id: i64,
        audit: SecurityAuditEvent,
    ) -> Result<bool, AuthServiceError> {
        Ok(delete_access_token_with_audit(
            &self.auth_db_path,
            user_id,
            token_id,
            Some(&audit),
        )?)
    }

    /// Revoke one token by raw token value.
    ///
    /// # Arguments
    ///
    /// * `token` - Raw token value.
    ///
    /// # Returns
    ///
    /// True when a token was revoked.
    pub fn revoke_access_token_value(&self, token: &str) -> Result<bool, AuthServiceError> {
        self.revoke_access_token_value_with_audit(
            token,
            SecurityAuditEvent::new("logout", "completed"),
        )
    }

    /// Revoke a raw token with an atomic completion audit.
    pub fn revoke_access_token_value_with_audit(
        &self,
        token: &str,
        audit: SecurityAuditEvent,
    ) -> Result<bool, AuthServiceError> {
        let token_hash = hash_token(token);
        Ok(delete_access_token_by_hash_with_audit(
            &self.auth_db_path,
            &token_hash,
            Some(&audit),
        )?)
    }

    /// Revoke every login and personal access token for one user with an atomic audit event.
    ///
    /// # Arguments
    ///
    /// * `user_id` - User whose sessions and personal access tokens must be revoked.
    /// * `audit` - Required terminal security audit event.
    ///
    /// # Returns
    ///
    /// Number of revoked tokens.
    pub fn revoke_all_access_tokens_with_audit(
        &self,
        user_id: UserId,
        audit: SecurityAuditEvent,
    ) -> Result<usize, AuthServiceError> {
        Ok(delete_all_access_tokens_with_audit(
            &self.auth_db_path,
            user_id,
            &audit,
        )?)
    }

    /// Change a user's password and revoke all active tokens.
    ///
    /// # Arguments
    ///
    /// * `user_id` - User identifier.
    /// * `old_password` - Current password.
    /// * `new_password` - Replacement password.
    ///
    /// # Returns
    ///
    /// True when the old password matched and the change was applied.
    pub fn change_password(
        &self,
        user_id: UserId,
        old_password: &str,
        new_password: &str,
    ) -> Result<bool, AuthServiceError> {
        self.change_password_with_audit(
            user_id,
            old_password,
            new_password,
            SecurityAuditEvent::new("password_change", "completed")
                .with_actor_id(user_id.value())
                .with_target_id(user_id.value()),
        )
    }

    /// Change a password and atomically audit credential rotation.
    pub fn change_password_with_audit(
        &self,
        user_id: UserId,
        old_password: &str,
        new_password: &str,
        audit: SecurityAuditEvent,
    ) -> Result<bool, AuthServiceError> {
        validate_new_password(new_password)?;
        let Some(row) = find_user_credentials_by_id(&self.auth_db_path, user_id)? else {
            return Ok(false);
        };
        if verify_password(old_password, &row.salt, &row.password_hash)
            == PasswordVerification::Invalid
        {
            return Ok(false);
        }
        let password_hash = hash_password(new_password)?;
        let did_update = compare_and_swap_user_password_and_delete_tokens_with_audit(
            &self.auth_db_path,
            user_id,
            &row.password_hash,
            &row.salt,
            &password_hash,
            "",
            now_seconds(),
            &audit,
        )?;
        Ok(did_update)
    }

    /// Reset a user's password without requiring the old password.
    ///
    /// # Arguments
    ///
    /// * `user_id` - User identifier.
    /// * `new_password` - Replacement password.
    ///
    /// # Returns
    ///
    /// True when the user exists and the reset was applied.
    pub fn reset_password(
        &self,
        user_id: UserId,
        new_password: &str,
    ) -> Result<bool, AuthServiceError> {
        self.reset_password_with_audit(
            user_id,
            new_password,
            SecurityAuditEvent::new("user_password_reset", "completed")
                .with_target_id(user_id.value()),
        )
    }

    /// Reset a password and atomically audit credential rotation.
    pub fn reset_password_with_audit(
        &self,
        user_id: UserId,
        new_password: &str,
        audit: SecurityAuditEvent,
    ) -> Result<bool, AuthServiceError> {
        validate_new_password(new_password)?;
        let password_hash = hash_password(new_password)?;
        let legacy_salt = String::new();
        Ok(update_user_password_and_delete_tokens_with_audit(
            &self.auth_db_path,
            user_id,
            &password_hash,
            &legacy_salt,
            now_seconds(),
            Some(&audit),
        )?)
    }

    /// Reset a password only while the requesting actor remains an administrator.
    ///
    /// # Arguments
    ///
    /// * `actor_id` - Administrator requesting the reset.
    /// * `user_id` - Target user identifier.
    /// * `new_password` - Replacement password.
    /// * `audit` - Required completion audit event.
    ///
    /// # Returns
    ///
    /// True when the actor remained authorized and the target reset committed.
    pub fn reset_password_as_administrator_with_audit(
        &self,
        actor_id: UserId,
        user_id: UserId,
        new_password: &str,
        audit: SecurityAuditEvent,
    ) -> Result<bool, AuthServiceError> {
        validate_new_password(new_password)?;
        let password_hash = hash_password(new_password)?;
        let legacy_salt = String::new();
        Ok(update_user_password_as_administrator_with_audit(
            &self.auth_db_path,
            actor_id,
            user_id,
            &password_hash,
            &legacy_salt,
            now_seconds(),
            &audit,
        )?)
    }

    /// Create a one-time invite code for a user.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    ///
    /// # Returns
    ///
    /// Invite code response.
    pub fn create_invite_code(
        &self,
        user_id: UserId,
    ) -> Result<InviteCodeResponse, AuthServiceError> {
        self.create_invite_code_with_audit(
            user_id,
            SecurityAuditEvent::new("invite_create", "completed").with_actor_id(user_id.value()),
        )
    }

    /// Create an invite code with an atomic completion audit.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    /// * `audit` - Completion audit event persisted in the same transaction.
    ///
    /// # Returns
    ///
    /// Invite code response.
    pub fn create_invite_code_with_audit(
        &self,
        user_id: UserId,
        audit: SecurityAuditEvent,
    ) -> Result<InviteCodeResponse, AuthServiceError> {
        let code = random_hex(INVITE_CODE_BYTES)?;
        let now = now_seconds();
        let row =
            create_invite_code_with_audit(&self.auth_db_path, user_id, &code, now, Some(&audit))?;
        Ok(invite_response(row, now))
    }

    /// Revoke the current user's unrevoked invite issuance.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    ///
    /// # Returns
    ///
    /// True when an unrevoked invite was changed.
    pub fn revoke_invite_code(&self, user_id: UserId) -> Result<bool, AuthServiceError> {
        self.revoke_invite_code_with_audit(
            user_id,
            SecurityAuditEvent::new("invite_revoke", "completed").with_actor_id(user_id.value()),
        )
    }

    /// Revoke the current user's unrevoked invite issuance with an atomic audit event.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    /// * `audit` - Completion audit event persisted in the same transaction.
    ///
    /// # Returns
    ///
    /// True when an unrevoked invite was changed.
    pub fn revoke_invite_code_with_audit(
        &self,
        user_id: UserId,
        audit: SecurityAuditEvent,
    ) -> Result<bool, AuthServiceError> {
        Ok(revoke_user_invite_code_with_audit(
            &self.auth_db_path,
            user_id,
            now_seconds(),
            Some(&audit),
        )?)
    }

    /// Rotate the current user's invite code with the default lifecycle policy.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    ///
    /// # Returns
    ///
    /// Newly issued replacement invite.
    pub fn rotate_invite_code(
        &self,
        user_id: UserId,
    ) -> Result<InviteCodeResponse, AuthServiceError> {
        self.rotate_invite_code_with_audit(
            user_id,
            SecurityAuditEvent::new("invite_rotate", "completed").with_actor_id(user_id.value()),
        )
    }

    /// Rotate the current user's invite code with an atomic audit event.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    /// * `audit` - Completion audit event persisted with the lifecycle transition.
    ///
    /// # Returns
    ///
    /// Newly issued replacement invite.
    pub fn rotate_invite_code_with_audit(
        &self,
        user_id: UserId,
        audit: SecurityAuditEvent,
    ) -> Result<InviteCodeResponse, AuthServiceError> {
        let code = random_hex(INVITE_CODE_BYTES)?;
        let now = now_seconds();
        let row = rotate_user_invite_code_with_audit(
            &self.auth_db_path,
            user_id,
            &code,
            now,
            now + DEFAULT_INVITE_CODE_TTL_SECONDS as f64,
            DEFAULT_INVITE_CODE_MAX_USES,
            Some(&audit),
        )?;
        Ok(invite_response(row, now))
    }

    /// Return the invite code created by a user.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Invite creator.
    ///
    /// # Returns
    ///
    /// Invite code response or None.
    pub fn get_user_invite_code(
        &self,
        user_id: UserId,
    ) -> Result<Option<InviteCodeResponse>, AuthServiceError> {
        let now = now_seconds();
        Ok(get_user_invite_code(&self.auth_db_path, user_id)?.map(|row| invite_response(row, now)))
    }

    /// Return whether registration requires an invite code.
    ///
    /// # Returns
    ///
    /// True because public registration always requires an invite code.
    pub fn is_invite_required(&self) -> Result<bool, AuthServiceError> {
        Ok(true)
    }

    /// Return whether local administrator bootstrap is still required.
    ///
    /// # Returns
    ///
    /// True when the database contains no users.
    pub fn is_bootstrap_required(&self) -> Result<bool, AuthServiceError> {
        Ok(count_users(&self.auth_db_path)? == 0)
    }
}

/// Return whether a username satisfies the account naming policy.
///
/// # Arguments
///
/// * `username` - Proposed normalized username.
///
/// # Returns
///
/// True for 3-32 ASCII letters, digits, or underscores.
pub fn is_valid_username(username: &str) -> bool {
    (MIN_USERNAME_LENGTH..=MAX_USERNAME_LENGTH).contains(&username.len())
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn validate_new_credentials(username: &str, password: &str) -> Result<(), AuthServiceError> {
    if !is_valid_username(username) {
        return Err(AuthServiceError::InvalidUsername);
    }
    validate_new_password(password)
}

fn validate_new_password(password: &str) -> Result<(), AuthServiceError> {
    if !is_valid_new_password(password) {
        return Err(AuthServiceError::PasswordTooShort);
    }
    Ok(())
}

fn validate_access_token_request(name: &str, ttl: i64) -> Result<&str, AuthServiceError> {
    if name.chars().count() > ACCESS_TOKEN_NAME_MAX_CODE_POINTS {
        return Err(AuthServiceError::AccessTokenNameTooLong);
    }
    let name = name.trim();
    if name == ACCESS_TOKEN_RESERVED_NAME {
        return Err(AuthServiceError::AccessTokenNameReserved);
    }
    if !(ACCESS_TOKEN_TTL_MIN_SECONDS..=ACCESS_TOKEN_TTL_MAX_SECONDS).contains(&ttl) {
        return Err(AuthServiceError::AccessTokenTtlOutOfRange);
    }
    Ok(name)
}

fn user_response(row: AuthUserRow) -> UserResponse {
    UserResponse {
        id: row.id,
        username: row.username,
        is_admin: row.is_admin,
    }
}

fn invite_response(row: InviteCodeRow, now: f64) -> InviteCodeResponse {
    InviteCodeResponse {
        id: row.id,
        code: row.code,
        used: row.use_count > 0,
        status: InviteCodeStatus::from_lifecycle(
            row.expires_at,
            row.revoked_at,
            row.max_uses,
            row.use_count,
            now,
        ),
        expires_at: row.expires_at,
        revoked_at: row.revoked_at,
        max_uses: row.max_uses,
        use_count: row.use_count,
        created_at: row.created_at,
    }
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    use litradar_domain::{InviteCodeStatus, UserId, DEFAULT_INVITE_CODE_TTL_SECONDS};
    use litradar_storage::{
        bootstrap_admin, count_users, find_user_credentials_by_id, migrate_auth_database,
        set_user_admin, AuthRepositoryError, SecurityAuditEvent, UserCredentialRow,
    };
    use tempfile::tempdir;

    use super::{AuthService, AuthServiceError};
    use crate::password::test_support::{kdf_invocations, reset_kdf_invocations};
    use crate::{
        hash_legacy_password, ACCESS_TOKEN_NAME_LENGTH_DETAIL, ACCESS_TOKEN_RESERVED_NAME_DETAIL,
        ACCESS_TOKEN_TTL_DETAIL, ACCESS_TOKEN_TTL_MAX_SECONDS, ACCESS_TOKEN_TTL_MIN_SECONDS,
    };

    const STRONG_PASSWORD: &str = "strong-password";

    fn credentials(auth_db_path: &Path, user_id: UserId) -> UserCredentialRow {
        find_user_credentials_by_id(auth_db_path, user_id)
            .expect("credentials should load")
            .expect("fixture user should exist")
    }

    #[test]
    fn auth_service_rejects_weak_new_passwords() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);

        let error = service
            .bootstrap_admin("admin", "short")
            .expect_err("weak bootstrap password should fail");

        assert!(matches!(error, AuthServiceError::PasswordTooShort));
        assert_eq!(
            count_users(&auth_db_path).expect("user count should load"),
            0
        );
    }

    #[test]
    fn legacy_password_login_upgrades_once_and_preserves_the_session() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let salt = "legacy-salt";
        let password_hash = hash_legacy_password("short", salt);
        bootstrap_admin(&auth_db_path, "legacy-admin", &password_hash, salt, 1.0)
            .expect("legacy administrator should be inserted");
        let service = AuthService::new(&auth_db_path);

        let session = service
            .login("legacy-admin", "short")
            .expect("legacy credentials should create a session");
        let upgraded = credentials(&auth_db_path, session.user.id);

        assert!(session.user.is_admin);
        assert!(upgraded.password_hash.starts_with("$argon2id$"));
        assert_eq!(upgraded.salt, "");
        assert!(service
            .verify_access_token(&session.token)
            .expect("upgraded session should verify")
            .is_some());

        service
            .verify_user("legacy-admin", "short")
            .expect("current PHC credentials should verify")
            .expect("legacy user should remain available");
        assert_eq!(
            credentials(&auth_db_path, session.user.id).password_hash,
            upgraded.password_hash
        );
    }

    #[test]
    fn legacy_wrong_password_never_upgrades_and_concurrent_successes_remain_valid() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let salt = "legacy-concurrent-salt";
        let password_hash = hash_legacy_password(STRONG_PASSWORD, salt);
        let user = bootstrap_admin(
            &auth_db_path,
            "legacy-concurrent",
            &password_hash,
            salt,
            1.0,
        )
        .expect("legacy administrator should be inserted");
        let service = AuthService::new(&auth_db_path);

        assert!(service
            .verify_user("legacy-concurrent", "wrong-password")
            .expect("wrong legacy password should run")
            .is_none());
        assert_eq!(
            credentials(&auth_db_path, user.id).password_hash,
            password_hash
        );
        assert_eq!(credentials(&auth_db_path, user.id).salt, salt);

        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let service = service.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    service.verify_user("legacy-concurrent", STRONG_PASSWORD)
                })
            })
            .collect::<Vec<_>>();
        let verified = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("legacy verification thread should finish")
                    .expect("legacy verification should not fail")
            })
            .collect::<Vec<_>>();
        let upgraded = credentials(&auth_db_path, user.id);

        assert!(verified.iter().all(Option::is_some));
        assert!(upgraded.password_hash.starts_with("$argon2id$"));
        assert_eq!(upgraded.salt, "");
    }

    #[test]
    fn legacy_upgrade_rechecks_password_after_concurrent_reset() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let salt = "legacy-reset-salt";
        let password_hash = hash_legacy_password(STRONG_PASSWORD, salt);
        let user = bootstrap_admin(&auth_db_path, "legacy-reset", &password_hash, salt, 1.0)
            .expect("legacy administrator should be inserted");
        let service = AuthService::new(&auth_db_path);
        let stale_credentials = credentials(&auth_db_path, user.id);

        assert!(service
            .reset_password(user.id, "replacement-password")
            .expect("concurrent reset should run"));
        assert!(service
            .upgrade_legacy_password(&stale_credentials, STRONG_PASSWORD)
            .expect("stale legacy upgrade should recheck current credentials")
            .is_none());
        assert!(service
            .verify_user("legacy-reset", STRONG_PASSWORD)
            .expect("old password verification should run")
            .is_none());
        assert!(service
            .verify_user("legacy-reset", "replacement-password")
            .expect("replacement password verification should run")
            .is_some());
    }

    #[test]
    fn missing_and_wrong_password_paths_each_invoke_argon2id() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        service
            .bootstrap_admin("current_admin", STRONG_PASSWORD)
            .expect("current administrator should bootstrap");

        reset_kdf_invocations();
        assert!(service
            .verify_user("missing_user", "submitted-password")
            .expect("missing-user verification should run")
            .is_none());
        assert_eq!(kdf_invocations(), 1);

        reset_kdf_invocations();
        assert!(service
            .verify_user("current_admin", "submitted-password")
            .expect("wrong-password verification should run")
            .is_none());
        assert_eq!(kdf_invocations(), 1);
    }

    #[test]
    fn new_registration_change_and_reset_store_argon2id_phc() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let admin = service
            .bootstrap_admin("current_admin", STRONG_PASSWORD)
            .expect("current administrator should bootstrap");
        let bootstrap_credentials = credentials(&auth_db_path, admin.id);
        assert!(bootstrap_credentials
            .password_hash
            .starts_with("$argon2id$"));
        assert_eq!(bootstrap_credentials.salt, "");

        let invite = service
            .create_invite_code(admin.id)
            .expect("invite should be created");
        let user = service
            .register("current_user", "registration-password", Some(&invite.code))
            .expect("current user should register");
        let registered = credentials(&auth_db_path, user.id);
        assert!(registered.password_hash.starts_with("$argon2id$"));
        assert_eq!(registered.salt, "");

        assert!(service
            .change_password(user.id, "registration-password", "changed-password")
            .expect("password change should run"));
        let changed = credentials(&auth_db_path, user.id);
        assert!(changed.password_hash.starts_with("$argon2id$"));
        assert_eq!(changed.salt, "");
        assert_ne!(changed.password_hash, registered.password_hash);

        assert!(service
            .reset_password(user.id, "administrator-reset-password")
            .expect("password reset should run"));
        let reset = credentials(&auth_db_path, user.id);
        assert!(reset.password_hash.starts_with("$argon2id$"));
        assert_eq!(reset.salt, "");
        assert_ne!(reset.password_hash, changed.password_hash);
    }

    #[test]
    fn invite_service_returns_default_rotate_and_revoke_lifecycle_states() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let administrator = service
            .bootstrap_admin("invite_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");

        let initial = service
            .create_invite_code(administrator.id)
            .expect("initial invite should be created");
        assert_eq!(initial.status, InviteCodeStatus::Active);
        assert_eq!(initial.max_uses, 1);
        assert_eq!(initial.use_count, 0);
        assert!(
            (initial.expires_at - initial.created_at - DEFAULT_INVITE_CODE_TTL_SECONDS as f64)
                .abs()
                < 0.001
        );

        let replacement = service
            .rotate_invite_code(administrator.id)
            .expect("invite should rotate");
        assert_ne!(replacement.id, initial.id);
        assert_ne!(replacement.code, initial.code);
        assert_eq!(replacement.status, InviteCodeStatus::Active);
        assert!(service
            .revoke_invite_code(administrator.id)
            .expect("replacement invite should revoke"));
        let revoked = service
            .get_user_invite_code(administrator.id)
            .expect("invite lookup should succeed")
            .expect("revoked invite should remain visible");
        assert_eq!(revoked.id, replacement.id);
        assert_eq!(revoked.status, InviteCodeStatus::Revoked);
        assert!(revoked.revoked_at.is_some());
        assert!(!service
            .revoke_invite_code(administrator.id)
            .expect("repeated revocation should be a no-op"));
    }

    #[test]
    fn auth_service_bootstrap_requires_an_empty_database() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        service
            .bootstrap_admin("first_admin", STRONG_PASSWORD)
            .expect("first bootstrap should succeed");

        let error = service
            .bootstrap_admin("second_admin", STRONG_PASSWORD)
            .expect_err("second bootstrap should fail");

        assert!(matches!(error, AuthServiceError::Repository(_)));
        assert_eq!(
            count_users(&auth_db_path).expect("user count should load"),
            1
        );
    }

    #[test]
    fn access_token_service_validates_raw_name_before_reserved_name_and_ttl() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let user = service
            .bootstrap_admin("token_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");
        let overlong_reserved_name = format!("{}login", "😀".repeat(101));
        let surrounding_spaces = format!(" {} ", "a".repeat(99));

        let accepted_astral = service
            .create_access_token(user.id, &"😀".repeat(100), ACCESS_TOKEN_TTL_MIN_SECONDS)
            .expect("100 astral code points should be accepted");

        let overlong_error = service
            .create_access_token(user.id, &overlong_reserved_name, 0)
            .expect_err("raw overlength should win over reserved name and TTL");
        let spaces_error = service
            .create_access_token(user.id, &surrounding_spaces, ACCESS_TOKEN_TTL_MIN_SECONDS)
            .expect_err("surrounding spaces should count before trimming");
        let reserved_error = service
            .create_access_token(user.id, "  login\t", 0)
            .expect_err("reserved name should win over TTL after trimming");
        let unnamed = service
            .create_access_token(user.id, " \t ", ACCESS_TOKEN_TTL_MIN_SECONDS)
            .expect("whitespace-only names should retain unnamed-token compatibility");

        assert_eq!(accepted_astral.name, "😀".repeat(100));
        assert!(matches!(
            overlong_error,
            AuthServiceError::AccessTokenNameTooLong
        ));
        assert_eq!(overlong_error.to_string(), ACCESS_TOKEN_NAME_LENGTH_DETAIL);
        assert!(matches!(
            spaces_error,
            AuthServiceError::AccessTokenNameTooLong
        ));
        assert_eq!(spaces_error.to_string(), ACCESS_TOKEN_NAME_LENGTH_DETAIL);
        assert!(matches!(
            reserved_error,
            AuthServiceError::AccessTokenNameReserved
        ));
        assert_eq!(
            reserved_error.to_string(),
            ACCESS_TOKEN_RESERVED_NAME_DETAIL
        );
        assert_eq!(unnamed.name, "");
    }

    #[test]
    fn access_token_service_rejects_out_of_range_ttl() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let user = service
            .bootstrap_admin("token_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");

        for ttl in [
            ACCESS_TOKEN_TTL_MIN_SECONDS - 1,
            ACCESS_TOKEN_TTL_MAX_SECONDS + 1,
        ] {
            let error = service
                .create_access_token(user.id, "integration", ttl)
                .expect_err("out-of-range TTL should be rejected");

            assert!(matches!(error, AuthServiceError::AccessTokenTtlOutOfRange));
            assert_eq!(error.to_string(), ACCESS_TOKEN_TTL_DETAIL);
        }
        let minimum = service
            .create_access_token(user.id, "minimum", ACCESS_TOKEN_TTL_MIN_SECONDS)
            .expect("minimum TTL should be accepted");
        let maximum = service
            .create_access_token(user.id, "maximum", ACCESS_TOKEN_TTL_MAX_SECONDS)
            .expect("maximum TTL should be accepted");

        assert_eq!(minimum.name, "minimum");
        assert_eq!(maximum.name, "maximum");
    }

    #[test]
    fn access_token_login_replacement_serializes_concurrent_sessions() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        service
            .bootstrap_admin("token_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");
        let previous = service
            .login("token_admin", STRONG_PASSWORD)
            .expect("initial login should succeed");
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let service = service.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    service.login("token_admin", STRONG_PASSWORD)
                })
            })
            .collect::<Vec<_>>();
        let sessions = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("login thread should finish")
                    .expect("concurrent login should succeed")
            })
            .collect::<Vec<_>>();
        let valid_session_count = sessions
            .iter()
            .filter(|session| {
                service
                    .verify_access_token(&session.token)
                    .expect("returned session token should verify deterministically")
                    .is_some()
            })
            .count();

        assert_eq!(valid_session_count, 1);
        assert!(service
            .verify_access_token(&previous.token)
            .expect("previous session should resolve")
            .is_none());
    }

    #[test]
    fn login_issuance_rejects_credentials_observed_before_password_rotation() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let user = service
            .bootstrap_admin("stale_login_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");
        let stale_authorization = service
            .verify_user_authorization("stale_login_admin", STRONG_PASSWORD)
            .expect("credentials should verify")
            .expect("fixture credentials should be valid");

        assert!(service
            .reset_password(user.id, "replacement-password")
            .expect("password reset should commit"));
        let error = service
            .create_login_session(
                stale_authorization,
                SecurityAuditEvent::new("login", "completed"),
            )
            .expect_err("pre-rotation credentials must not mint a post-rotation session");

        assert!(matches!(error, AuthServiceError::InvalidCredentials));
        assert!(service
            .list_access_tokens(user.id)
            .expect("token list should load")
            .is_empty());
    }

    #[test]
    fn personal_token_issuance_rejects_authorization_observed_before_logout_all() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let user = service
            .bootstrap_admin("stale_token_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");
        let session = service
            .login("stale_token_admin", STRONG_PASSWORD)
            .expect("fixture login should succeed");
        let stale_authorization = service
            .verify_access_token_authorization(&session.token)
            .expect("session verification should run")
            .expect("fixture session should be valid");

        service
            .revoke_all_access_tokens_with_audit(
                user.id,
                SecurityAuditEvent::new("logout_all", "completed").with_actor_id(user.id.value()),
            )
            .expect("global revocation should commit");
        let error = service
            .create_access_token_with_authorization_and_audit(
                user.id,
                stale_authorization.token_generation,
                &session.token,
                "stale-successor",
                ACCESS_TOKEN_TTL_MIN_SECONDS,
                SecurityAuditEvent::new("token_create", "completed").with_actor_id(user.id.value()),
            )
            .expect_err("pre-revocation authorization must not mint a successor token");

        assert!(matches!(
            error,
            AuthServiceError::Repository(AuthRepositoryError::StaleAuthorization)
        ));
        assert!(service
            .list_access_tokens(user.id)
            .expect("token list should load")
            .is_empty());
    }

    #[test]
    fn credential_rotation_revokes_tokens_and_redacts_session_debug() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let user = service
            .bootstrap_admin("rotation_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");
        let session = service
            .login("rotation_admin", STRONG_PASSWORD)
            .expect("fixture login should succeed");
        let personal = service
            .create_access_token(user.id, "integration", ACCESS_TOKEN_TTL_MIN_SECONDS)
            .expect("fixture personal token should be created");
        let session_debug = format!("{session:?}");

        assert!(service
            .change_password(user.id, STRONG_PASSWORD, "replacement-password")
            .expect("password change should run"));
        assert!(service
            .verify_access_token(&session.token)
            .expect("old session should resolve")
            .is_none());
        assert!(service
            .verify_access_token(&personal.token)
            .expect("old personal token should resolve")
            .is_none());
        assert!(service
            .verify_user("rotation_admin", STRONG_PASSWORD)
            .expect("old password verification should run")
            .is_none());
        assert!(service
            .verify_user("rotation_admin", "replacement-password")
            .expect("new password verification should run")
            .is_some());
        assert!(session_debug.contains("[REDACTED]"));
        assert!(!session_debug.contains(&session.token));
        assert!(!session_debug.contains("rotation_admin"));
        assert!(!service
            .reset_password(UserId(i64::MAX), "unused-password")
            .expect("missing-user reset should run"));
    }

    #[test]
    fn administrator_reset_rejects_actor_demoted_after_authorization() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let authority = service
            .bootstrap_admin("reset_authority", STRONG_PASSWORD)
            .expect("authority administrator should bootstrap");
        let invite = service
            .create_invite_code(authority.id)
            .expect("fixture invite should create");
        let stale_actor = service
            .register(
                "reset_stale_actor",
                "stale-actor-password",
                Some(&invite.code),
            )
            .expect("stale actor should register");
        set_user_admin(&auth_db_path, authority.id, stale_actor.id, true)
            .expect("stale actor should be promoted");
        let audit = SecurityAuditEvent::new("user_password_reset", "completed")
            .with_actor_id(stale_actor.id.value())
            .with_target_id(authority.id.value());
        set_user_admin(&auth_db_path, authority.id, stale_actor.id, false)
            .expect("stale actor should be demoted");

        let error = service
            .reset_password_as_administrator_with_audit(
                stale_actor.id,
                authority.id,
                "unauthorized-replacement",
                audit,
            )
            .expect_err("demoted actor must not reset the authority password");

        assert!(matches!(
            error,
            AuthServiceError::Repository(AuthRepositoryError::AdministratorActorForbidden)
        ));
        assert!(service
            .verify_user("reset_authority", STRONG_PASSWORD)
            .expect("original password should verify")
            .is_some());
        assert!(service
            .verify_user("reset_authority", "unauthorized-replacement")
            .expect("replacement password should be rejected")
            .is_none());
    }

    #[test]
    fn concurrent_change_password_requests_report_only_one_success() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let service = AuthService::new(&auth_db_path);
        let user = service
            .bootstrap_admin("concurrent_rotation_admin", STRONG_PASSWORD)
            .expect("fixture administrator should bootstrap");
        let user_id = user.id;
        let barrier = Arc::new(Barrier::new(3));
        let handles = ["replacement-password-a", "replacement-password-b"]
            .into_iter()
            .map(|replacement_password| {
                let service = service.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    (
                        replacement_password,
                        service
                            .change_password(user_id, STRONG_PASSWORD, replacement_password)
                            .expect("concurrent password change should complete"),
                    )
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("concurrent password change thread should finish")
            })
            .collect::<Vec<_>>();
        let winner = results
            .iter()
            .find_map(|(password, did_update)| did_update.then_some(*password))
            .expect("one password change should win");

        assert_eq!(
            results.iter().filter(|(_, did_update)| *did_update).count(),
            1
        );
        assert!(service
            .verify_user("concurrent_rotation_admin", STRONG_PASSWORD)
            .expect("old password verification should run")
            .is_none());
        assert!(service
            .verify_user("concurrent_rotation_admin", winner)
            .expect("winning password verification should run")
            .is_some());
        for (password, did_update) in results {
            if !did_update {
                assert!(service
                    .verify_user("concurrent_rotation_admin", password)
                    .expect("losing password verification should run")
                    .is_none());
            }
        }
    }
}
