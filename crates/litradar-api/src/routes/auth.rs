//! Authentication route handlers.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Extension, Path, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use litradar_auth::{
    is_valid_new_password, is_valid_username, AuthService, AuthServiceError, MIN_PASSWORD_LENGTH,
    SESSION_COOKIE_NAME,
};
use litradar_domain::{
    ChangePasswordRequest, ErrorEnvelope, InviteCodeResponse, InviteRequiredResponse, LoginRequest,
    LoginResponse, LogoutResponse, OkResponse, RegisterRequest, SessionRevocationErrorDetail,
    SessionRevocationErrorResponse, TokenCreateRequest, TokenCreateResponse, TokenInfo,
    UserResponse,
};
use litradar_storage::{AuthRepositoryError, SecurityAuditEvent};
use tower_http::request_id::RequestId;

use crate::audit::{persist_security_audit_event, request_id_text};
use crate::response::ApiError;
use crate::state::{ApiState, AuthAttemptKind, AuthRateLimitRejection};

const AUTH_RATE_LIMIT_DETAIL: &str = "Too many authentication attempts; try again later";
const SESSION_REVOCATION_ERROR_CODE: &str = "session_revocation_unconfirmed";
const SESSION_REVOCATION_ERROR_MESSAGE: &str = "Session revocation could not be confirmed";
const SESSION_REVOCATION_RETRY_DELAY: Duration = Duration::from_millis(25);

struct AuthAudit {
    action: &'static str,
    actor_id: i64,
    target_id: i64,
    started_at: Instant,
    is_terminal: bool,
}

impl AuthAudit {
    fn new(action: &'static str) -> Self {
        Self {
            action,
            actor_id: 0,
            target_id: 0,
            started_at: Instant::now(),
            is_terminal: false,
        }
    }

    fn set_actor_id(&mut self, actor_id: i64) {
        self.actor_id = actor_id;
    }

    fn set_target_id(&mut self, target_id: i64) {
        self.target_id = target_id;
    }

    fn completed(&mut self) {
        tracing::info!(
            event = "security.auth.completed",
            component = "security",
            action = self.action,
            outcome = "completed",
            actor_id = self.actor_id,
            target_id = self.target_id,
            duration_ms = self.started_at.elapsed().as_millis() as u64,
        );
        self.is_terminal = true;
    }

    fn rejected(&mut self, reason: &'static str) {
        tracing::warn!(
            event = "security.auth.rejected",
            component = "security",
            action = self.action,
            outcome = "rejected",
            actor_id = self.actor_id,
            target_id = self.target_id,
            reason,
            duration_ms = self.started_at.elapsed().as_millis() as u64,
        );
        self.is_terminal = true;
    }

    fn rate_limited(&mut self, rejection: AuthRateLimitRejection, request_id: &str) {
        tracing::warn!(
            event = "security.auth.rate_limited",
            component = "security",
            action = self.action,
            outcome = "rate_limited",
            actor_id = self.actor_id,
            target_id = self.target_id,
            reason = rejection.reason,
            bucket = rejection.bucket,
            source_class = rejection.source_class,
            rejected_count = rejection.rejected_count,
            request_id,
            retry_after_seconds = rejection.retry_after_seconds,
            duration_ms = self.started_at.elapsed().as_millis() as u64,
        );
        self.is_terminal = true;
    }
}

