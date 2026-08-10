//! Admin route handlers for auth database business state.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Extension, Path, State};
use axum::http::HeaderMap;
use axum::Json;
use litradar_auth::{is_valid_new_password, MIN_PASSWORD_LENGTH};
use litradar_domain::{
    validate_scheduled_task_timing, AdminInviteCodeCreate, AdminInviteCodeInfo, AdminResetPassword,
    AdminSetAdmin, AdminStatsResponse, AdminUserInfo, AnnouncementCreate, AnnouncementInfo,
    AnnouncementUpdate, OkResponse, ProviderCapabilityInfo, ProviderCatalogResponse,
    ProviderOrderConfiguration, RuntimeSettingInfo, RuntimeSettingsUpdate, ScheduledJobSpec,
    ScheduledTaskCreate, ScheduledTaskInfo, ScheduledTaskUpdate, SchedulerStatusResponse, UserId,
};
use litradar_storage::{BusinessRepositoryError, SecurityAuditEvent, StorageConfig};
use tower_http::request_id::RequestId;

use crate::audit::{persist_security_audit_event, request_id_text};
use crate::config::validate_runtime_settings_update;
use crate::response::ApiError;
use crate::routes::auth::{auth_service, map_auth_error, require_admin_user};
use crate::state::ApiState;

type AnnouncementPayload<'a> = (Option<&'a str>, Option<&'a str>, Option<String>);
type ScheduledTaskPayload<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a str>);

#[derive(Debug, Clone, Copy)]
enum ProviderConfigurationCapability {
    IndexContent,
    ArticleAbstract,
    ArticleFullText,
}

struct AdminAudit {
    action: &'static str,
    actor_id: i64,
    target_id: i64,
    started_at: Instant,
    is_terminal: bool,
}

impl AdminAudit {
    fn new(action: &'static str, actor_id: i64, target_id: i64) -> Self {
        Self {
            action,
            actor_id,
            target_id,
            started_at: Instant::now(),
            is_terminal: false,
        }
    }

    fn set_target_id(&mut self, target_id: i64) {
        self.target_id = target_id;
    }

    fn completed(&mut self) {
        tracing::info!(
            event = "security.admin.completed",
            component = "security",
            action = self.action,
            outcome = "completed",
            actor_id = self.actor_id,
            target_id = self.target_id,
            duration_ms = self.started_at.elapsed().as_millis() as u64,
        );
        self.is_terminal = true;
    }
}

