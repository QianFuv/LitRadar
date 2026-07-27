//! Authentication request and response models.

use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::UserId;

/// Default invite-code lifetime in seconds.
pub const DEFAULT_INVITE_CODE_TTL_SECONDS: i64 = 7 * 24 * 3600;

/// Longest administrator-configurable invite-code lifetime in seconds.
pub const MAX_INVITE_CODE_TTL_SECONDS: i64 = 365 * 24 * 3600;

/// Default number of registrations permitted by a new invite code.
pub const DEFAULT_INVITE_CODE_MAX_USES: i64 = 1;

/// Largest administrator-configurable invite-code redemption quota.
pub const MAX_INVITE_CODE_USES: i64 = 1_000;

/// Return whether an invite-code lifecycle policy is within managed bounds.
///
/// # Arguments
///
/// * `now` - Current Unix timestamp.
/// * `expires_at` - Absolute expiration timestamp.
/// * `max_uses` - Maximum permitted registrations.
///
/// # Returns
///
/// True when timestamps are finite, expiry is in the next 365 days, and quota is `1..=1000`.
pub fn is_valid_invite_code_policy(now: f64, expires_at: f64, max_uses: i64) -> bool {
    now.is_finite()
        && expires_at.is_finite()
        && expires_at > now
        && expires_at - now <= MAX_INVITE_CODE_TTL_SECONDS as f64
        && (1..=MAX_INVITE_CODE_USES).contains(&max_uses)
}

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

/// Public lifecycle state for an invite code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InviteCodeStatus {
    /// The code can currently register another user.
    Active,
    /// The code passed its expiration timestamp.
    Expired,
    /// The code was explicitly or rotationally revoked.
    Revoked,
    /// The code consumed its complete redemption quota.
    Exhausted,
}

impl InviteCodeStatus {
    /// Derive a lifecycle state from persisted invite-code fields.
    ///
    /// # Arguments
    ///
    /// * `expires_at` - Absolute expiration timestamp.
    /// * `revoked_at` - Optional irreversible revocation timestamp.
    /// * `max_uses` - Maximum permitted redemption count.
    /// * `use_count` - Committed redemption count.
    /// * `now` - Current Unix timestamp.
    ///
    /// # Returns
    ///
    /// Stable public status with revocation and exhaustion taking precedence over expiry.
    pub fn from_lifecycle(
        expires_at: f64,
        revoked_at: Option<f64>,
        max_uses: i64,
        use_count: i64,
        now: f64,
    ) -> Self {
        if revoked_at.is_some() {
            Self::Revoked
        } else if use_count >= max_uses {
            Self::Exhausted
        } else if expires_at <= now {
            Self::Expired
        } else {
            Self::Active
        }
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
    /// Current lifecycle state.
    pub status: InviteCodeStatus,
    /// Absolute expiration timestamp.
    pub expires_at: f64,
    /// Optional irreversible revocation timestamp.
    pub revoked_at: Option<f64>,
    /// Maximum permitted registrations.
    pub max_uses: i64,
    /// Number of committed registrations.
    pub use_count: i64,
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
            .field("status", &self.status)
            .field("expires_at", &self.expires_at)
            .field("revoked_at", &self.revoked_at)
            .field("max_uses", &self.max_uses)
            .field("use_count", &self.use_count)
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

/// Stable detail returned when durable session revocation cannot be confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionRevocationErrorDetail {
    /// Stable client classification.
    pub code: String,
    /// Safe user-facing failure summary.
    pub message: String,
    /// Server-generated request identifier for audit correlation.
    pub request_id: String,
}

/// Error envelope for an unconfirmed session revocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionRevocationErrorResponse {
    /// Structured revocation failure detail.
    pub detail: SessionRevocationErrorDetail,
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
        default_token_ttl, is_valid_invite_code_policy, ChangePasswordRequest, InviteCodeResponse,
        InviteCodeStatus, InviteRequiredResponse, LoginRequest, RegisterRequest,
        TokenCreateRequest, TokenCreateResponse, UserResponse,
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
    fn invite_status_uses_revocation_quota_and_expiry_precedence() {
        assert_eq!(
            InviteCodeStatus::from_lifecycle(20.0, Some(15.0), 1, 1, 20.0),
            InviteCodeStatus::Revoked
        );
        assert_eq!(
            InviteCodeStatus::from_lifecycle(20.0, None, 1, 1, 20.0),
            InviteCodeStatus::Exhausted
        );
        assert_eq!(
            InviteCodeStatus::from_lifecycle(20.0, None, 2, 1, 20.0),
            InviteCodeStatus::Expired
        );
        assert_eq!(
            InviteCodeStatus::from_lifecycle(21.0, None, 2, 1, 20.0),
            InviteCodeStatus::Active
        );
    }

    #[test]
    fn invite_policy_requires_finite_future_expiry_and_bounded_quota() {
        assert!(is_valid_invite_code_policy(10.0, 20.0, 1));
        assert!(is_valid_invite_code_policy(
            10.0,
            10.0 + super::MAX_INVITE_CODE_TTL_SECONDS as f64,
            super::MAX_INVITE_CODE_USES
        ));
        assert!(!is_valid_invite_code_policy(10.0, 10.0, 1));
        assert!(!is_valid_invite_code_policy(10.0, f64::INFINITY, 1));
        assert!(!is_valid_invite_code_policy(f64::NAN, 20.0, 1));
        assert!(!is_valid_invite_code_policy(10.0, 20.0, 0));
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
                    status: InviteCodeStatus::Active,
                    expires_at: 4.0,
                    revoked_at: None,
                    max_uses: 1,
                    use_count: 0,
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