impl Drop for AuthAudit {
    fn drop(&mut self) {
        if !self.is_terminal {
            tracing::warn!(
                event = "security.auth.rejected",
                component = "security",
                action = self.action,
                outcome = "rejected",
                actor_id = self.actor_id,
                target_id = self.target_id,
                reason = "operation_failed",
                duration_ms = self.started_at.elapsed().as_millis() as u64,
            );
        }
    }
}

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
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let mut audit = AuthAudit::new("register");
    let request_id = request_id_text(request_id.as_ref());
    let username = body.username.trim().to_string();
    if !is_valid_username(&username) {
        persist_auth_rejection(&state, &audit, "validation_failed", &request_id).await?;
        audit.rejected("validation_failed");
        return Err(ApiError::bad_request(
            "Username must be 3-32 alphanumeric or underscore characters",
        ));
    }
    if !is_valid_new_password(&body.password) {
        persist_auth_rejection(&state, &audit, "validation_failed", &request_id).await?;
        audit.rejected("validation_failed");
        return Err(ApiError::bad_request(password_policy_message()));
    }
    if let Err(rejection) = check_auth_rate_limit(
        &state,
        AuthAttemptKind::Register,
        &username,
        peer_address(peer),
        &headers,
    ) {
        let retry_after_seconds = rejection.retry_after_seconds;
        persist_auth_rate_limit(&state, &audit, rejection, &request_id).await?;
        audit.rate_limited(rejection, &request_id);
        return Err(ApiError::too_many_requests(
            AUTH_RATE_LIMIT_DETAIL,
            retry_after_seconds,
        ));
    }
    let password = body.password;
    let invite_code = (!body.invite_code.is_empty()).then_some(body.invite_code);
    let auth_username = username.clone();
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let user = match run_auth_kdf(&state, move |service| {
        service.register_with_audit(
            &auth_username,
            &password,
            invite_code.as_deref(),
            completion_audit,
        )
    })
    .await
    {
        Ok(user) => user,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "registration_failed", &request_id).await?;
            audit.rejected("registration_failed");
            return Err(error);
        }
    };
    state.clear_auth_attempts(AuthAttemptKind::Register, &username);
    audit.set_actor_id(user.id.0);
    audit.completed();
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
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let mut audit = AuthAudit::new("login");
    let request_id = request_id_text(request_id.as_ref());
    let username = body.username.trim().to_string();
    if let Err(rejection) = check_auth_rate_limit(
        &state,
        AuthAttemptKind::Login,
        &username,
        peer_address(peer),
        &headers,
    ) {
        let retry_after_seconds = rejection.retry_after_seconds;
        persist_auth_rate_limit(&state, &audit, rejection, &request_id).await?;
        audit.rate_limited(rejection, &request_id);
        return Err(ApiError::too_many_requests(
            AUTH_RATE_LIMIT_DETAIL,
            retry_after_seconds,
        ));
    }
    let password = body.password;
    let auth_username = username.clone();
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let session = match run_auth_kdf(&state, move |service| {
        service.login_with_audit(&auth_username, &password, completion_audit)
    })
    .await
    {
        Ok(session) => session,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "authentication_failed", &request_id).await?;
            audit.rejected("authentication_failed");
            return Err(error);
        }
    };
    state.clear_auth_attempts(AuthAttemptKind::Login, &username);
    audit.set_actor_id(session.user.id.0);
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
    audit.completed();
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
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    let mut audit = AuthAudit::new("password_change");
    let request_id = request_id_text(request_id.as_ref());
    if !is_valid_new_password(&body.new_password) {
        persist_auth_rejection(&state, &audit, "validation_failed", &request_id).await?;
        audit.rejected("validation_failed");
        return Err(ApiError::bad_request(password_policy_message()));
    }
    let (user, _) = match require_current_user(&state, &headers).await {
        Ok(current) => current,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "authentication_required", &request_id).await?;
            audit.rejected("authentication_required");
            return Err(error);
        }
    };
    audit.set_actor_id(user.id.0);
    let old_password = body.old_password;
    let new_password = body.new_password;
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let did_change = match run_auth_kdf(&state, move |service| {
        service.change_password_with_audit(user.id, &old_password, &new_password, completion_audit)
    })
    .await
    {
        Ok(did_change) => did_change,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "operation_failed", &request_id).await?;
            audit.rejected("operation_failed");
            return Err(error);
        }
    };
    if !did_change {
        persist_auth_rejection(&state, &audit, "authentication_failed", &request_id).await?;
        audit.rejected("authentication_failed");
        return Err(ApiError::bad_request("Old password is incorrect"));
    }
    audit.completed();
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
    responses(
        (status = 200, description = "Logged out.", body = LogoutResponse),
        (status = 503, description = "The browser cookie was cleared but durable token revocation was not confirmed.", body = SessionRevocationErrorResponse)
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn logout(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Response, ApiError> {
    let mut audit = AuthAudit::new("logout");
    let request_id = request_id_text(request_id.as_ref());
    let should_clear_cookie = session_cookie(&headers).is_some();
    let (user, token) = match require_current_user(&state, &headers).await {
        Ok(current) => current,
        Err(error) => {
            let audit_result =
                persist_auth_rejection(&state, &audit, "authentication_required", &request_id)
                    .await;
            audit.rejected("authentication_required");
            let response = match audit_result {
                Ok(()) => error.into_response(),
                Err(error) => error.into_response(),
            };
            return clear_browser_session_cookie(&state, response, should_clear_cookie);
        }
    };
    audit.set_actor_id(user.id.0);
    let token_to_revoke = token.clone();
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let revoke_result = run_auth_revocation(&state, move |service| {
        service.revoke_access_token_value_with_audit(&token_to_revoke, completion_audit.clone())
    })
    .await;
    if !matches!(revoke_result, Ok(Ok(_))) {
        return failed_revocation_response(&state, &mut audit, &request_id, should_clear_cookie)
            .await;
    }
    let response = Json(LogoutResponse {
        ok: true,
        user_id: user.id,
    })
    .into_response();
    audit.completed();
    clear_browser_session_cookie(&state, response, should_clear_cookie)
}

/// Revoke every session and personal access token for the current user.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `request_id` - Server-generated request identifier.
///
/// # Returns
///
/// Logout response after every token is durably revoked.
#[utoipa::path(
    post,
    path = "/api/auth/logout-all",
    tag = "auth",
    responses(
        (status = 200, description = "All sessions and personal access tokens were revoked.", body = LogoutResponse),
        (status = 503, description = "The browser cookie was cleared but durable token revocation was not confirmed.", body = SessionRevocationErrorResponse)
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn logout_all(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Response, ApiError> {
    let mut audit = AuthAudit::new("logout_all");
    let request_id = request_id_text(request_id.as_ref());
    let should_clear_cookie = session_cookie(&headers).is_some();
    let (user, _) = match require_current_user(&state, &headers).await {
        Ok(current) => current,
        Err(error) => {
            let audit_result =
                persist_auth_rejection(&state, &audit, "authentication_required", &request_id)
                    .await;
            audit.rejected("authentication_required");
            let response = match audit_result {
                Ok(()) => error.into_response(),
                Err(error) => error.into_response(),
            };
            return clear_browser_session_cookie(&state, response, should_clear_cookie);
        }
    };
    audit.set_actor_id(user.id.0);
    audit.set_target_id(user.id.0);
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let revoke_result = run_auth_revocation(&state, move |service| {
        service.revoke_all_access_tokens_with_audit(user.id, completion_audit.clone())
    })
    .await;
    if !matches!(revoke_result, Ok(Ok(_))) {
        return failed_revocation_response(&state, &mut audit, &request_id, should_clear_cookie)
            .await;
    }
    let response = Json(LogoutResponse {
        ok: true,
        user_id: user.id,
    })
    .into_response();
    audit.completed();
    clear_browser_session_cookie(&state, response, should_clear_cookie)
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
    responses(
        (status = 200, description = "Created access token.", body = TokenCreateResponse),
        (
            status = 400,
            description = "Validation order: authentication, raw name length, normalized reserved name, TTL, then quota. Errors: Access token name must be at most 100 Unicode code points; Access token name \"login\" is reserved; Access token TTL must be between 3600 and 31536000 seconds.",
            body = ErrorEnvelope
        ),
        (
            status = 409,
            description = "Validation order: authentication, raw name length, normalized reserved name, TTL, then quota. Error: Active access token limit of 50 reached; revoke a token before creating another.",
            body = ErrorEnvelope
        )
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<TokenCreateRequest>,
) -> Result<Json<TokenCreateResponse>, ApiError> {
    let mut audit = AuthAudit::new("token_create");
    let request_id = request_id_text(request_id.as_ref());
    let (user, _) = match require_current_user(&state, &headers).await {
        Ok(current) => current,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "authentication_required", &request_id).await?;
            audit.rejected("authentication_required");
            return Err(error);
        }
    };
    audit.set_actor_id(user.id.0);
    let name = body.name;
    let ttl = body.ttl;
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let token = match run_auth(&state, move |service| {
        service.create_access_token_with_audit(user.id, &name, ttl, completion_audit)
    })
    .await
    {
        Ok(token) => token,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "operation_failed", &request_id).await?;
            audit.rejected("operation_failed");
            return Err(error);
        }
    };
    audit.set_target_id(token.id);
    audit.completed();
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
    request_id: Option<Extension<RequestId>>,
    Path(token_id): Path<i64>,
) -> Result<Json<OkResponse>, ApiError> {
    let mut audit = AuthAudit::new("token_revoke");
    let request_id = request_id_text(request_id.as_ref());
    let (user, _) = match require_current_user(&state, &headers).await {
        Ok(current) => current,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "authentication_required", &request_id).await?;
            audit.rejected("authentication_required");
            return Err(error);
        }
    };
    audit.set_actor_id(user.id.0);
    audit.set_target_id(token_id);
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let did_delete = match run_auth(&state, move |service| {
        service.revoke_access_token_with_audit(user.id, token_id, completion_audit)
    })
    .await
    {
        Ok(did_delete) => did_delete,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "operation_failed", &request_id).await?;
            audit.rejected("operation_failed");
            return Err(error);
        }
    };
    if !did_delete {
        persist_auth_rejection(&state, &audit, "not_found", &request_id).await?;
        audit.rejected("not_found");
        return Err(ApiError::not_found("Token not found"));
    }
    audit.completed();
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
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<InviteCodeResponse>, ApiError> {
    let mut audit = AuthAudit::new("invite_create");
    let request_id = request_id_text(request_id.as_ref());
    let (user, _) = match require_current_user(&state, &headers).await {
        Ok(current) => current,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "authentication_required", &request_id).await?;
            audit.rejected("authentication_required");
            return Err(error);
        }
    };
    audit.set_actor_id(user.id.0);
    let completion_audit = auth_security_event(&audit, "completed", "", &request_id);
    let invite = match run_auth(&state, move |service| {
        service.create_invite_code_with_audit(user.id, completion_audit)
    })
    .await
    {
        Ok(invite) => invite,
        Err(error) => {
            persist_auth_rejection(&state, &audit, "operation_failed", &request_id).await?;
            audit.rejected("operation_failed");
            return Err(error);
        }
    };
    audit.set_target_id(invite.id);
    audit.completed();
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

