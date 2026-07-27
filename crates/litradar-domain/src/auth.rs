//! Authentication request and response models.

use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::UserId;

/// Maximum active personal access tokens admitted for one user.
pub const ACCESS_TOKEN_ACTIVE_LIMIT: i64 = 50;

/// Maximum Unicode code points in an untrimmed access-token name.
pub const ACCESS_TOKEN_NAME_MAX_CODE_POINTS: usize = 100;

/// Reserved display name for the internal browser login token.
pub const ACCESS_TOKEN_RESERVED_NAME: &str = "login";

/// Minimum accepted personal access-token TTL in seconds.
pub const ACCESS_TOKEN_TTL_MIN_SECONDS: i64 = 3600;

/// Maximum accepted personal access-token TTL in seconds.
pub const ACCESS_TOKEN_TTL_MAX_SECONDS: i64 = 31_536_000;

/// Exact error detail for an overlength raw access-token name.
pub const ACCESS_TOKEN_NAME_LENGTH_DETAIL: &str =
    "Access token name must be at most 100 Unicode code points";

/// Exact error detail for the normalized reserved access-token name.
pub const ACCESS_TOKEN_RESERVED_NAME_DETAIL: &str = "Access token name \"login\" is reserved";

/// Exact error detail for an out-of-range access-token TTL.
pub const ACCESS_TOKEN_TTL_DETAIL: &str =
    "Access token TTL must be between 3600 and 31536000 seconds";

/// Exact error detail for exhausted personal access-token capacity.
pub const ACCESS_TOKEN_LIMIT_DETAIL: &str =
    "Active access token limit of 50 reached; revoke a token before creating another";

/// Published validation order for new personal access-token requests.
pub const ACCESS_TOKEN_VALIDATION_ORDER: &str =
    "authentication, raw name length, normalized reserved name, TTL, then quota";

/// User profile returned by auth endpoints.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserResponse {
    /// User identifier.
    pub id: UserId,
    /// Login username.
    pub username: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
}

impl fmt::Debug for UserResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserResponse")
            .field("id", &self.id)
            .field("username", &"[REDACTED]")
            .field("is_admin", &self.is_admin)
            .finish()
    }
}

/// Account registration request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RegisterRequest {
    /// Requested username.
    pub username: String,
    /// Requested password.
    pub password: String,
    /// Invite code text required for every public registration.
    pub invite_code: String,
}

impl fmt::Debug for RegisterRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisterRequest")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("invite_code", &"[REDACTED]")
            .finish()
    }
}

/// Login request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LoginRequest {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Login response that intentionally omits the raw session token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LoginResponse {
    /// Authenticated user.
    pub user: UserResponse,
    /// Session expiration timestamp.
    pub expires_at: f64,
}

/// Access token creation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TokenCreateRequest {
    /// Token display name.
    #[serde(default)]
    #[schema(max_length = 100)]
    pub name: String,
    /// Requested token TTL in seconds.
    #[serde(default = "default_token_ttl")]
    #[schema(minimum = 3600, maximum = 31536000)]
    pub ttl: i64,
}

/// Access token creation response.
#[derive(Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TokenCreateResponse {
    /// Token row identifier.
    pub id: i64,
    /// Raw token value returned only at creation time.
    pub token: String,
    /// Token display name.
    pub name: String,
    /// Token expiration timestamp.
    pub expires_at: f64,
}

impl fmt::Debug for TokenCreateResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenCreateResponse")
            .field("id", &self.id)
            .field("token", &"[REDACTED]")
            .field("name", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Access token metadata response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TokenInfo {
    /// Token row identifier.
    pub id: i64,
    /// Token display name.
    pub name: String,
    /// Token expiration timestamp.
    pub expires_at: f64,
    /// Token creation timestamp.
    pub created_at: f64,
}

/// Password change request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    /// Current password.
    pub old_password: String,
    /// Replacement password.
    pub new_password: String,
}

