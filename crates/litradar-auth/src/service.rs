//! Authentication service operations built on storage repositories.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_domain::{InviteCodeResponse, TokenCreateResponse, TokenInfo, UserId, UserResponse};
use litradar_storage::{
    bootstrap_admin, compare_and_swap_legacy_password_hash, count_users, create_invite_code,
    delete_access_token, delete_access_token_by_hash, find_user_credentials_by_id,
    find_user_credentials_by_username, get_user_invite_code, initialize_auth_database,
    insert_personal_access_token, list_access_tokens, random_hex, register_user_with_invite,
    replace_login_access_token, update_user_password_and_delete_tokens, verify_access_token_hash,
    AuthRepositoryError, AuthUserRow, InviteCodeRow, UserCredentialRow,
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
        validate_new_credentials(username, password)?;
        let password_hash = hash_password(password)?;
        let user = register_user_with_invite(
            &self.auth_db_path,
            username,
            &password_hash,
            "",
            invite_code,
            now_seconds(),
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
        validate_new_credentials(username, password)?;
        let password_hash = hash_password(password)?;
        let user = bootstrap_admin(
            &self.auth_db_path,
            username,
            &password_hash,
            "",
            now_seconds(),
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
        let Some(row) = find_user_credentials_by_username(&self.auth_db_path, username)? else {
            verify_dummy_password(password);
            return Ok(None);
        };
        match verify_password(password, &row.salt, &row.password_hash) {
            PasswordVerification::Invalid => return Ok(None),
            PasswordVerification::ValidCurrent => {}
            PasswordVerification::ValidLegacy => {
                if !self.upgrade_legacy_password(&row, password)? {
                    return Ok(None);
                }
            }
        }
        Ok(Some(UserResponse {
            id: row.id,
            username: row.username,
            is_admin: row.is_admin,
        }))
    }

    fn upgrade_legacy_password(
        &self,
        legacy_row: &UserCredentialRow,
        password: &str,
    ) -> Result<bool, AuthServiceError> {
        let replacement_hash = hash_password(password)?;
        if compare_and_swap_legacy_password_hash(
            &self.auth_db_path,
            legacy_row.id,
            &legacy_row.password_hash,
            &legacy_row.salt,
            &replacement_hash,
            now_seconds(),
        )? {
            return Ok(true);
        }
        let Some(current) = find_user_credentials_by_id(&self.auth_db_path, legacy_row.id)? else {
            return Ok(false);
        };
        Ok(
            verify_password(password, &current.salt, &current.password_hash)
                != PasswordVerification::Invalid,
        )
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
        let user = self
            .verify_user(username, password)?
            .ok_or(AuthServiceError::InvalidCredentials)?;
        let token = random_hex(ACCESS_TOKEN_BYTES)?;
        let token_hash = hash_token(&token);
        let created_at = now_seconds();
        let expires_at = created_at + ACCESS_TOKEN_DEFAULT_TTL as f64;
        let row = replace_login_access_token(
            &self.auth_db_path,
            user.id,
            &token_hash,
            expires_at,
            created_at,
        )?;
        Ok(LoginSession {
            user,
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
        let token = random_hex(ACCESS_TOKEN_BYTES)?;
        let token_hash = hash_token(&token);
        let created_at = now_seconds();
        let expires_at = created_at + ttl as f64;
        let row = insert_personal_access_token(
            &self.auth_db_path,
            user_id,
            &token_hash,
            name,
            expires_at,
            created_at,
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
        let token_hash = hash_token(token);
        let user = verify_access_token_hash(&self.auth_db_path, &token_hash, now_seconds())?;
        Ok(user.map(user_response))
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
        Ok(delete_access_token(&self.auth_db_path, user_id, token_id)?)
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
        let token_hash = hash_token(token);
        Ok(delete_access_token_by_hash(
            &self.auth_db_path,
            &token_hash,
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
        let did_update = update_user_password_and_delete_tokens(
            &self.auth_db_path,
            user_id,
            &password_hash,
            "",
            now_seconds(),
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
        validate_new_password(new_password)?;
        let password_hash = hash_password(new_password)?;
        Ok(update_user_password_and_delete_tokens(
            &self.auth_db_path,
            user_id,
            &password_hash,
            "",
            now_seconds(),
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
        let code = random_hex(INVITE_CODE_BYTES)?;
        let row = create_invite_code(&self.auth_db_path, user_id, &code, now_seconds())?;
        Ok(invite_response(row))
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
        Ok(get_user_invite_code(&self.auth_db_path, user_id)?.map(invite_response))
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

fn user_response(row: AuthUserRow) -> UserResponse {
    UserResponse {
        id: row.id,
        username: row.username,
        is_admin: row.is_admin,
    }
}

fn invite_response(row: InviteCodeRow) -> InviteCodeResponse {
    InviteCodeResponse {
        id: row.id,
        code: row.code,
        used: row.used_by.is_some(),
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

    use litradar_domain::UserId;
    use litradar_storage::{
        bootstrap_admin, count_users, find_user_credentials_by_id, migrate_auth_database,
        UserCredentialRow,
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
        assert!(!service
            .upgrade_legacy_password(&stale_credentials, STRONG_PASSWORD)
            .expect("stale legacy upgrade should recheck current credentials"));
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
}