async fn run_auth_kdf<Output, Work>(state: &ApiState, work: Work) -> Result<Output, ApiError>
where
    Work: FnOnce(AuthService) -> Result<Output, AuthServiceError> + Send + 'static,
    Output: Send + 'static,
{
    let service = auth_service(state);
    state
        .run_kdf_blocking(move || work(service))
        .await?
        .map_err(map_auth_error)
}

async fn run_auth_revocation<Output, Work>(
    state: &ApiState,
    mut work: Work,
) -> Result<Result<Output, AuthServiceError>, ApiError>
where
    Work: FnMut(&AuthService) -> Result<Output, AuthServiceError> + Send + 'static,
    Output: Send + 'static,
{
    let service = auth_service(state);
    state
        .run_blocking(move || retry_transient_revocation(|| work(&service)))
        .await
        .map_err(ApiError::from)
}

fn retry_transient_revocation<Output, Work>(mut work: Work) -> Result<Output, AuthServiceError>
where
    Work: FnMut() -> Result<Output, AuthServiceError>,
{
    match work() {
        Err(error) if error.is_transient_sqlite_contention() => {
            std::thread::sleep(SESSION_REVOCATION_RETRY_DELAY);
            work()
        }
        result => result,
    }
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

fn clear_browser_session_cookie(
    state: &ApiState,
    mut response: Response,
    should_clear_cookie: bool,
) -> Result<Response, ApiError> {
    if should_clear_cookie {
        response.headers_mut().append(
            SET_COOKIE,
            HeaderValue::from_str(&clear_session_cookie_header(
                state.are_session_cookies_secure(),
            ))
            .map_err(|_| ApiError::internal_server_error())?,
        );
    }
    Ok(response)
}

async fn failed_revocation_response(
    state: &ApiState,
    audit: &mut AuthAudit,
    request_id: &str,
    should_clear_cookie: bool,
) -> Result<Response, ApiError> {
    tracing::warn!(
        event = "security.auth.revocation_unconfirmed",
        component = "security",
        action = audit.action,
        actor_id = audit.actor_id,
        request_id,
    );
    if persist_auth_rejection(state, audit, "operation_failed", request_id)
        .await
        .is_err()
    {
        tracing::error!(
            event = "security.audit.persistence_failed",
            component = "security",
            action = audit.action,
            request_id,
        );
    }
    audit.rejected("operation_failed");
    let response = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(SessionRevocationErrorResponse {
            detail: SessionRevocationErrorDetail {
                code: SESSION_REVOCATION_ERROR_CODE.to_string(),
                message: SESSION_REVOCATION_ERROR_MESSAGE.to_string(),
                request_id: request_id.to_string(),
            },
        }),
    )
        .into_response();
    clear_browser_session_cookie(state, response, should_clear_cookie)
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
    peer_address: Option<SocketAddr>,
    headers: &HeaderMap,
) -> Result<(), AuthRateLimitRejection> {
    state.check_auth_attempt(kind, username, peer_address, headers)
}

