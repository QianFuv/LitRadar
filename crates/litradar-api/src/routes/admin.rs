//! Admin route handlers for auth database business state.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use litradar_auth::{is_valid_new_password, MIN_PASSWORD_LENGTH};
use litradar_domain::{
    validate_scheduled_task_timing, AdminInviteCodeInfo, AdminResetPassword, AdminSetAdmin,
    AdminStatsResponse, AdminUserInfo, AnnouncementCreate, AnnouncementInfo, AnnouncementUpdate,
    OkResponse, RuntimeSettingInfo, RuntimeSettingsUpdate, ScheduledJobSpec, ScheduledTaskCreate,
    ScheduledTaskInfo, ScheduledTaskUpdate, SchedulerStatusResponse, UserId,
};
use litradar_storage::{BusinessRepositoryError, StorageConfig};

use crate::config::validate_runtime_origin_settings_update;
use crate::response::ApiError;
use crate::routes::auth::{auth_service, map_auth_error, require_admin_user};
use crate::state::ApiState;

type AnnouncementPayload<'a> = (Option<&'a str>, Option<&'a str>, Option<String>);
type ScheduledTaskPayload<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a str>);

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
    Json(body): Json<AdminSetAdmin>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("user_admin_update", admin.id.0, user_id);
    let target_id = UserId(user_id);
    if target_id == admin.id && !body.is_admin {
        return Err(ApiError::bad_request("Cannot revoke own admin status"));
    }
    let is_admin = body.is_admin;
    let did_update = run_business(&state, move |storage| {
        litradar_storage::set_user_admin(storage.auth_db_path(), target_id, is_admin)
    })
    .await?;
    if !did_update {
        return Err(ApiError::not_found("User not found"));
    }
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
    Json(body): Json<AdminResetPassword>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("user_password_reset", admin.id.0, user_id);
    if !is_valid_new_password(&body.new_password) {
        return Err(ApiError::bad_request(format!(
            "Password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    let service = auth_service(&state);
    let new_password = body.new_password;
    let did_reset = state
        .run_blocking(move || service.reset_password(UserId(user_id), &new_password))
        .await?
        .map_err(map_auth_error)?;
    if !did_reset {
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
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("user_delete", admin.id.0, user_id);
    let target_id = UserId(user_id);
    if target_id == admin.id {
        return Err(ApiError::bad_request("Cannot delete yourself"));
    }
    let did_delete = run_business(&state, move |storage| {
        litradar_storage::delete_user(storage.auth_db_path(), target_id)
    })
    .await?;
    if !did_delete {
        return Err(ApiError::not_found("User not found"));
    }
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
#[utoipa::path(
    post,
    path = "/api/admin/invite-codes",
    tag = "admin",
    responses((status = 200, description = "Created invite code.", body = serde_json::Value)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_invite_code(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("invite_create", admin.id.0, 0);
    let code = run_business(&state, move |storage| {
        litradar_storage::admin_create_invite_code(storage.auth_db_path())
    })
    .await?;
    audit.set_target_id(code.id);
    audit.completed();
    Ok(Json(serde_json::json!({
        "id": code.id,
        "code": code.code,
        "created_at": code.created_at,
    })))
}

/// Delete an unused invite code.
#[utoipa::path(
    delete,
    path = "/api/admin/invite-codes/{code_id}",
    tag = "admin",
    params(("code_id" = i64, Path, description = "Invite code row identifier.")),
    responses((status = 200, description = "Invite code deleted.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn delete_invite_code(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(code_id): Path<i64>,
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("invite_delete", admin.id.0, code_id);
    let did_delete = run_business(&state, move |storage| {
        litradar_storage::delete_invite_code(storage.auth_db_path(), code_id)
    })
    .await?;
    if !did_delete {
        return Err(ApiError::not_found("Code not found or already used"));
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
    Json(body): Json<ScheduledTaskCreate>,
) -> Result<Json<ScheduledTaskInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("scheduled_task_create", admin.id.0, 0);
    let (name, cron, timezone) = validate_scheduled_task_payload(
        Some(&body.name),
        Some(&body.cron),
        Some(&body.timezone),
        Some(body.timeout_seconds),
        Some(&body.job),
    )?;
    let name = name.unwrap_or_default().to_string();
    let cron = cron.unwrap_or_default().to_string();
    let timezone = timezone.unwrap_or("UTC").to_string();
    let job = body.job;
    let timeout_seconds = body.timeout_seconds;
    let coalesce = body.coalesce;
    let enabled = body.enabled;
    let task = run_business(&state, move |storage| {
        litradar_storage::create_scheduled_task(
            storage.auth_db_path(),
            litradar_storage::ScheduledTaskCreateParams {
                name: &name,
                job: &job,
                cron: &cron,
                timezone: &timezone,
                timeout_seconds,
                coalesce,
                enabled,
            },
        )
    })
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
    Json(body): Json<ScheduledTaskUpdate>,
) -> Result<Json<ScheduledTaskInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("scheduled_task_update", admin.id.0, task_id);
    let (name, cron, timezone) = validate_scheduled_task_payload(
        body.name.as_deref(),
        body.cron.as_deref(),
        body.timezone.as_deref(),
        body.timeout_seconds,
        body.job.as_ref(),
    )?;
    let name = name.map(str::to_string);
    let cron = cron.map(str::to_string);
    let timezone = timezone.map(str::to_string);
    let job = body.job;
    let timeout_seconds = body.timeout_seconds;
    let coalesce = body.coalesce;
    let enabled = body.enabled;
    let task = run_business(&state, move |storage| {
        litradar_storage::update_scheduled_task(
            storage.auth_db_path(),
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
        )
    })
    .await?;
    let Some(task) = task else {
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
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("scheduled_task_delete", admin.id.0, task_id);
    let did_delete = run_business(&state, move |storage| {
        litradar_storage::delete_scheduled_task(storage.auth_db_path(), task_id)
    })
    .await?;
    if !did_delete {
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
    Json(body): Json<RuntimeSettingsUpdate>,
) -> Result<Json<Vec<RuntimeSettingInfo>>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("runtime_settings_update", admin.id.0, 0);
    validate_runtime_origin_settings_update(&body)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let values = body.values;
    let secret_pool_updates = body.secret_pool_updates;
    let secret_codec = state.secret_codec().clone();
    let settings = run_business(&state, move |storage| {
        litradar_storage::upsert_runtime_settings(
            storage.auth_db_path(),
            &secret_codec,
            &values,
            &secret_pool_updates,
        )
    })
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
    Json(body): Json<AnnouncementCreate>,
) -> Result<Json<AnnouncementInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("announcement_create", admin.id.0, 0);
    let (title, message, priority) = validate_announcement_payload(
        Some(&body.title),
        Some(&body.message),
        Some(&body.priority),
    )?;
    let title = title.unwrap_or_default().to_string();
    let message = message.unwrap_or_default().to_string();
    let priority = priority.unwrap_or_else(|| "normal".to_string());
    let enabled = body.enabled;
    let announcement = run_business(&state, move |storage| {
        litradar_storage::create_announcement(
            storage.auth_db_path(),
            &title,
            &message,
            &priority,
            enabled,
        )
    })
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
    Json(body): Json<AnnouncementUpdate>,
) -> Result<Json<AnnouncementInfo>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("announcement_update", admin.id.0, announcement_id);
    let (title, message, priority) = validate_announcement_payload(
        body.title.as_deref(),
        body.message.as_deref(),
        body.priority.as_deref(),
    )?;
    let title = title.map(str::to_string);
    let message = message.map(str::to_string);
    let enabled = body.enabled;
    let announcement = run_business(&state, move |storage| {
        litradar_storage::update_announcement(
            storage.auth_db_path(),
            announcement_id,
            title.as_deref(),
            message.as_deref(),
            priority.as_deref(),
            enabled,
        )
    })
    .await?;
    let Some(announcement) = announcement else {
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
) -> Result<Json<OkResponse>, ApiError> {
    let (admin, _) = require_admin_user(&state, &headers).await?;
    let mut audit = AdminAudit::new("announcement_delete", admin.id.0, announcement_id);
    let did_delete = run_business(&state, move |storage| {
        litradar_storage::delete_announcement(storage.auth_db_path(), announcement_id)
    })
    .await?;
    if !did_delete {
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
    if clean_title == Some("") {
        return Err(ApiError::bad_request("Title must not be empty"));
    }
    if clean_message == Some("") {
        return Err(ApiError::bad_request("Message must not be empty"));
    }
    if let Some(priority) = clean_priority.as_deref() {
        if !matches!(priority, "high" | "normal" | "low") {
            return Err(ApiError::bad_request(
                "Priority must be high, normal, or low",
            ));
        }
    }
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
        | BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(_)
        | BusinessRepositoryError::InvalidScheduledJob(_)
        | BusinessRepositoryError::InvalidScheduledTask(_)
        | BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled => {
            ApiError::bad_request(error.to_string())
        }
        _ => ApiError::internal_server_error(),
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
    use serde_json::json;

    use crate::state::tracing_test_support::CapturedLogs;
    use crate::test_support::{json_request, TestBackend};

    #[tokio::test]
    async fn admin_write_events_include_safe_ids_without_request_content() {
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
}