impl Drop for AdminAudit {
    fn drop(&mut self) {
        if !self.is_terminal {
            tracing::warn!(
                event = "security.admin.rejected",
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

fn admin_security_event(
    action: &'static str,
    outcome: &'static str,
    actor_id: i64,
    target_id: i64,
    request_id: &str,
) -> SecurityAuditEvent {
    let event = SecurityAuditEvent::new(action, outcome)
        .with_actor_id(actor_id)
        .with_request_id(request_id);
    if target_id > 0 {
        event.with_target_id(target_id)
    } else {
        event
    }
}

async fn persist_admin_rejection(
    state: &ApiState,
    action: &'static str,
    actor_id: i64,
    target_id: i64,
    request_id: &str,
    reason: &'static str,
) -> Result<(), ApiError> {
    persist_security_audit_event(
        state,
        admin_security_event(action, "rejected", actor_id, target_id, request_id)
            .with_reason(reason),
    )
    .await
}

async fn run_audited_business<Output, Work>(
    state: &ApiState,
    action: &'static str,
    actor_id: i64,
    target_id: i64,
    request_id: String,
    work: Work,
) -> Result<Output, ApiError>
where
    Work: FnOnce(StorageConfig, SecurityAuditEvent) -> Result<Output, BusinessRepositoryError>
        + Send
        + 'static,
    Output: Send + 'static,
{
    let storage = state.storage_config().clone();
    let audit = admin_security_event(action, "completed", actor_id, target_id, &request_id);
    match state.run_blocking(move || work(storage, audit)).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error @ BusinessRepositoryError::AuditPersistence(_))) => {
            Err(map_business_error(error))
        }
        Ok(Err(error)) => {
            let reason = business_rejection_reason(&error);
            persist_admin_rejection(state, action, actor_id, target_id, &request_id, reason)
                .await?;
            Err(map_business_error(error))
        }
        Err(error) => {
            persist_admin_rejection(
                state,
                action,
                actor_id,
                target_id,
                &request_id,
                "executor_failed",
            )
            .await?;
            Err(error.into())
        }
    }
}

/// List all users with admin dashboard counts.
#[utoipa::path(
    get,
    path = "/api/admin/users",
    tag = "admin",
    responses((status = 200, description = "Admin user list.", body = Vec<AdminUserInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_users(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminUserInfo>>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let users = run_business(&state, move |storage| {
        litradar_storage::list_all_users(storage.auth_db_path())
    })
    .await?;
    Ok(Json(users))
}

/// Grant or revoke admin status.
#[utoipa::path(
    put,
    path = "/api/admin/users/{user_id}/admin",
    tag = "admin",
    params(("user_id" = i64, Path, description = "User row identifier.")),
    request_body = AdminSetAdmin,
    responses((status = 200, description = "Admin status updated.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn set_admin(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<AdminSetAdmin>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("user_admin_update", admin.id.0, user_id);
    let request_id = request_id_text(request_id.as_ref());
    let target_id = UserId(user_id);
    if target_id == admin.id && !body.is_admin {
        persist_admin_rejection(
            &state,
            "user_admin_update",
            admin.id.0,
            user_id,
            &request_id,
            "self_revocation_forbidden",
        )
        .await?;
        return Err(ApiError::bad_request("Cannot revoke own admin status"));
    }
    let actor_id = admin.id;
    let is_admin = body.is_admin;
    run_audited_business(
        &state,
        "user_admin_update",
        actor_id.value(),
        user_id,
        request_id,
        move |storage, event| {
            litradar_storage::set_user_admin_with_audit(
                storage.auth_db_path(),
                actor_id,
                target_id,
                is_admin,
                Some(&event),
            )
        },
    )
    .await?;
    audit.completed();
    Ok(Json(OkResponse { ok: true }))
}

/// Reset a user's password.
#[utoipa::path(
    post,
    path = "/api/admin/users/{user_id}/reset-password",
    tag = "admin",
    params(("user_id" = i64, Path, description = "User row identifier.")),
    request_body = AdminResetPassword,
    responses((status = 200, description = "Password reset.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn reset_password(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<AdminResetPassword>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("user_password_reset", admin.id.0, user_id);
    let request_id = request_id_text(request_id.as_ref());
    if !is_valid_new_password(&body.new_password) {
        persist_admin_rejection(
            &state,
            "user_password_reset",
            admin.id.0,
            user_id,
            &request_id,
            "password_policy_failed",
        )
        .await?;
        return Err(ApiError::bad_request(format!(
            "Password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    let service = auth_service(&state);
    let new_password = body.new_password;
    let completion = admin_security_event(
        "user_password_reset",
        "completed",
        admin.id.0,
        user_id,
        &request_id,
    );
    let reset_result = state
        .run_kdf_blocking(move || {
            service.reset_password_as_administrator_with_audit(
                actor_id,
                UserId(user_id),
                &new_password,
                completion,
            )
        })
        .await;
    let did_reset = match reset_result {
        Ok(Ok(did_reset)) => did_reset,
        Ok(Err(
            error @ litradar_auth::AuthServiceError::Repository(
                litradar_storage::AuthRepositoryError::AuditPersistence(_),
            ),
        )) => return Err(map_auth_error(error)),
        Ok(Err(error)) => {
            persist_admin_rejection(
                &state,
                "user_password_reset",
                admin.id.0,
                user_id,
                &request_id,
                "operation_failed",
            )
            .await?;
            return Err(map_auth_error(error));
        }
        Err(error) => {
            persist_admin_rejection(
                &state,
                "user_password_reset",
                admin.id.0,
                user_id,
                &request_id,
                "executor_failed",
            )
            .await?;
            return Err(error.into());
        }
    };
    if !did_reset {
        persist_admin_rejection(
            &state,
            "user_password_reset",
            admin.id.0,
            user_id,
            &request_id,
            "target_not_found",
        )
        .await?;
        return Err(ApiError::not_found("User not found"));
    }
    audit.completed();
    Ok(Json(OkResponse { ok: true }))
}

/// Delete a user and associated data.
#[utoipa::path(
    delete,
    path = "/api/admin/users/{user_id}",
    tag = "admin",
    params(("user_id" = i64, Path, description = "User row identifier.")),
    responses((status = 200, description = "User deleted.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn delete_user(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("user_delete", admin.id.0, user_id);
    let request_id = request_id_text(request_id.as_ref());
    let target_id = UserId(user_id);
    if target_id == admin.id {
        persist_admin_rejection(
            &state,
            "user_delete",
            admin.id.0,
            user_id,
            &request_id,
            "self_delete_forbidden",
        )
        .await?;
        return Err(ApiError::bad_request("Cannot delete yourself"));
    }
    let actor_id = admin.id;
    run_audited_business(
        &state,
        "user_delete",
        actor_id.value(),
        user_id,
        request_id,
        move |storage, event| {
            litradar_storage::delete_user_with_audit(
                storage.auth_db_path(),
                actor_id,
                target_id,
                Some(&event),
            )
        },
    )
    .await?;
    audit.completed();
    Ok(Json(OkResponse { ok: true }))
}

/// List invite codes.
#[utoipa::path(
    get,
    path = "/api/admin/invite-codes",
    tag = "admin",
    responses((status = 200, description = "Invite codes.", body = Vec<AdminInviteCodeInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_invite_codes(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminInviteCodeInfo>>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let codes = run_business(&state, move |storage| {
        litradar_storage::list_all_invite_codes(storage.auth_db_path())
    })
    .await?;
    Ok(Json(codes))
}

/// Create an admin-generated invite code.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `request_id` - Server-generated request identifier.
/// * `body` - Optional bounded lifecycle overrides.
///
/// # Returns
///
/// Created invite code metadata.
#[utoipa::path(
    post,
    path = "/api/admin/invite-codes",
    tag = "admin",
    request_body = Option<AdminInviteCodeCreate>,
    responses((status = 200, description = "Created invite code.", body = AdminInviteCodeInfo)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_invite_code(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    body: Option<Json<AdminInviteCodeCreate>>,
) -> Result<Json<AdminInviteCodeInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("invite_create", admin.id.0, 0);
    let request_id = request_id_text(request_id.as_ref());
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let code = run_audited_business(
        &state,
        "invite_create",
        admin.id.0,
        0,
        request_id,
        move |storage, event| {
            litradar_storage::admin_create_invite_code_with_policy_and_audit(
                storage.auth_db_path(),
                actor_id,
                body.expires_at,
                body.max_uses,
                Some(&event),
            )
        },
    )
    .await?;
    audit.set_target_id(code.id);
    audit.completed();
    Ok(Json(code))
}

/// Irreversibly revoke an invite code.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `code_id` - Invite code row identifier.
/// * `request_id` - Server-generated request identifier.
///
/// # Returns
///
/// OK response when an unrevoked invite was changed.
#[utoipa::path(
    delete,
    path = "/api/admin/invite-codes/{code_id}",
    tag = "admin",
    params(("code_id" = i64, Path, description = "Invite code row identifier.")),
    responses((status = 200, description = "Invite code revoked.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn revoke_admin_invite_code(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(code_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("invite_revoke", admin.id.0, code_id);
    let request_id = request_id_text(request_id.as_ref());
    let did_revoke = run_audited_business(
        &state,
        "invite_revoke",
        admin.id.0,
        code_id,
        request_id.clone(),
        move |storage, event| {
            litradar_storage::revoke_admin_invite_code_with_audit(
                storage.auth_db_path(),
                actor_id,
                code_id,
                Some(&event),
            )
        },
    )
    .await?;
    if !did_revoke {
        persist_admin_rejection(
            &state,
            "invite_revoke",
            admin.id.0,
            code_id,
            &request_id,
            "target_not_found",
        )
        .await?;
        return Err(ApiError::not_found("Code not found or already revoked"));
    }
    audit.completed();
    Ok(Json(OkResponse { ok: true }))
}

/// Return dashboard statistics.
#[utoipa::path(
    get,
    path = "/api/admin/stats",
    tag = "admin",
    responses((status = 200, description = "Admin dashboard statistics.", body = AdminStatsResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn stats(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<AdminStatsResponse>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let stats = run_business(&state, move |storage| {
        litradar_storage::get_admin_stats(&storage)
    })
    .await?;
    Ok(Json(stats))
}

/// List scheduled tasks.
#[utoipa::path(
    get,
    path = "/api/admin/scheduled-tasks",
    tag = "admin",
    responses((status = 200, description = "Scheduled tasks.", body = Vec<ScheduledTaskInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_scheduled_tasks(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ScheduledTaskInfo>>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let tasks = run_business(&state, move |storage| {
        litradar_storage::list_scheduled_tasks(storage.auth_db_path())
    })
    .await?;
    Ok(Json(tasks))
}

/// Create a scheduled task.
#[utoipa::path(
    post,
    path = "/api/admin/scheduled-tasks",
    tag = "admin",
    request_body = ScheduledTaskCreate,
    responses((status = 200, description = "Created scheduled task.", body = ScheduledTaskInfo)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_scheduled_task(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<ScheduledTaskCreate>,
) -> Result<Json<ScheduledTaskInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("scheduled_task_create", admin.id.0, 0);
    let request_id = request_id_text(request_id.as_ref());
    let validation = validate_scheduled_task_payload(
        Some(&body.name),
        Some(&body.cron),
        Some(&body.timezone),
        Some(body.timeout_seconds),
        Some(&body.job),
    );
    let (name, cron, timezone) = match validation {
        Ok(payload) => payload,
        Err(error) => {
            persist_admin_rejection(
                &state,
                "scheduled_task_create",
                admin.id.0,
                0,
                &request_id,
                "validation_failed",
            )
            .await?;
            return Err(error);
        }
    };
    let name = name.unwrap_or_default().to_string();
    let cron = cron.unwrap_or_default().to_string();
    let timezone = timezone.unwrap_or("UTC").to_string();
    let job = body.job;
    let timeout_seconds = body.timeout_seconds;
    let coalesce = body.coalesce;
    let enabled = body.enabled;
    let task = run_audited_business(
        &state,
        "scheduled_task_create",
        admin.id.0,
        0,
        request_id,
        move |storage, event| {
            litradar_storage::create_scheduled_task_with_audit(
                storage.auth_db_path(),
                actor_id,
                litradar_storage::ScheduledTaskCreateParams {
                    name: &name,
                    job: &job,
                    cron: &cron,
                    timezone: &timezone,
                    timeout_seconds,
                    coalesce,
                    enabled,
                },
                Some(&event),
            )
        },
    )
    .await?;
    audit.set_target_id(task.id);
    audit.completed();
    Ok(Json(task))
}

/// Update a scheduled task.
#[utoipa::path(
    put,
    path = "/api/admin/scheduled-tasks/{task_id}",
    tag = "admin",
    params(("task_id" = i64, Path, description = "Scheduled task row identifier.")),
    request_body = ScheduledTaskUpdate,
    responses((status = 200, description = "Updated scheduled task.", body = ScheduledTaskInfo)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn update_scheduled_task(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<ScheduledTaskUpdate>,
) -> Result<Json<ScheduledTaskInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("scheduled_task_update", admin.id.0, task_id);
    let request_id = request_id_text(request_id.as_ref());
    let validation = validate_scheduled_task_payload(
        body.name.as_deref(),
        body.cron.as_deref(),
        body.timezone.as_deref(),
        body.timeout_seconds,
        body.job.as_ref(),
    );
    let (name, cron, timezone) = match validation {
        Ok(payload) => payload,
        Err(error) => {
            persist_admin_rejection(
                &state,
                "scheduled_task_update",
                admin.id.0,
                task_id,
                &request_id,
                "validation_failed",
            )
            .await?;
            return Err(error);
        }
    };
    let name = name.map(str::to_string);
    let cron = cron.map(str::to_string);
    let timezone = timezone.map(str::to_string);
    let job = body.job;
    let timeout_seconds = body.timeout_seconds;
    let coalesce = body.coalesce;
    let enabled = body.enabled;
    let task = run_audited_business(
        &state,
        "scheduled_task_update",
        admin.id.0,
        task_id,
        request_id.clone(),
        move |storage, event| {
            litradar_storage::update_scheduled_task_with_audit(
                storage.auth_db_path(),
                actor_id,
                litradar_storage::ScheduledTaskUpdateParams {
                    task_id,
                    name: name.as_deref(),
                    job: job.as_ref(),
                    cron: cron.as_deref(),
                    timezone: timezone.as_deref(),
                    timeout_seconds,
                    coalesce,
                    enabled,
                },
                Some(&event),
            )
        },
    )
    .await?;
    let Some(task) = task else {
        persist_admin_rejection(
            &state,
            "scheduled_task_update",
            admin.id.0,
            task_id,
            &request_id,
            "target_not_found",
        )
        .await?;
        return Err(ApiError::not_found("Scheduled task not found"));
    };
    audit.completed();
    Ok(Json(task))
}

/// Delete a scheduled task.
#[utoipa::path(
    delete,
    path = "/api/admin/scheduled-tasks/{task_id}",
    tag = "admin",
    params(("task_id" = i64, Path, description = "Scheduled task row identifier.")),
    responses((status = 200, description = "Scheduled task deleted.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn delete_scheduled_task(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("scheduled_task_delete", admin.id.0, task_id);
    let request_id = request_id_text(request_id.as_ref());
    let did_delete = run_audited_business(
        &state,
        "scheduled_task_delete",
        admin.id.0,
        task_id,
        request_id.clone(),
        move |storage, event| {
            litradar_storage::delete_scheduled_task_with_audit(
                storage.auth_db_path(),
                actor_id,
                task_id,
                Some(&event),
            )
        },
    )
    .await?;
    if !did_delete {
        persist_admin_rejection(
            &state,
            "scheduled_task_delete",
            admin.id.0,
            task_id,
            &request_id,
            "target_not_found",
        )
        .await?;
        return Err(ApiError::not_found("Scheduled task not found"));
    }
    audit.completed();
    Ok(Json(OkResponse { ok: true }))
}

/// Read durable scheduler cursor, worker heartbeat, and run status.
#[utoipa::path(
    get,
    path = "/api/admin/scheduler/status",
    tag = "admin",
    responses((status = 200, description = "Durable scheduler status.", body = SchedulerStatusResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn scheduler_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<SchedulerStatusResponse>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let now = current_unix_time();
    let status = run_business(&state, move |storage| {
        litradar_storage::get_scheduler_status(
            storage.auth_db_path(),
            now,
            litradar_worker::scheduler::SCHEDULER_HEALTH_WINDOW_SECONDS,
            20,
        )
    })
    .await?;
    Ok(Json(status))
}

/// List managed runtime settings.
#[utoipa::path(
    get,
    path = "/api/admin/runtime-settings",
    tag = "admin",
    responses((status = 200, description = "Runtime settings.", body = Vec<RuntimeSettingInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_runtime_settings(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RuntimeSettingInfo>>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let secret_codec = state.secret_codec().clone();
    let settings = run_business(&state, move |storage| {
        litradar_storage::list_runtime_settings(storage.auth_db_path(), &secret_codec)
    })
    .await?;
    Ok(Json(settings))
}

/// Return built-in Provider capabilities and discovered catalog files.
#[utoipa::path(
    get,
    path = "/api/admin/provider-catalog",
    tag = "admin",
    responses((status = 200, description = "Provider capabilities and catalogs.", body = ProviderCatalogResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_provider_catalog(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<ProviderCatalogResponse>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let storage = state.storage_config().clone();
    let catalogs = state
        .run_blocking(move || storage.list_provider_catalogs())
        .await?
        .map_err(|_| ApiError::internal_server_error())?;
    Ok(Json(ProviderCatalogResponse {
        providers: litradar_sources::built_in_provider_capabilities(),
        catalogs,
    }))
}

/// Update managed runtime settings.
#[utoipa::path(
    put,
    path = "/api/admin/runtime-settings",
    tag = "admin",
    request_body = RuntimeSettingsUpdate,
    responses((status = 200, description = "Updated runtime settings.", body = Vec<RuntimeSettingInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn update_runtime_settings(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<RuntimeSettingsUpdate>,
) -> Result<Json<Vec<RuntimeSettingInfo>>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("runtime_settings_update", admin.id.0, 0);
    let request_id = request_id_text(request_id.as_ref());
    if let Err(error) = validate_runtime_settings_update(&body) {
        persist_admin_rejection(
            &state,
            "runtime_settings_update",
            admin.id.0,
            0,
            &request_id,
            "validation_failed",
        )
        .await?;
        return Err(ApiError::bad_request(error.to_string()));
    }
    if let Err(error) = validate_runtime_provider_settings_update(&body) {
        persist_admin_rejection(
            &state,
            "runtime_settings_update",
            admin.id.0,
            0,
            &request_id,
            "validation_failed",
        )
        .await?;
        return Err(error);
    }
    let values = body.values;
    let secret_pool_updates = body.secret_pool_updates;
    let secret_codec = state.secret_codec().clone();
    let settings = run_audited_business(
        &state,
        "runtime_settings_update",
        admin.id.0,
        0,
        request_id,
        move |storage, event| {
            litradar_storage::upsert_runtime_settings_with_audit(
                storage.auth_db_path(),
                actor_id,
                &secret_codec,
                &values,
                &secret_pool_updates,
                Some(&event),
            )
        },
    )
    .await?;
    audit.completed();
    Ok(Json(settings))
}

/// List all announcements for admin management.
#[utoipa::path(
    get,
    path = "/api/admin/announcements",
    tag = "admin",
    responses((status = 200, description = "All announcements.", body = Vec<AnnouncementInfo>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_announcements(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AnnouncementInfo>>, ApiError> {
    require_admin_user(&state, &headers).await?;
    let announcements = run_business(&state, move |storage| {
        litradar_storage::list_all_announcements(storage.auth_db_path())
    })
    .await?;
    Ok(Json(announcements))
}

/// Create an announcement.
#[utoipa::path(
    post,
    path = "/api/admin/announcements",
    tag = "admin",
    request_body = AnnouncementCreate,
    responses((status = 200, description = "Created announcement.", body = AnnouncementInfo)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_announcement(
    State(state): State<ApiState>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<AnnouncementCreate>,
) -> Result<Json<AnnouncementInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("announcement_create", admin.id.0, 0);
    let request_id = request_id_text(request_id.as_ref());
    let validation =
        validate_announcement_payload(Some(&body.title), Some(&body.message), Some(&body.priority));
    let (title, message, priority) = match validation {
        Ok(payload) => payload,
        Err(error) => {
            persist_admin_rejection(
                &state,
                "announcement_create",
                admin.id.0,
                0,
                &request_id,
                "validation_failed",
            )
            .await?;
            return Err(error);
        }
    };
    let title = title.unwrap_or_default().to_string();
    let message = message.unwrap_or_default().to_string();
    let priority = priority.unwrap_or_else(|| "normal".to_string());
    let enabled = body.enabled;
    let announcement = run_audited_business(
        &state,
        "announcement_create",
        admin.id.0,
        0,
        request_id,
        move |storage, event| {
            litradar_storage::create_announcement_with_audit(
                storage.auth_db_path(),
                actor_id,
                &title,
                &message,
                &priority,
                enabled,
                Some(&event),
            )
        },
    )
    .await?;
    audit.set_target_id(announcement.id);
    audit.completed();
    Ok(Json(announcement))
}

/// Update an announcement.
#[utoipa::path(
    put,
    path = "/api/admin/announcements/{announcement_id}",
    tag = "admin",
    params(("announcement_id" = i64, Path, description = "Announcement row identifier.")),
    request_body = AnnouncementUpdate,
    responses((status = 200, description = "Updated announcement.", body = AnnouncementInfo)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn update_announcement(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(announcement_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<AnnouncementUpdate>,
) -> Result<Json<AnnouncementInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("announcement_update", admin.id.0, announcement_id);
    let request_id = request_id_text(request_id.as_ref());
    let validation = validate_announcement_payload(
        body.title.as_deref(),
        body.message.as_deref(),
        body.priority.as_deref(),
    );
    let (title, message, priority) = match validation {
        Ok(payload) => payload,
        Err(error) => {
            persist_admin_rejection(
                &state,
                "announcement_update",
                admin.id.0,
                announcement_id,
                &request_id,
                "validation_failed",
            )
            .await?;
            return Err(error);
        }
    };
    let title = title.map(str::to_string);
    let message = message.map(str::to_string);
    let enabled = body.enabled;
    let announcement = run_audited_business(
        &state,
        "announcement_update",
        admin.id.0,
        announcement_id,
        request_id.clone(),
        move |storage, event| {
            litradar_storage::update_announcement_with_audit(
                storage.auth_db_path(),
                actor_id,
                litradar_storage::AnnouncementUpdateParams {
                    announcement_id,
                    title: title.as_deref(),
                    message: message.as_deref(),
                    priority: priority.as_deref(),
                    enabled,
                },
                Some(&event),
            )
        },
    )
    .await?;
    let Some(announcement) = announcement else {
        persist_admin_rejection(
            &state,
            "announcement_update",
            admin.id.0,
            announcement_id,
            &request_id,
            "target_not_found",
        )
        .await?;
        return Err(ApiError::not_found("Announcement not found"));
    };
    audit.completed();
    Ok(Json(announcement))
}

/// Delete an announcement.
#[utoipa::path(
    delete,
    path = "/api/admin/announcements/{announcement_id}",
    tag = "admin",
    params(("announcement_id" = i64, Path, description = "Announcement row identifier.")),
    responses((status = 200, description = "Announcement deleted.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn delete_announcement(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(announcement_id): Path<i64>,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let actor_id = admin.id;
    let mut audit = AdminAudit::new("announcement_delete", admin.id.0, announcement_id);
    let request_id = request_id_text(request_id.as_ref());
    let did_delete = run_audited_business(
        &state,
        "announcement_delete",
        admin.id.0,
        announcement_id,
        request_id.clone(),
        move |storage, event| {
            litradar_storage::delete_announcement_with_audit(
                storage.auth_db_path(),
                actor_id,
                announcement_id,
                Some(&event),
            )
        },
    )
    .await?;
    if !did_delete {
        persist_admin_rejection(
            &state,
            "announcement_delete",
            admin.id.0,
            announcement_id,
            &request_id,
            "target_not_found",
        )
        .await?;
        return Err(ApiError::not_found("Announcement not found"));
    }
    audit.completed();
    Ok(Json(OkResponse { ok: true }))
}

fn validate_announcement_payload<'a>(
    title: Option<&'a str>,
    message: Option<&'a str>,
    priority: Option<&'a str>,
) -> Result<AnnouncementPayload<'a>, ApiError> {
    let clean_title = title.map(str::trim);
    let clean_message = message.map(str::trim);
    let clean_priority = priority.map(|value| value.trim().to_ascii_lowercase());
    litradar_domain::validate_announcement_fields(
        clean_title,
        clean_message,
        clean_priority.as_deref(),
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok((clean_title, clean_message, clean_priority))
}

fn validate_scheduled_task_payload<'a>(
    name: Option<&'a str>,
    cron: Option<&'a str>,
    timezone: Option<&'a str>,
    timeout_seconds: Option<u64>,
    job: Option<&ScheduledJobSpec>,
) -> Result<ScheduledTaskPayload<'a>, ApiError> {
    let clean_name = name.map(str::trim);
    let clean_cron = cron.map(str::trim);
    let clean_timezone = timezone.map(str::trim);
    if clean_name == Some("") {
        return Err(ApiError::bad_request("Task name must not be empty"));
    }
    if clean_cron == Some("") {
        return Err(ApiError::bad_request("Cron must not be empty"));
    }
    if clean_timezone == Some("") {
        return Err(ApiError::bad_request("Timezone must not be empty"));
    }
    if let Some(cron) = clean_cron {
        litradar_worker::scheduler::validate_cron_expression(cron)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    }
    if let Some(job) = job {
        job.validate()
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    }
    if let (Some(timezone), Some(timeout_seconds)) = (clean_timezone, timeout_seconds) {
        validate_scheduled_task_timing(timezone, timeout_seconds)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    } else if let Some(timezone) = clean_timezone {
        validate_scheduled_task_timing(timezone, 3_600)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    } else if let Some(timeout_seconds) = timeout_seconds {
        validate_scheduled_task_timing("UTC", timeout_seconds)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    }
    Ok((clean_name, clean_cron, clean_timezone))
}

fn map_business_error(error: BusinessRepositoryError) -> ApiError {
    match error {
        BusinessRepositoryError::UnknownRuntimeSetting(_)
        | BusinessRepositoryError::InvalidRuntimeBoolean(_)
        | BusinessRepositoryError::InvalidRuntimeSetting(_)
        | BusinessRepositoryError::InvalidInput(_)
        | BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(_)
        | BusinessRepositoryError::InvalidScheduledJob(_)
        | BusinessRepositoryError::InvalidScheduledTask(_)
        | BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled
        | BusinessRepositoryError::InvalidInvitePolicy => ApiError::bad_request(error.to_string()),
        BusinessRepositoryError::AdministratorActorForbidden => {
            ApiError::forbidden(error.to_string())
        }
        BusinessRepositoryError::AdministratorTargetNotFound => {
            ApiError::not_found(error.to_string())
        }
        BusinessRepositoryError::AdministratorInvariantViolation => {
            ApiError::conflict(error.to_string())
        }
        BusinessRepositoryError::AuditPersistence(_) => ApiError::service_unavailable(),
        _ => ApiError::internal_server_error(),
    }
}

fn business_rejection_reason(error: &BusinessRepositoryError) -> &'static str {
    match error {
        BusinessRepositoryError::AdministratorActorForbidden => "actor_forbidden",
        BusinessRepositoryError::AdministratorTargetNotFound => "target_not_found",
        BusinessRepositoryError::AdministratorInvariantViolation => "administrator_invariant",
        BusinessRepositoryError::UnknownRuntimeSetting(_)
        | BusinessRepositoryError::InvalidRuntimeBoolean(_)
        | BusinessRepositoryError::InvalidRuntimeSetting(_)
        | BusinessRepositoryError::InvalidInput(_)
        | BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(_)
        | BusinessRepositoryError::InvalidScheduledJob(_)
        | BusinessRepositoryError::InvalidScheduledTask(_)
        | BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled
        | BusinessRepositoryError::InvalidInvitePolicy => "validation_failed",
        BusinessRepositoryError::OutboundEndpointNotAllowed => "endpoint_not_allowed",
        BusinessRepositoryError::AuditPersistence(_) => "audit_persistence_failed",
        _ => "operation_failed",
    }
}

fn validate_runtime_provider_settings_update(
    update: &RuntimeSettingsUpdate,
) -> Result<(), ApiError> {
    let providers = litradar_sources::built_in_provider_capabilities();
    for (field, value) in &update.values {
        let Some(value) = value else {
            continue;
        };
        match field.as_str() {
            "index_provider_routes" => {
                let routes = serde_json::from_str::<BTreeMap<String, String>>(value)
                    .map_err(|_| ApiError::bad_request("Invalid index Provider routes"))?;
                for provider in routes.values() {
                    validate_provider_capability(
                        &providers,
                        provider,
                        ProviderConfigurationCapability::IndexContent,
                    )?;
                }
            }
            "article_abstract_provider_orders" => {
                let configuration = parse_provider_order_configuration(value, field)?;
                validate_provider_order_capabilities(
                    &providers,
                    &configuration,
                    ProviderConfigurationCapability::ArticleAbstract,
                )?;
            }
            "article_fulltext_provider_orders" => {
                let configuration = parse_provider_order_configuration(value, field)?;
                validate_provider_order_capabilities(
                    &providers,
                    &configuration,
                    ProviderConfigurationCapability::ArticleFullText,
                )?;
            }
            "provider_proxy_policy" => {
                let policy = serde_json::from_str::<BTreeMap<String, bool>>(value)
                    .map_err(|_| ApiError::bad_request("Invalid Provider proxy policy"))?;
                for provider in policy.keys() {
                    validate_provider_exists(&providers, provider)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_provider_order_configuration(
    value: &str,
    field: &str,
) -> Result<ProviderOrderConfiguration, ApiError> {
    serde_json::from_str(value).map_err(|_| ApiError::bad_request(format!("Invalid {field}")))
}

fn validate_provider_order_capabilities(
    providers: &[ProviderCapabilityInfo],
    configuration: &ProviderOrderConfiguration,
    capability: ProviderConfigurationCapability,
) -> Result<(), ApiError> {
    validate_provider_order(providers, &configuration.default, capability)?;
    for order in configuration.catalogs.values() {
        validate_provider_order(providers, order, capability)?;
    }
    Ok(())
}

fn validate_provider_order(
    providers: &[ProviderCapabilityInfo],
    order: &[String],
    capability: ProviderConfigurationCapability,
) -> Result<(), ApiError> {
    let mut seen = BTreeSet::new();
    for provider in order {
        if !seen.insert(provider) {
            return Err(ApiError::bad_request(format!(
                "Duplicate Provider in order: {provider}"
            )));
        }
        validate_provider_capability(providers, provider, capability)?;
    }
    Ok(())
}

fn validate_provider_capability(
    providers: &[ProviderCapabilityInfo],
    provider_name: &str,
    capability: ProviderConfigurationCapability,
) -> Result<(), ApiError> {
    let provider = providers
        .iter()
        .find(|provider| provider.name == provider_name)
        .ok_or_else(|| ApiError::bad_request(format!("Unknown Provider: {provider_name}")))?;
    let is_supported = match capability {
        ProviderConfigurationCapability::IndexContent => provider.index_content,
        ProviderConfigurationCapability::ArticleAbstract => provider.article_abstract,
        ProviderConfigurationCapability::ArticleFullText => provider.article_full_text,
    };
    if !is_supported {
        return Err(ApiError::bad_request(format!(
            "Provider {provider_name} does not support the configured capability"
        )));
    }
    Ok(())
}

fn validate_provider_exists(
    providers: &[ProviderCapabilityInfo],
    provider_name: &str,
) -> Result<(), ApiError> {
    if providers
        .iter()
        .any(|provider| provider.name == provider_name)
    {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "Unknown Provider: {provider_name}"
        )))
    }
}

async fn run_business<Output, Work>(state: &ApiState, work: Work) -> Result<Output, ApiError>
where
    Work: FnOnce(StorageConfig) -> Result<Output, BusinessRepositoryError> + Send + 'static,
    Output: Send + 'static,
{
    let storage = state.storage_config().clone();
    state
        .run_blocking(move || work(storage))
        .await?
        .map_err(map_business_error)
}

fn current_unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use axum::response::IntoResponse;
    use litradar_storage::{
        list_security_audit_events, security_audit_persistence_failure_count,
        BusinessRepositoryError,
    };
    use rusqlite::Connection;
    use serde_json::json;

    use super::{map_business_error, validate_announcement_payload};
    use crate::state::tracing_test_support::CapturedLogs;
    use crate::test_support::{json_request, TestBackend};

    #[test]
    fn announcement_route_validation_uses_shared_unicode_limits() {
        validate_announcement_payload(
            Some(&"文".repeat(litradar_domain::MAX_ANNOUNCEMENT_TITLE_CHARS)),
            Some(&"文".repeat(litradar_domain::MAX_ANNOUNCEMENT_MESSAGE_CHARS)),
            Some("normal"),
        )
        .expect("announcement boundaries should pass");
        assert!(validate_announcement_payload(
            Some(&"文".repeat(litradar_domain::MAX_ANNOUNCEMENT_TITLE_CHARS + 1,)),
            Some("message"),
            Some("normal"),
        )
        .is_err());
    }

    #[tokio::test]
    async fn admin_audit_events_are_durable_and_exclude_request_content() {
        const TITLE_SENTINEL: &str = "announcement-title-sentinel-never-log";
        const MESSAGE_SENTINEL: &str = "announcement-message-sentinel-never-log";
        const REJECTED_SENTINEL: &str = "rejected-message-sentinel-never-log";

        let backend = TestBackend::new();
        let admin = backend.authenticated_user("audit_admin", true);
        let authorization = admin.authorization_header();
        let router = backend.router();

        let create_logs = CapturedLogs::default();
        let create_response = create_logs
            .capture_async(json_request(
                &router,
                Method::POST,
                "/api/admin/announcements",
                Some(&authorization),
                None,
                Some(json!({
                    "title": TITLE_SENTINEL,
                    "message": MESSAGE_SENTINEL,
                    "priority": "high",
                    "enabled": true,
                })),
            ))
            .await;
        assert_eq!(create_response.status, StatusCode::OK);
        let create_event = create_logs
            .events()
            .into_iter()
            .find(|event| {
                event["event"] == "security.admin.completed"
                    && event["action"] == "announcement_create"
            })
            .expect("announcement creation event should be captured");
        assert_eq!(create_event["actor_id"], admin.user_id().0);
        assert_eq!(create_event["target_id"], create_response.payload["id"]);
        assert!(create_event["spans"].as_array().is_some_and(|spans| {
            spans
                .iter()
                .any(|span| span["request_id"].as_str().is_some())
        }));
        let create_text = create_logs.text();
        assert!(!create_text.contains(TITLE_SENTINEL));
        assert!(!create_text.contains(MESSAGE_SENTINEL));
        assert!(!create_text.contains(&authorization));

        let rejected_logs = CapturedLogs::default();
        let rejected_response = rejected_logs
            .capture_async(json_request(
                &router,
                Method::PUT,
                "/api/admin/announcements/999999",
                Some(&authorization),
                None,
                Some(json!({
                    "message": REJECTED_SENTINEL,
                })),
            ))
            .await;
        assert_eq!(rejected_response.status, StatusCode::NOT_FOUND);
        let rejected_event = rejected_logs
            .events()
            .into_iter()
            .find(|event| {
                event["event"] == "security.admin.rejected"
                    && event["action"] == "announcement_update"
            })
            .expect("announcement rejection event should be captured");
        assert_eq!(rejected_event["actor_id"], admin.user_id().0);
        assert_eq!(rejected_event["target_id"], 999999);
        assert!(!rejected_logs.text().contains(REJECTED_SENTINEL));

        let durable = list_security_audit_events(backend.auth_db_path())
            .expect("durable administrator audit events should load");
        let create_record = durable
            .iter()
            .find(|event| {
                event.action == "announcement_create"
                    && event.outcome == "completed"
                    && !event.request_id.is_empty()
            })
            .expect("announcement creation should persist");
        assert_eq!(create_record.actor_id, Some(admin.user_id().0));
        assert_eq!(
            create_record.target_id,
            create_response.payload["id"].as_i64()
        );
        let rejected_record = durable
            .iter()
            .find(|event| {
                event.action == "announcement_update"
                    && event.outcome == "rejected"
                    && event.target_id == Some(999999)
            })
            .expect("missing announcement rejection should persist");
        assert_eq!(rejected_record.reason, "target_not_found");
        assert!(!rejected_record.request_id.is_empty());
        let durable_text = format!("{durable:?}");
        assert!(!durable_text.contains(TITLE_SENTINEL));
        assert!(!durable_text.contains(MESSAGE_SENTINEL));
        assert!(!durable_text.contains(REJECTED_SENTINEL));
        assert!(!durable_text.contains(&authorization));

        let read_logs = CapturedLogs::default();
        let list_response = read_logs
            .capture_async(json_request(
                &router,
                Method::GET,
                "/api/admin/announcements",
                Some(&authorization),
                None,
                None,
            ))
            .await;
        assert_eq!(list_response.status, StatusCode::OK);
        assert!(!read_logs.events().iter().any(|event| {
            event["event"]
                .as_str()
                .is_some_and(|name| name.starts_with("security.admin."))
        }));
    }

    #[tokio::test]
    async fn admin_audit_failure_returns_service_unavailable_and_rolls_back_mutation() {
        let backend = TestBackend::new();
        let admin = backend.authenticated_user("audit_failure_admin", true);
        let member = backend.authenticated_user("audit_failure_member", false);
        let completion_count_before = list_security_audit_events(backend.auth_db_path())
            .expect("fixture audit rows should load")
            .into_iter()
            .filter(|event| event.action == "user_admin_update" && event.outcome == "completed")
            .count();
        let failures_before = security_audit_persistence_failure_count();
        let connection =
            Connection::open(backend.auth_db_path()).expect("authentication database should open");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_api_security_audit \
                 BEFORE INSERT ON security_audit_events \
                 BEGIN SELECT RAISE(ABORT, 'injected API audit failure'); END;",
            )
            .expect("audit fault trigger should install");
        drop(connection);
        let router = backend.router();
        let authorization = admin.authorization_header();
        let logs = CapturedLogs::default();

        let response = logs
            .capture_async(json_request(
                &router,
                Method::PUT,
                &format!("/api/admin/users/{}/admin", member.user_id().value()),
                Some(&authorization),
                None,
                Some(json!({"is_admin": true})),
            ))
            .await;

        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        let stored_member = litradar_storage::list_all_users(backend.auth_db_path())
            .expect("users should remain readable")
            .into_iter()
            .find(|user| user.id == member.user_id())
            .expect("member should remain present");
        assert!(!stored_member.is_admin);
        let completion_count_after = list_security_audit_events(backend.auth_db_path())
            .expect("audit rows should remain readable")
            .into_iter()
            .filter(|event| event.action == "user_admin_update" && event.outcome == "completed")
            .count();
        assert_eq!(completion_count_after, completion_count_before);
        assert!(security_audit_persistence_failure_count() > failures_before);
        assert!(logs.text().contains("audit.persistence_failed"));
        assert!(!logs.text().contains("injected API audit failure"));
    }

    #[tokio::test]
    async fn admin_users_concurrent_cross_demotion_preserves_one_administrator() {
        let backend = TestBackend::new();
        let first = backend.authenticated_user("first_admin", true);
        let second = backend.authenticated_user("second_admin", true);
        let router = backend.router();
        let first_authorization = first.authorization_header();
        let second_authorization = second.authorization_header();
        let first_path = format!("/api/admin/users/{}/admin", second.user_id().value());
        let second_path = format!("/api/admin/users/{}/admin", first.user_id().value());
        let first_request = json_request(
            &router,
            Method::PUT,
            &first_path,
            Some(&first_authorization),
            None,
            Some(json!({"is_admin": false})),
        );
        let second_request = json_request(
            &router,
            Method::PUT,
            &second_path,
            Some(&second_authorization),
            None,
            Some(json!({"is_admin": false})),
        );
        let (first_response, second_response) = tokio::join!(first_request, second_request);
        let mut statuses = [first_response.status, second_response.status];
        statuses.sort();
        let administrator_count = litradar_storage::list_all_users(backend.auth_db_path())
            .expect("users should list")
            .into_iter()
            .filter(|user| user.is_admin)
            .count();

        assert_eq!(statuses, [StatusCode::OK, StatusCode::FORBIDDEN]);
        assert_eq!(administrator_count, 1);
    }

    #[tokio::test]
    async fn admin_users_stale_actor_and_missing_target_return_distinct_errors() {
        let backend = TestBackend::new();
        let first = backend.authenticated_user("first_admin", true);
        let second = backend.authenticated_user("second_admin", true);
        let member = backend.authenticated_user("member", false);
        litradar_storage::set_user_admin(
            backend.auth_db_path(),
            first.user_id(),
            second.user_id(),
            false,
        )
        .expect("first administrator should demote the second");
        let router = backend.router();
        let forbidden = json_request(
            &router,
            Method::PUT,
            &format!("/api/admin/users/{}/admin", member.user_id().value()),
            Some(&second.authorization_header()),
            None,
            Some(json!({"is_admin": true})),
        )
        .await;
        let missing = json_request(
            &router,
            Method::PUT,
            "/api/admin/users/999999/admin",
            Some(&first.authorization_header()),
            None,
            Some(json!({"is_admin": true})),
        )
        .await;

        assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
        assert_eq!(forbidden.payload["detail"], "Admin access required");
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        assert_eq!(missing.payload["detail"], "User not found");
    }

    #[test]
    fn admin_users_storage_error_mapping_preserves_distinct_statuses() {
        for (error, expected_status) in [
            (
                BusinessRepositoryError::AdministratorActorForbidden,
                StatusCode::FORBIDDEN,
            ),
            (
                BusinessRepositoryError::AdministratorTargetNotFound,
                StatusCode::NOT_FOUND,
            ),
            (
                BusinessRepositoryError::AdministratorInvariantViolation,
                StatusCode::CONFLICT,
            ),
        ] {
            assert_eq!(
                map_business_error(error).into_response().status(),
                expected_status
            );
        }
    }
}