fn peer_address(peer: Option<Extension<ConnectInfo<SocketAddr>>>) -> Option<SocketAddr> {
    peer.map(|Extension(ConnectInfo(address))| address)
}

fn auth_security_event(
    audit: &AuthAudit,
    outcome: &'static str,
    reason: &'static str,
    request_id: &str,
) -> SecurityAuditEvent {
    let mut event = SecurityAuditEvent::new(audit.action, outcome)
        .with_reason(reason)
        .with_request_id(request_id);
    if audit.actor_id > 0 {
        event = event.with_actor_id(audit.actor_id);
    }
    if audit.target_id > 0 {
        event = event.with_target_id(audit.target_id);
    }
    event
}

async fn persist_auth_rejection(
    state: &ApiState,
    audit: &AuthAudit,
    reason: &'static str,
    request_id: &str,
) -> Result<(), ApiError> {
    persist_security_audit_event(
        state,
        auth_security_event(audit, "rejected", reason, request_id),
    )
    .await
}

async fn persist_auth_rate_limit(
    state: &ApiState,
    audit: &AuthAudit,
    rejection: AuthRateLimitRejection,
    request_id: &str,
) -> Result<(), ApiError> {
    let event = auth_security_event(audit, "rate_limited", "", request_id).with_rate_limit(
        rejection.reason,
        rejection.bucket,
        rejection.source_class,
        rejection.rejected_count,
        rejection.retry_after_seconds,
    );
    persist_security_audit_event(state, event).await
}

