//! Authentication route handlers.

use axum::extract::{Path, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ps_auth::{
    is_valid_new_password, is_valid_username, AuthService, AuthServiceError, MIN_PASSWORD_LENGTH,
    SESSION_COOKIE_NAME,
};
use ps_domain::{
    ChangePasswordRequest, ErrorEnvelope, InviteCodeResponse, InviteRequiredResponse, LoginRequest,
    LoginResponse, LogoutResponse, OkResponse, RegisterRequest, TokenCreateRequest,
    TokenCreateResponse, TokenInfo, UserResponse,
};
use ps_storage::AuthRepositoryError;

use crate::response::ApiError;
use crate::state::{ApiState, AuthAttemptKind};

const MIN_TOKEN_TTL_SECONDS: i64 = 3600;
const MAX_TOKEN_TTL_SECONDS: i64 = 365 * 24 * 3600;
const AUTH_RATE_LIMIT_DETAIL: &str = "Too many authentication attempts; try again later";

/// Register a new user account.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `body` - Registration request.
///
/// # Returns
///
/// Created user response.
#[utoipa::path(
    post,
    path = "/api/auth/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 200, description = "Registered user.", body = UserResponse),
        (status = 429, description = "Authentication rate limit exceeded.", body = ErrorEnvelope)
    )
)]
pub(crate) async fn register(
    State(state): State<ApiState>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let username = body.username.trim().to_string();
    if !is_valid_username(&username) {
        return Err(ApiError::bad_request(
            "Username must be 3-32 alphanumeric or underscore characters",
        ));
    }
    if !is_valid_new_password(&body.password) {
        return Err(ApiError::bad_request(password_policy_message()));
    }
    check_auth_rate_limit(&state, AuthAttemptKind::Register, &username)?;
    let password = body.password;
    let invite_code = (!body.invite_code.is_empty()).then_some(body.invite_code);
    let auth_username = username.clone();
    let user = run_auth(&state, move |service| {
        service.register(&auth_username, &password, invite_code.as_deref())
    })
    .await?;
    state.clear_auth_attempts(&username);
    Ok(Json(user))
}

/// Authenticate a user and set a session cookie.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `body` - Login request.
///
/// # Returns
///
/// Login response without the raw token.
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login response.", body = LoginResponse),
        (status = 429, description = "Authentication rate limit exceeded.", body = ErrorEnvelope)
    )
)]
pub(crate) async fn login(
    State(state): State<ApiState>,
    Json(body): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let username = body.username.trim().to_string();
    check_auth_rate_limit(&state, AuthAttemptKind::Login, &username)?;
    let password = body.password;
    let auth_username = username.clone();
    let session = run_auth(&state, move |service| {
        service.login(&auth_username, &password)
    })
    .await?;
    state.clear_auth_attempts(&username);
    let payload = LoginResponse {
        user: session.user,
        expires_at: session.expires_at,
    };
    let mut response = Json(payload).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie_header(
            &session.token,
            session.expires_at,
            state.are_session_cookies_secure(),
        ))
        .map_err(|_| ApiError::internal_server_error())?,
    );
    Ok(response)
}

