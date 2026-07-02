//! Authentication compatibility helpers.

pub mod password;
pub mod service;
pub mod session;
pub mod token;

use std::path::Path;

use ps_domain::UserId;
use serde::{Deserialize, Serialize};

pub use password::{hash_password, verify_password, PBKDF2_ITERATIONS};
pub use service::{AuthService, AuthServiceError, LoginSession, ACCESS_TOKEN_DEFAULT_TTL};
pub use session::{SessionCookiePolicy, AUTH_COOKIE_SECURE_ENV, SESSION_COOKIE_NAME};
pub use token::hash_token;

/// Authenticated user payload shared by auth services and handlers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    /// User identifier.
    pub id: UserId,
    /// Login username.
    pub username: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
}

/// Return the auth database path from storage configuration.
///
/// # Arguments
///
/// * `config` - Storage path configuration.
///
/// # Returns
///
/// Auth database path.
pub fn auth_database_path(config: &ps_storage::StorageConfig) -> &Path {
    config.auth_db_path()
}
