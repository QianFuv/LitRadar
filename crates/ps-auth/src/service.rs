//! Authentication service operations built on storage repositories.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::{InviteCodeResponse, TokenCreateResponse, TokenInfo, UserId, UserResponse};
use ps_storage::{
    count_users, create_invite_code, delete_access_token, delete_access_token_by_hash,
    delete_access_tokens_by_name, find_user_credentials_by_id, find_user_credentials_by_username,
    get_user_invite_code, initialize_auth_database, insert_access_token, list_access_tokens,
    random_hex, register_user_with_invite, update_user_password_and_delete_tokens,
    verify_access_token_hash, AuthRepositoryError, AuthUserRow, InviteCodeRow,
};

use crate::{hash_password, hash_token, verify_password};

/// Python-compatible default access token TTL in seconds.
pub const ACCESS_TOKEN_DEFAULT_TTL: i64 = 7 * 24 * 3600;

const ACCESS_TOKEN_BYTES: i64 = 32;
const PASSWORD_SALT_BYTES: i64 = 16;
const INVITE_CODE_BYTES: i64 = 8;
const LOGIN_TOKEN_NAME: &str = "login";

/// Authentication service error.
#[derive(Debug)]
pub enum AuthServiceError {
    /// Repository operation failed.
    Repository(AuthRepositoryError),
    /// Credentials did not match a stored user.
    InvalidCredentials,
}

impl fmt::Display for AuthServiceError {
    /// Format the service error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::InvalidCredentials => formatter.write_str("Invalid username or password"),
        }
    }
}

impl Error for AuthServiceError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::InvalidCredentials => None,
        }
    }
}

impl From<AuthRepositoryError> for AuthServiceError {
    /// Convert repository errors into service errors.
    fn from(error: AuthRepositoryError) -> Self {
        Self::Repository(error)
    }
}

/// Created login session with the raw token kept out of JSON responses.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginSession {
    /// Authenticated user.
    pub user: UserResponse,
    /// Raw token to set in the browser cookie.
    pub token: String,
    /// Token expiration timestamp.
    pub expires_at: f64,
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
        let salt = random_hex(&self.auth_db_path, PASSWORD_SALT_BYTES)?;
        let password_hash = hash_password(password, &salt);
        let user = register_user_with_invite(
            &self.auth_db_path,
            username,
            &password_hash,
            &salt,
            invite_code,
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
            return Ok(None);
        };
        if !verify_password(password, &row.salt, &row.password_hash) {
            return Ok(None);
        }
        Ok(Some(UserResponse {
            id: row.id,
            username: row.username,
            is_admin: row.is_admin,
        }))
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
        delete_access_tokens_by_name(&self.auth_db_path, user.id, LOGIN_TOKEN_NAME)?;
        let token =
            self.create_access_token(user.id, LOGIN_TOKEN_NAME, ACCESS_TOKEN_DEFAULT_TTL)?;
        Ok(LoginSession {
            user,
            token: token.token,
            expires_at: token.expires_at,
        })
    }

    /// Create a raw access token and store only its hash.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Owner user identifier.
    /// * `name` - Token display name.
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
        let token = random_hex(&self.auth_db_path, ACCESS_TOKEN_BYTES)?;
        let token_hash = hash_token(&token);
        let created_at = now_seconds();
        let expires_at = created_at + ttl as f64;
        let row = insert_access_token(
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
        let Some(row) = find_user_credentials_by_id(&self.auth_db_path, user_id)? else {
            return Ok(false);
        };
        if !verify_password(old_password, &row.salt, &row.password_hash) {
            return Ok(false);
        }
        let salt = random_hex(&self.auth_db_path, PASSWORD_SALT_BYTES)?;
        let password_hash = hash_password(new_password, &salt);
        update_user_password_and_delete_tokens(
            &self.auth_db_path,
            user_id,
            &password_hash,
            &salt,
            now_seconds(),
        )?;
        Ok(true)
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
        if find_user_credentials_by_id(&self.auth_db_path, user_id)?.is_none() {
            return Ok(false);
        }
        let salt = random_hex(&self.auth_db_path, PASSWORD_SALT_BYTES)?;
        let password_hash = hash_password(new_password, &salt);
        update_user_password_and_delete_tokens(
            &self.auth_db_path,
            user_id,
            &password_hash,
            &salt,
            now_seconds(),
        )?;
        Ok(true)
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
        let code = random_hex(&self.auth_db_path, INVITE_CODE_BYTES)?;
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
    /// True when one or more users already exist.
    pub fn is_invite_required(&self) -> Result<bool, AuthServiceError> {
        Ok(count_users(&self.auth_db_path)? > 0)
    }
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
