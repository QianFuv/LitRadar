//! Tracking status and notification settings route handlers.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use ps_domain::{
    NotificationSettingsResponse, NotificationSettingsUpdate, TrackingFolderSummary,
    TrackingStatusResponse,
};

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

const ALLOWED_DELIVERY_METHODS: [&str; 2] = ["folder", "pushplus"];

/// Get tracking status for the authenticated user.
#[utoipa::path(
    get,
    path = "/api/tracking/status",
    tag = "tracking",
    responses((status = 200, description = "Tracking status.", body = TrackingStatusResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn status(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<TrackingStatusResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let folder = ps_storage::get_tracking_folder(state.storage_config().auth_db_path(), user.id)
        .map_err(|_| ApiError::internal_server_error())?;
    let folders = ps_storage::list_folders(state.storage_config().auth_db_path(), user.id)
        .map_err(|_| ApiError::internal_server_error())?;
    let settings =
        ps_storage::get_notification_settings(state.storage_config().auth_db_path(), user.id)
            .map_err(|_| ApiError::internal_server_error())?;
    let selected_databases = settings
        .as_ref()
        .map(|item| item.selected_databases.as_slice())
        .unwrap_or_default();
    let weekly_articles_available =
        ps_storage::count_weekly_articles(state.storage_config(), selected_databases)
            .map_err(|_| ApiError::internal_server_error())?;
    Ok(Json(TrackingStatusResponse {
        tracking_folder: folder.map(|item| TrackingFolderSummary {
            id: item.id,
            name: item.name,
        }),
        total_folders: folders.len(),
        weekly_articles_available,
        notification_configured: settings.is_some(),
    }))
}

/// Get the user's notification settings.
#[utoipa::path(
    get,
    path = "/api/tracking/notification-settings",
    tag = "tracking",
    responses((status = 200, description = "Notification settings.", body = Option<NotificationSettingsResponse>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_notification_settings(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Option<NotificationSettingsResponse>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let settings =
        ps_storage::get_notification_settings(state.storage_config().auth_db_path(), user.id)
            .map_err(|_| ApiError::internal_server_error())?;
    Ok(Json(settings))
}

/// Create or update the user's notification settings.
#[utoipa::path(
    put,
    path = "/api/tracking/notification-settings",
    tag = "tracking",
    request_body = NotificationSettingsUpdate,
    responses((status = 200, description = "Updated notification settings.", body = NotificationSettingsResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn update_notification_settings(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<NotificationSettingsUpdate>,
) -> Result<Json<NotificationSettingsResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let available_databases = ps_storage::list_available_database_names(state.storage_config())
        .map_err(|_| ApiError::internal_server_error())?;
    let mut selected_databases = ps_storage::normalize_database_names(&body.selected_databases);
    let invalid_databases = selected_databases
        .iter()
        .filter(|db_name| !available_databases.contains(db_name))
        .cloned()
        .collect::<Vec<_>>();
    if !invalid_databases.is_empty() {
        return Err(ApiError::bad_request(format!(
            "Unknown databases: {}",
            invalid_databases.join(", ")
        )));
    }
    if !selected_databases.is_empty()
        && selected_databases
            .iter()
            .all(|db_name| available_databases.contains(db_name))
        && selected_databases.len() == available_databases.len()
    {
        selected_databases.clear();
    }
    if !ALLOWED_DELIVERY_METHODS.contains(&body.delivery_method.as_str()) {
        return Err(ApiError::bad_request(format!(
            "delivery_method must be one of: {}",
            ALLOWED_DELIVERY_METHODS.join(", ")
        )));
    }
    if body.delivery_method == "pushplus" && body.pushplus_token.trim().is_empty() {
        return Err(ApiError::bad_request(
            "pushplus_token is required when delivery_method is 'pushplus'",
        ));
    }
    if body.delivery_method == "pushplus"
        && body.sync_to_tracking_folder
        && ps_storage::get_tracking_folder(state.storage_config().auth_db_path(), user.id)
            .map_err(|_| ApiError::internal_server_error())?
            .is_none()
    {
        return Err(ApiError::bad_request(
            "A tracking folder is required before enabling PushPlus sync to tracking",
        ));
    }
    let normalized = NotificationSettingsUpdate {
        keywords: trim_nonempty(body.keywords),
        directions: trim_nonempty(body.directions),
        selected_databases,
        delivery_method: body.delivery_method,
        pushplus_token: body.pushplus_token.trim().to_string(),
        pushplus_template: nonempty_or_default(body.pushplus_template, "markdown"),
        pushplus_topic: body.pushplus_topic.trim().to_string(),
        pushplus_channel: body.pushplus_channel.trim().to_string(),
        sync_to_tracking_folder: body.sync_to_tracking_folder,
        ai_base_url: body.ai_base_url.trim().to_string(),
        ai_api_key: body.ai_api_key.trim().to_string(),
        ai_model: body.ai_model.trim().to_string(),
        ai_system_prompt: body.ai_system_prompt.trim().to_string(),
        ai_backup_base_url: body.ai_backup_base_url.trim().to_string(),
        ai_backup_api_key: body.ai_backup_api_key.trim().to_string(),
        ai_backup_model: body.ai_backup_model.trim().to_string(),
        ai_backup_system_prompt: body.ai_backup_system_prompt.trim().to_string(),
        ai_retry_attempts: body.ai_retry_attempts,
        enabled: body.enabled,
    };
    let settings = ps_storage::upsert_notification_settings(
        state.storage_config().auth_db_path(),
        user.id,
        &normalized,
    )
    .map_err(|_| ApiError::internal_server_error())?;
    Ok(Json(settings))
}

fn trim_nonempty(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn nonempty_or_default(value: String, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}