/// Return the current authenticated user.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Current user response.
#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "auth",
    responses((status = 200, description = "Current user.", body = UserResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_me(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<UserResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    Ok(Json(user))
}

/// Change the current user's password.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `body` - Password change request.
///
/// # Returns
///
/// OK response.
#[utoipa::path(
    post,
    path = "/api/auth/change-password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses((status = 200, description = "Password changed.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn change_password(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    if !is_valid_new_password(&body.new_password) {
        return Err(ApiError::bad_request(password_policy_message()));
    }
    let (user, _) = require_current_user(&state, &headers).await?;
    let old_password = body.old_password;
    let new_password = body.new_password;
    let did_change = run_auth(&state, move |service| {
        service.change_password(user.id, &old_password, &new_password)
    })
    .await?;
    if !did_change {
        return Err(ApiError::bad_request("Old password is incorrect"));
    }
    Ok(Json(OkResponse { ok: true }))
}

/// Logout the current session token and clear the browser cookie.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Logout response.
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "auth",
    responses((status = 200, description = "Logged out.", body = LogoutResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn logout(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (user, token) = require_current_user(&state, &headers).await?;
    let token_to_revoke = token.clone();
    run_auth(&state, move |service| {
        service.revoke_access_token_value(&token_to_revoke)
    })
    .await?;
    let mut response = Json(LogoutResponse {
        ok: true,
        user_id: user.id,
    })
    .into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_session_cookie_header(
            state.are_session_cookies_secure(),
        ))
        .map_err(|_| ApiError::internal_server_error())?,
    );
    Ok(response)
}

/// Create an access token for the current user.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `body` - Token creation request.
///
/// # Returns
///
/// Created token response.
#[utoipa::path(
    post,
    path = "/api/auth/tokens",
    tag = "auth",
    request_body = TokenCreateRequest,
    responses((status = 200, description = "Created access token.", body = TokenCreateResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<TokenCreateRequest>,
) -> Result<Json<TokenCreateResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let ttl = body.ttl.clamp(MIN_TOKEN_TTL_SECONDS, MAX_TOKEN_TTL_SECONDS);
    let name = body.name.trim().to_string();
    let token = run_auth(&state, move |service| {
        service.create_access_token(user.id, &name, ttl)
    })
    .await?;
    Ok(Json(token))
}

/// List active access tokens for the current user.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Active token metadata.
#[utoipa::path(
    get,
    path = "/api/auth/tokens",
    tag = "auth",
    responses((status = 200, description = "Active access tokens.", body = Vec<TokenInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_tokens(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<TokenInfo>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let tokens = run_auth(&state, move |service| service.list_access_tokens(user.id)).await?;
    Ok(Json(tokens))
}

/// Delete an access token by row id.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `token_id` - Token row identifier.
///
/// # Returns
///
/// OK response.
#[utoipa::path(
    delete,
    path = "/api/auth/tokens/{token_id}",
    tag = "auth",
    params(("token_id" = i64, Path, description = "Token row identifier.")),
    responses((status = 200, description = "Token deleted.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn delete_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(token_id): Path<i64>,
) -> Result<Json<OkResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let did_delete = run_auth(&state, move |service| {
        service.revoke_access_token(user.id, token_id)
    })
    .await?;
    if !did_delete {
        return Err(ApiError::not_found("Token not found"));
    }
    Ok(Json(OkResponse { ok: true }))
}

/// Generate a one-time invite code.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Invite code response.
#[utoipa::path(
    post,
    path = "/api/auth/invite-code",
    tag = "auth",
    responses((status = 200, description = "Generated invite code.", body = InviteCodeResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn generate_invite_code(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<InviteCodeResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let invite = run_auth(&state, move |service| service.create_invite_code(user.id)).await?;
    Ok(Json(invite))
}

/// Get the invite code generated by the current user.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Invite code response or null.
#[utoipa::path(
    get,
    path = "/api/auth/invite-code",
    tag = "auth",
    responses((status = 200, description = "Current user's invite code.", body = Option<InviteCodeResponse>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_invite_code(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Option<InviteCodeResponse>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let invite = run_auth(&state, move |service| service.get_user_invite_code(user.id)).await?;
    Ok(Json(invite))
}

/// Return whether registration requires an invite code.
///
/// # Arguments
///
/// * `state` - Shared API state.
///
/// # Returns
///
/// Invite requirement response.
#[utoipa::path(
    get,
    path = "/api/auth/invite-required",
    tag = "auth",
    responses((status = 200, description = "Invite requirement.", body = InviteRequiredResponse))
)]
pub(crate) async fn check_invite_required(
    State(state): State<ApiState>,
) -> Result<Json<InviteRequiredResponse>, ApiError> {
    let (required, bootstrap_required) = run_auth(&state, move |service| {
        Ok((
            service.is_invite_required()?,
            service.is_bootstrap_required()?,
        ))
    })
    .await?;
    Ok(Json(InviteRequiredResponse {
        required,
        bootstrap_required,
    }))
}

pub(crate) fn auth_service(state: &ApiState) -> AuthService {
    AuthService::new(state.storage_config().auth_db_path())
}

pub(crate) async fn require_current_user(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<(UserResponse, String), ApiError> {
    let token = resolve_auth_token(headers)?
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::unauthorized("Authentication required"))?;
    let token_to_verify = token.clone();
    let user = run_auth(state, move |service| {
        service.verify_access_token(&token_to_verify)
    })
    .await?
    .ok_or_else(|| ApiError::unauthorized("Invalid or expired token"))?;
    Ok((user, token))
}

/// Resolve and require an authenticated admin user.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Current admin user and raw token.
pub(crate) async fn require_admin_user(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<(UserResponse, String), ApiError> {
    let (user, token) = require_current_user(state, headers).await?;
    if !user.is_admin {
        return Err(ApiError::forbidden("Admin access required"));
    }
    Ok((user, token))
}

async fn run_auth<Output, Work>(state: &ApiState, work: Work) -> Result<Output, ApiError>
where
    Work: FnOnce(AuthService) -> Result<Output, AuthServiceError> + Send + 'static,
    Output: Send + 'static,
{
    let service = auth_service(state);
    state
        .run_blocking(move || work(service))
        .await?
        .map_err(map_auth_error)
}

fn resolve_auth_token(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    if let Some(authorization) = headers.get(AUTHORIZATION) {
        let value = authorization
            .to_str()
            .map_err(|_| ApiError::unauthorized("Invalid authorization format"))?;
        let mut parts = value.splitn(2, ' ');
        let scheme = parts.next().unwrap_or_default();
        let token = parts.next();
        if token.is_none() || !scheme.eq_ignore_ascii_case("bearer") {
            return Err(ApiError::unauthorized("Invalid authorization format"));
        }
        let token = token.unwrap_or_default().trim();
        if !token.is_empty() {
            return Ok(Some(token.to_string()));
        }
    }
    Ok(session_cookie(headers))
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .filter_map(|cookie| cookie.split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE_NAME).then_some(value.trim().to_string()))
}

fn session_cookie_header(token: &str, expires_at: f64, is_secure: bool) -> String {
    let max_age = (expires_at - current_unix_time()).max(0.0).floor() as i64;
    let mut value =
        format!("{SESSION_COOKIE_NAME}={token}; Max-Age={max_age}; Path=/; SameSite=lax; HttpOnly");
    if is_secure {
        value.push_str("; Secure");
    }
    value
}

fn clear_session_cookie_header(is_secure: bool) -> String {
    let mut value = format!("{SESSION_COOKIE_NAME}=; Max-Age=0; Path=/; SameSite=lax; HttpOnly");
    if is_secure {
        value.push_str("; Secure");
    }
    value
}

fn current_unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

fn check_auth_rate_limit(
    state: &ApiState,
    kind: AuthAttemptKind,
    username: &str,
) -> Result<(), ApiError> {
    state
        .check_auth_attempt(kind, username)
        .map_err(|retry_after| ApiError::too_many_requests(AUTH_RATE_LIMIT_DETAIL, retry_after))
}

fn password_policy_message() -> String {
    format!("Password must be at least {MIN_PASSWORD_LENGTH} characters")
}

pub(crate) fn map_auth_error(error: AuthServiceError) -> ApiError {
    match error {
        AuthServiceError::InvalidCredentials => {
            ApiError::unauthorized("Invalid username or password")
        }
        AuthServiceError::InvalidUsername | AuthServiceError::PasswordTooShort => {
            ApiError::bad_request(error.to_string())
        }
        AuthServiceError::Repository(AuthRepositoryError::InviteCodeRequired)
        | AuthServiceError::Repository(AuthRepositoryError::AdministratorBootstrapRequired)
        | AuthServiceError::Repository(AuthRepositoryError::InvalidOrUsedInviteCode)
        | AuthServiceError::Repository(AuthRepositoryError::UserHasAlreadyGeneratedInviteCode) => {
            ApiError::bad_request(error.to_string())
        }
        AuthServiceError::Repository(AuthRepositoryError::UsernameAlreadyExists) => {
            ApiError::conflict("Username already exists")
        }
        AuthServiceError::Repository(_) => ApiError::internal_server_error(),
    }
}