fn password_policy_message() -> String {
    format!("Password must be at least {MIN_PASSWORD_LENGTH} characters")
}

pub(crate) fn map_auth_error(error: AuthServiceError) -> ApiError {
    match error {
        AuthServiceError::InvalidCredentials => {
            ApiError::unauthorized("Invalid username or password")
        }
        AuthServiceError::InvalidUsername
        | AuthServiceError::PasswordTooShort
        | AuthServiceError::AccessTokenNameTooLong
        | AuthServiceError::AccessTokenNameReserved
        | AuthServiceError::AccessTokenTtlOutOfRange => ApiError::bad_request(error.to_string()),
        AuthServiceError::Repository(AuthRepositoryError::InviteCodeRequired)
        | AuthServiceError::Repository(AuthRepositoryError::AdministratorBootstrapRequired)
        | AuthServiceError::Repository(AuthRepositoryError::InvalidOrUsedInviteCode)
        | AuthServiceError::Repository(AuthRepositoryError::UserHasAlreadyGeneratedInviteCode) => {
            ApiError::bad_request(error.to_string())
        }
        AuthServiceError::Repository(AuthRepositoryError::UsernameAlreadyExists) => {
            ApiError::conflict("Username already exists")
        }
        AuthServiceError::Repository(AuthRepositoryError::AccessTokenLimitReached) => {
            ApiError::conflict(error.to_string())
        }
        AuthServiceError::Repository(AuthRepositoryError::AuditPersistence(_)) => {
            ApiError::service_unavailable()
        }
        AuthServiceError::Password(_) => ApiError::internal_server_error(),
        AuthServiceError::Repository(_) => ApiError::internal_server_error(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use axum::http::{Method, StatusCode};
    use litradar_auth::{AuthService, AuthServiceError, ACCESS_TOKEN_DEFAULT_TTL};
    use litradar_storage::{list_security_audit_events, AuthRepositoryError};
    use serde_json::json;

    use crate::state::tracing_test_support::CapturedLogs;
    use crate::test_support::{json_request, TestBackend};

    use super::retry_transient_revocation;

    #[test]
    fn logout_retry_runs_once_only_for_transient_sqlite_contention() {
        let attempts = Cell::new(0_u8);
        let result = retry_transient_revocation(|| {
            let attempt = attempts.get() + 1;
            attempts.set(attempt);
            if attempt == 1 {
                return Err(AuthServiceError::Repository(AuthRepositoryError::Sqlite(
                    rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        None,
                    ),
                )));
            }
            Ok(true)
        });

        assert!(matches!(result, Ok(true)));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn logout_retry_does_not_repeat_non_transient_failures() {
        let attempts = Cell::new(0_u8);
        let result = retry_transient_revocation(|| -> Result<(), AuthServiceError> {
            attempts.set(attempts.get() + 1);
            Err(AuthServiceError::Repository(
                AuthRepositoryError::CredentialMutationInvariant,
            ))
        });

        assert!(matches!(
            result,
            Err(AuthServiceError::Repository(
                AuthRepositoryError::CredentialMutationInvariant
            ))
        ));
        assert_eq!(attempts.get(), 1);
    }

    #[tokio::test]
    async fn auth_audit_events_are_durable_and_exclude_credentials_or_names() {
        const USERNAME_SENTINEL: &str = "audit_user_sentinel";
        const PASSWORD_SENTINEL: &str = "credential-sentinel-never-log";
        const TOKEN_NAME_SENTINEL: &str = "token-name-sentinel-never-log";

        let backend = TestBackend::new();
        let service = AuthService::new(backend.auth_db_path());
        let user = service
            .bootstrap_admin(USERNAME_SENTINEL, PASSWORD_SENTINEL)
            .expect("audit user should bootstrap");
        let authorization_token = service
            .create_access_token(user.id, "test-authorization", ACCESS_TOKEN_DEFAULT_TTL)
            .expect("authorization token should be created")
            .token;
        let authorization = format!("Bearer {authorization_token}");
        let router = backend.router();

        let success_logs = CapturedLogs::default();
        let success = success_logs
            .capture_async(json_request(
                &router,
                Method::POST,
                "/api/auth/login",
                None,
                None,
                Some(json!({
                    "username": USERNAME_SENTINEL,
                    "password": PASSWORD_SENTINEL,
                })),
            ))
            .await;
        assert_eq!(success.status, StatusCode::OK);
        let success_event = success_logs
            .events()
            .into_iter()
            .find(|event| event["event"] == "security.auth.completed" && event["action"] == "login")
            .expect("login completion event should be captured");
        assert_eq!(success_event["actor_id"], user.id.0);
        assert!(success_event["spans"].as_array().is_some_and(|spans| {
            spans
                .iter()
                .any(|span| span["request_id"].as_str().is_some())
        }));
        let success_text = success_logs.text();
        assert!(!success_text.contains(USERNAME_SENTINEL));
        assert!(!success_text.contains(PASSWORD_SENTINEL));

        let failure_logs = CapturedLogs::default();
        let statuses = failure_logs
            .capture_async(async {
                let mut statuses = Vec::new();
                for _ in 0..6 {
                    let response = json_request(
                        &router,
                        Method::POST,
                        "/api/auth/login",
                        None,
                        None,
                        Some(json!({
                            "username": USERNAME_SENTINEL,
                            "password": "wrong-credential-sentinel-never-log",
                        })),
                    )
                    .await;
                    statuses.push(response.status);
                }
                statuses
            })
            .await;
        assert_eq!(statuses[..5], [StatusCode::UNAUTHORIZED; 5]);
        assert_eq!(statuses[5], StatusCode::TOO_MANY_REQUESTS);
        let failure_events = failure_logs.events();
        assert_eq!(
            failure_events
                .iter()
                .filter(|event| {
                    event["event"] == "security.auth.rejected"
                        && event["action"] == "login"
                        && event["reason"] == "authentication_failed"
                })
                .count(),
            5
        );
        assert_eq!(
            failure_events
                .iter()
                .filter(|event| {
                    event["event"] == "security.auth.rate_limited" && event["action"] == "login"
                })
                .count(),
            1
        );
        let failure_text = failure_logs.text();
        assert!(!failure_text.contains(USERNAME_SENTINEL));
        assert!(!failure_text.contains("wrong-credential-sentinel-never-log"));

        let token_logs = CapturedLogs::default();
        let token_response = token_logs
            .capture_async(json_request(
                &router,
                Method::POST,
                "/api/auth/tokens",
                Some(&authorization),
                None,
                Some(json!({
                    "name": TOKEN_NAME_SENTINEL,
                    "ttl": 3600,
                })),
            ))
            .await;
        assert_eq!(token_response.status, StatusCode::OK);
        let created_token = token_response.payload["token"]
            .as_str()
            .expect("created token should be returned");
        let token_event = token_logs
            .events()
            .into_iter()
            .find(|event| {
                event["event"] == "security.auth.completed" && event["action"] == "token_create"
            })
            .expect("token creation event should be captured");
        assert_eq!(token_event["actor_id"], user.id.0);
        assert_eq!(token_event["target_id"], token_response.payload["id"]);
        let token_text = token_logs.text();
        assert!(!token_text.contains(TOKEN_NAME_SENTINEL));
        assert!(!token_text.contains(&authorization_token));
        assert!(!token_text.contains(created_token));

        let read_logs = CapturedLogs::default();
        let me = read_logs
            .capture_async(json_request(
                &router,
                Method::GET,
                "/api/auth/me",
                Some(&authorization),
                None,
                None,
            ))
            .await;
        assert_eq!(me.status, StatusCode::OK);
        assert!(!read_logs.events().iter().any(|event| {
            event["event"]
                .as_str()
                .is_some_and(|name| name.starts_with("security.auth."))
        }));

        let durable = list_security_audit_events(backend.auth_db_path())
            .expect("durable authentication audit events should load");
        let login_completion = durable
            .iter()
            .find(|event| {
                event.action == "login"
                    && event.outcome == "completed"
                    && !event.request_id.is_empty()
            })
            .expect("completed login should persist with a request id");
        assert_eq!(login_completion.actor_id, Some(user.id.0));
        assert_eq!(
            durable
                .iter()
                .filter(|event| {
                    event.action == "login"
                        && event.outcome == "rejected"
                        && event.reason == "authentication_failed"
                })
                .count(),
            5
        );
        let rate_limit = durable
            .iter()
            .find(|event| event.action == "login" && event.outcome == "rate_limited")
            .expect("login limiter rejection should persist");
        assert!(!rate_limit.request_id.is_empty());
        assert!(!rate_limit.source_class.is_empty());
        assert!(!rate_limit.bucket.is_empty());
        assert!(rate_limit.rejected_count > 0);
        assert!(rate_limit.retry_after_seconds > 0);
        let token_completion = durable
            .iter()
            .find(|event| {
                event.action == "token_create"
                    && event.outcome == "completed"
                    && !event.request_id.is_empty()
            })
            .expect("request token creation should persist");
        assert_eq!(token_completion.actor_id, Some(user.id.0));
        assert_eq!(
            token_completion.target_id,
            token_response.payload["id"].as_i64()
        );
        let durable_text = format!("{durable:?}");
        assert!(!durable_text.contains(USERNAME_SENTINEL));
        assert!(!durable_text.contains(PASSWORD_SENTINEL));
        assert!(!durable_text.contains(TOKEN_NAME_SENTINEL));
        assert!(!durable_text.contains(&authorization_token));
        assert!(!durable_text.contains(created_token));
    }
}
