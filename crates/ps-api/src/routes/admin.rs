//! Admin route handlers for auth database business state.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use ps_auth::{is_valid_new_password, MIN_PASSWORD_LENGTH};
use ps_domain::{
    AdminInviteCodeInfo, AdminResetPassword, AdminSetAdmin, AdminStatsResponse, AdminUserInfo,
    AnnouncementCreate, AnnouncementInfo, AnnouncementUpdate, OkResponse, RuntimeSettingInfo,
    RuntimeSettingsUpdate, ScheduledTaskCreate, ScheduledTaskInfo, ScheduledTaskUpdate, UserId,
};
use ps_storage::BusinessRepositoryError;

use crate::response::ApiError;
use crate::routes::auth::{auth_service, map_auth_error, require_admin_user};
use crate::state::ApiState;

type AnnouncementPayload<'a> = (Option<&'a str>, Option<&'a str>, Option<String>);
type ScheduledTaskPayload<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a str>);

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
    require_admin_user(&state, &headers)?;
    let users = ps_storage::list_all_users(state.storage_config().auth_db_path())
        .map_err(map_business_error)?;
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
    let (admin, _) = require_admin_user(&state, &headers)?;
    let target_id = UserId(user_id);
    if target_id == admin.id && !body.is_admin {
        return Err(ApiError::bad_request("Cannot revoke own admin status"));
    }
    let did_update = ps_storage::set_user_admin(
        state.storage_config().auth_db_path(),
        target_id,
        body.is_admin,
    )
    .map_err(map_business_error)?;
    if !did_update {
        return Err(ApiError::not_found("User not found"));
    }
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
    require_admin_user(&state, &headers)?;
    if !is_valid_new_password(&body.new_password) {
        return Err(ApiError::bad_request(format!(
            "Password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    let did_reset = auth_service(&state)
        .reset_password(UserId(user_id), &body.new_password)
        .map_err(map_auth_error)?;
    if !did_reset {
        return Err(ApiError::not_found("User not found"));
    }
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
    let (admin, _) = require_admin_user(&state, &headers)?;
    let target_id = UserId(user_id);
    if target_id == admin.id {
        return Err(ApiError::bad_request("Cannot delete yourself"));
    }
    let did_delete = ps_storage::delete_user(state.storage_config().auth_db_path(), target_id)
        .map_err(map_business_error)?;
    if !did_delete {
        return Err(ApiError::not_found("User not found"));
    }
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
    require_admin_user(&state, &headers)?;
    let codes = ps_storage::list_all_invite_codes(state.storage_config().auth_db_path())
        .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let code = ps_storage::admin_create_invite_code(state.storage_config().auth_db_path())
        .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let did_delete = ps_storage::delete_invite_code(state.storage_config().auth_db_path(), code_id)
        .map_err(map_business_error)?;
    if !did_delete {
        return Err(ApiError::not_found("Code not found or already used"));
    }
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
    require_admin_user(&state, &headers)?;
    let stats = ps_storage::get_admin_stats(state.storage_config()).map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let tasks = ps_storage::list_scheduled_tasks(state.storage_config().auth_db_path())
        .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let (name, command, cron) =
        validate_scheduled_task_payload(Some(&body.name), Some(&body.command), Some(&body.cron))?;
    let task = ps_storage::create_scheduled_task(
        state.storage_config().auth_db_path(),
        name.unwrap_or_default(),
        command.unwrap_or_default(),
        cron.unwrap_or_default(),
        body.enabled,
    )
    .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let (name, command, cron) = validate_scheduled_task_payload(
        body.name.as_deref(),
        body.command.as_deref(),
        body.cron.as_deref(),
    )?;
    let task = ps_storage::update_scheduled_task(
        state.storage_config().auth_db_path(),
        task_id,
        name,
        command,
        cron,
        body.enabled,
    )
    .map_err(map_business_error)?;
    let Some(task) = task else {
        return Err(ApiError::not_found("Scheduled task not found"));
    };
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
    require_admin_user(&state, &headers)?;
    let did_delete =
        ps_storage::delete_scheduled_task(state.storage_config().auth_db_path(), task_id)
            .map_err(map_business_error)?;
    if !did_delete {
        return Err(ApiError::not_found("Scheduled task not found"));
    }
    Ok(Json(OkResponse { ok: true }))
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
    require_admin_user(&state, &headers)?;
    let settings = ps_storage::list_runtime_settings(state.storage_config().auth_db_path())
        .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let settings =
        ps_storage::upsert_runtime_settings(state.storage_config().auth_db_path(), &body.values)
            .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let announcements = ps_storage::list_all_announcements(state.storage_config().auth_db_path())
        .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let (title, message, priority) = validate_announcement_payload(
        Some(&body.title),
        Some(&body.message),
        Some(&body.priority),
    )?;
    let announcement = ps_storage::create_announcement(
        state.storage_config().auth_db_path(),
        title.unwrap_or_default(),
        message.unwrap_or_default(),
        priority.as_deref().unwrap_or("normal"),
        body.enabled,
    )
    .map_err(map_business_error)?;
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
    require_admin_user(&state, &headers)?;
    let (title, message, priority) = validate_announcement_payload(
        body.title.as_deref(),
        body.message.as_deref(),
        body.priority.as_deref(),
    )?;
    let announcement = ps_storage::update_announcement(
        state.storage_config().auth_db_path(),
        announcement_id,
        title,
        message,
        priority.as_deref(),
        body.enabled,
    )
    .map_err(map_business_error)?;
    let Some(announcement) = announcement else {
        return Err(ApiError::not_found("Announcement not found"));
    };
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
    require_admin_user(&state, &headers)?;
    let did_delete =
        ps_storage::delete_announcement(state.storage_config().auth_db_path(), announcement_id)
            .map_err(map_business_error)?;
    if !did_delete {
        return Err(ApiError::not_found("Announcement not found"));
    }
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
    command: Option<&'a str>,
    cron: Option<&'a str>,
) -> Result<ScheduledTaskPayload<'a>, ApiError> {
    let clean_name = name.map(str::trim);
    let clean_command = command.map(str::trim);
    let clean_cron = cron.map(str::trim);
    if clean_name == Some("") {
        return Err(ApiError::bad_request("Task name must not be empty"));
    }
    if clean_command == Some("") {
        return Err(ApiError::bad_request("Command must not be empty"));
    }
    if clean_cron == Some("") {
        return Err(ApiError::bad_request("Cron must not be empty"));
    }
    if let Some(cron) = clean_cron {
        validate_cron_expression(cron)?;
    }
    Ok((clean_name, clean_command, clean_cron))
}

fn validate_cron_expression(cron: &str) -> Result<(), ApiError> {
    let fields = cron.split_whitespace().count();
    if fields != 5 {
        return Err(ApiError::bad_request(format!(
            "Wrong number of fields; got {fields}, expected 5"
        )));
    }
    Ok(())
}

fn map_business_error(error: BusinessRepositoryError) -> ApiError {
    match error {
        BusinessRepositoryError::UnknownRuntimeSetting(_)
        | BusinessRepositoryError::InvalidRuntimeBoolean(_) => {
            ApiError::bad_request(error.to_string())
        }
        _ => ApiError::internal_server_error(),
    }
}