impl fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChangePasswordRequest")
            .field("old_password", &"[REDACTED]")
            .field("new_password", &"[REDACTED]")
            .finish()
    }
}

/// Invite code response.
#[derive(Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InviteCodeResponse {
    /// Invite code row identifier.
    pub id: i64,
    /// Raw invite code.
    pub code: String,
    /// Whether the invite code has been consumed.
    pub used: bool,
    /// Invite code creation timestamp.
    pub created_at: f64,
}

impl fmt::Debug for InviteCodeResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InviteCodeResponse")
            .field("id", &self.id)
            .field("code", &"[REDACTED]")
            .field("used", &self.used)
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Boolean ok response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct OkResponse {
    /// Whether the operation succeeded.
    pub ok: bool,
}

/// Logout response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LogoutResponse {
    /// Whether the operation succeeded.
    pub ok: bool,
    /// Authenticated user identifier.
    pub user_id: UserId,
}

/// Invite requirement response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct InviteRequiredResponse {
    /// Whether registration requires an invite code.
    pub required: bool,
    /// Whether a local administrator bootstrap must run before invites can be issued.
    pub bootstrap_required: bool,
}

/// Return the Python default access token TTL.
///
/// # Returns
///
/// Default token TTL in seconds.
pub fn default_token_ttl() -> i64 {
    7 * 24 * 3600
}

#[cfg(test)]
mod tests {
    use super::{
        default_token_ttl, ChangePasswordRequest, InviteCodeResponse, InviteRequiredResponse,
        LoginRequest, RegisterRequest, TokenCreateRequest, TokenCreateResponse, UserResponse,
    };

    #[test]
    fn token_create_request_keeps_python_default_ttl() {
        let request: TokenCreateRequest =
            serde_json::from_str(r#"{"name":"weekly"}"#).expect("request should deserialize");

        assert_eq!(request.name, "weekly");
        assert_eq!(request.ttl, default_token_ttl());
    }

    #[test]
    fn auth_invite_requirement_reports_bootstrap_state() {
        let response = InviteRequiredResponse {
            required: true,
            bootstrap_required: true,
        };

        assert_eq!(
            serde_json::to_value(response).expect("response should serialize"),
            serde_json::json!({"required": true, "bootstrap_required": true})
        );
    }

    #[test]
    fn auth_debug_output_redacts_credentials_and_raw_tokens() {
        let debug = format!(
            "{:?}",
            (
                UserResponse {
                    id: crate::UserId(1),
                    username: "user-name-sentinel".to_string(),
                    is_admin: true,
                },
                RegisterRequest {
                    username: "register-name-sentinel".to_string(),
                    password: "register-password-sentinel".to_string(),
                    invite_code: "register-invite-sentinel".to_string(),
                },
                LoginRequest {
                    username: "login-name-sentinel".to_string(),
                    password: "login-password-sentinel".to_string(),
                },
                TokenCreateResponse {
                    id: 1,
                    token: "access-token-sentinel".to_string(),
                    name: "token-name-sentinel".to_string(),
                    expires_at: 2.0,
                },
                ChangePasswordRequest {
                    old_password: "old-password-sentinel".to_string(),
                    new_password: "new-password-sentinel".to_string(),
                },
                InviteCodeResponse {
                    id: 2,
                    code: "invite-code-sentinel".to_string(),
                    used: false,
                    created_at: 3.0,
                },
            )
        );

        assert!(debug.contains("[REDACTED]"));
        for sentinel in [
            "user-name-sentinel",
            "register-name-sentinel",
            "register-password-sentinel",
            "register-invite-sentinel",
            "login-name-sentinel",
            "login-password-sentinel",
            "access-token-sentinel",
            "token-name-sentinel",
            "old-password-sentinel",
            "new-password-sentinel",
            "invite-code-sentinel",
        ] {
            assert!(!debug.contains(sentinel));
        }
    }
}
