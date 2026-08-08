//! Tracking status and notification settings route handlers.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use litradar_domain::{
    ErrorEnvelope, ManualPushState, ManualWeeklyPushStatus, NotificationSettingsResponse,
    NotificationSettingsUpdate, TrackingFolderSummary, TrackingStatusResponse,
    NOTIFICATION_AI_RETRY_ATTEMPTS_MAX, NOTIFICATION_AI_RETRY_ATTEMPTS_MIN,
};
use litradar_storage::{
    DeliveryRepositoryError, DeliveryRunAdmissionOutcome, DeliveryRunCreate, DeliveryRunMode,
    DeliveryRunRecord, DeliveryRunStatus, DeliveryTriggerKind, DeliveryWorkflow,
    ManualDeliveryRunAdmissionOutcome, StorageConfig,
};
use litradar_worker::delivery::{ManualWeeklyPushOutcome, MANUAL_DELIVERY_JOB_DEADLINE_SECONDS};

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

const ALLOWED_DELIVERY_METHODS: [&str; 2] = ["folder", "pushplus"];
const MANUAL_PUSH_IDLE_MESSAGE: &str = "No manual push task is available";

/// Start one manual weekly-push job for the authenticated user.
#[utoipa::path(
    post,
    path = "/api/tracking/push-weekly",
    tag = "tracking",
    responses(
        (status = 202, description = "Queued or existing active manual weekly push.", body = ManualWeeklyPushStatus),
        (status = 409, description = "The latest manual push has an ambiguous outcome.", body = ErrorEnvelope)
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn push_weekly_to_tracking(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<ManualWeeklyPushStatus>), ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let job_id = run_storage(&state, move |_| litradar_storage::random_hex(16)).await?;
    let now = current_epoch_seconds();
    let user_id = user.id.value();
    let run = run_storage(&state, move |storage| {
        litradar_storage::admit_manual_delivery_run(
            storage.auth_db_path(),
            &DeliveryRunCreate {
                external_id: job_id,
                workflow: DeliveryWorkflow::Push,
                scope_key: format!("manual-user-{user_id}"),
                db_name: None,
                trigger_kind: DeliveryTriggerKind::Manual,
                mode: DeliveryRunMode::Execute,
                user_id: Some(user_id),
                deadline_at: Some(now + MANUAL_DELIVERY_JOB_DEADLINE_SECONDS as f64),
                created_at: now,
            },
        )
    })
    .await?;
    let run = match run {
        ManualDeliveryRunAdmissionOutcome::Admitted(
            DeliveryRunAdmissionOutcome::Enqueued(run)
            | DeliveryRunAdmissionOutcome::Existing(run)
            | DeliveryRunAdmissionOutcome::Busy(run),
        ) => run,
        ManualDeliveryRunAdmissionOutcome::BlockedUnknown(_) => {
            return Err(ApiError::conflict(
                "Manual push outcome is unknown; review delivery state before retrying",
            ));
        }
    };
    Ok((StatusCode::ACCEPTED, Json(manual_push_status(&run))))
}

/// Get the current manual weekly-push job status for the authenticated user.
#[utoipa::path(
    get,
    path = "/api/tracking/push-weekly/status",
    tag = "tracking",
    responses((status = 200, description = "Manual weekly push status.", body = ManualWeeklyPushStatus)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_push_weekly_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<ManualWeeklyPushStatus>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let run = run_storage(&state, move |storage| {
        litradar_storage::load_latest_manual_delivery_run(storage.auth_db_path(), user.id.value())
    })
    .await?;
    Ok(Json(
        run.as_ref()
            .map(manual_push_status)
            .unwrap_or_else(idle_manual_push_status),
    ))
}

/// Get one durable manual weekly-push run visible to its owner or an administrator.
#[utoipa::path(
    get,
    path = "/api/tracking/push-weekly/runs/{run_id}",
    tag = "tracking",
    params(("run_id" = String, Path, description = "Opaque manual push job id.")),
    responses(
        (status = 200, description = "Durable manual weekly push status.", body = ManualWeeklyPushStatus),
        (status = 404, description = "Manual push job not found.", body = ErrorEnvelope)
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_push_weekly_run(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Result<Json<ManualWeeklyPushStatus>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    validate_manual_job_id(&run_id)?;
    let run = load_authorized_manual_run(&state, user.id.value(), user.is_admin, run_id).await?;
    Ok(Json(manual_push_status(&run)))
}

/// Request cancellation of one durable manual weekly-push run.
#[utoipa::path(
    post,
    path = "/api/tracking/push-weekly/runs/{run_id}/cancel",
    tag = "tracking",
    params(("run_id" = String, Path, description = "Opaque manual push job id.")),
    responses(
        (status = 200, description = "Cancellation state.", body = ManualWeeklyPushStatus),
        (status = 404, description = "Manual push job not found.", body = ErrorEnvelope)
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn cancel_push_weekly_run(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Result<Json<ManualWeeklyPushStatus>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    validate_manual_job_id(&run_id)?;
    let run = load_authorized_manual_run(&state, user.id.value(), user.is_admin, run_id).await?;
    if run.status.is_terminal() || run.cancellation_requested {
        return Ok(Json(manual_push_status(&run)));
    }
    let updated =
        run_storage(
            &state,
            move |storage| match litradar_storage::request_delivery_run_cancellation(
                storage.auth_db_path(),
                run.id,
                run.revision,
                current_epoch_seconds(),
            ) {
                Ok(updated) => Ok(updated),
                Err(DeliveryRepositoryError::Conflict) => {
                    litradar_storage::load_delivery_run(storage.auth_db_path(), run.id)?
                        .ok_or(DeliveryRepositoryError::NotFound)
                }
                Err(error) => Err(error),
            },
        )
        .await?;
    Ok(Json(manual_push_status(&updated)))
}

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
    let (user, _) = require_current_user(&state, &headers).await?;
    let (folder, folders, settings, weekly_articles_available) = run_storage(&state, {
        let secret_codec = state.secret_codec().clone();
        move |storage| {
            let folder = litradar_storage::get_tracking_folder(storage.auth_db_path(), user.id)?;
            let folders = litradar_storage::list_folders(storage.auth_db_path(), user.id)?;
            let settings = litradar_storage::get_notification_settings(
                storage.auth_db_path(),
                &secret_codec,
                user.id,
            )?;
            let selected_databases = settings
                .as_ref()
                .map(|item| item.selected_databases.as_slice())
                .unwrap_or_default();
            let weekly_articles_available =
                litradar_storage::count_weekly_articles(&storage, selected_databases)?;
            Ok::<_, litradar_storage::BusinessRepositoryError>((
                folder,
                folders,
                settings,
                weekly_articles_available,
            ))
        }
    })
    .await?;
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
    let (user, _) = require_current_user(&state, &headers).await?;
    let secret_codec = state.secret_codec().clone();
    let settings = run_storage(&state, move |storage| {
        litradar_storage::get_notification_settings(storage.auth_db_path(), &secret_codec, user.id)
    })
    .await?;
    Ok(Json(
        settings.as_ref().map(NotificationSettingsResponse::from),
    ))
}

/// Get the administrator-approved AI endpoint catalog.
#[utoipa::path(
    get,
    path = "/api/tracking/ai-endpoints",
    tag = "tracking",
    responses((status = 200, description = "Approved AI base URLs.", body = Vec<String>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_ai_endpoints(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, ApiError> {
    require_current_user(&state, &headers).await?;
    let endpoints = run_storage(&state, move |storage| {
        litradar_storage::load_ai_allowed_base_urls(storage.auth_db_path())
    })
    .await?;
    Ok(Json(endpoints))
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
    let (user, _) = require_current_user(&state, &headers).await?;
    validate_notification_update(&body)?;
    if !(NOTIFICATION_AI_RETRY_ATTEMPTS_MIN..=NOTIFICATION_AI_RETRY_ATTEMPTS_MAX)
        .contains(&body.ai_retry_attempts)
    {
        return Err(ApiError::bad_request(format!(
            "ai_retry_attempts must be between {NOTIFICATION_AI_RETRY_ATTEMPTS_MIN} and {NOTIFICATION_AI_RETRY_ATTEMPTS_MAX}"
        )));
    }
    let requested_databases = body.selected_databases;
    let (available_databases, mut selected_databases) = run_storage(&state, move |storage| {
        let available_databases = litradar_storage::list_available_database_names(&storage)?;
        let selected_databases = litradar_storage::normalize_database_names(&requested_databases);
        Ok::<_, litradar_storage::BusinessRepositoryError>((
            available_databases,
            selected_databases,
        ))
    })
    .await?;
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
    let existing_secret_codec = state.secret_codec().clone();
    let existing_settings = run_storage(&state, move |storage| {
        litradar_storage::get_notification_settings(
            storage.auth_db_path(),
            &existing_secret_codec,
            user.id,
        )
    })
    .await?;
    let has_effective_pushplus_token = match body.pushplus_token.as_ref() {
        Some(None) => false,
        Some(Some(value)) if !value.trim().is_empty() => true,
        _ => existing_settings
            .as_ref()
            .is_some_and(|settings| !settings.pushplus_token.is_empty()),
    };
    if body.delivery_method == "pushplus" && !has_effective_pushplus_token {
        return Err(ApiError::bad_request(
            "pushplus_token is required when delivery_method is 'pushplus'",
        ));
    }
    if body.delivery_method == "pushplus"
        && body.sync_to_tracking_folder
        && run_storage(&state, move |storage| {
            litradar_storage::get_tracking_folder(storage.auth_db_path(), user.id)
        })
        .await?
        .is_none()
    {
        return Err(ApiError::bad_request(
            "A tracking folder is required before enabling PushPlus sync to tracking",
        ));
    }
    let allowed_ai_endpoints = run_storage(&state, move |storage| {
        litradar_storage::load_ai_allowed_base_urls(storage.auth_db_path())
    })
    .await?;
    let ai_base_url = normalize_selected_ai_endpoint(&body.ai_base_url, &allowed_ai_endpoints)?;
    let ai_backup_base_url =
        normalize_selected_ai_endpoint(&body.ai_backup_base_url, &allowed_ai_endpoints)?;
    let normalized = NotificationSettingsUpdate {
        keywords: trim_nonempty(body.keywords),
        directions: trim_nonempty(body.directions),
        selected_databases,
        delivery_method: body.delivery_method,
        pushplus_token: normalize_secret_update(body.pushplus_token),
        pushplus_template: nonempty_or_default(body.pushplus_template, "markdown"),
        pushplus_topic: body.pushplus_topic.trim().to_string(),
        pushplus_channel: body.pushplus_channel.trim().to_string(),
        sync_to_tracking_folder: body.sync_to_tracking_folder,
        ai_base_url,
        ai_api_key: normalize_secret_update(body.ai_api_key),
        ai_model: body.ai_model.trim().to_string(),
        ai_system_prompt: body.ai_system_prompt.trim().to_string(),
        ai_backup_base_url,
        ai_backup_api_key: normalize_secret_update(body.ai_backup_api_key),
        ai_backup_model: body.ai_backup_model.trim().to_string(),
        ai_backup_system_prompt: body.ai_backup_system_prompt.trim().to_string(),
        ai_retry_attempts: body.ai_retry_attempts,
        enabled: body.enabled,
    };
    let secret_codec = state.secret_codec().clone();
    let storage = state.storage_config().clone();
    let settings = state
        .run_blocking(move || {
            litradar_storage::upsert_notification_settings(
                storage.auth_db_path(),
                &secret_codec,
                user.id,
                &normalized,
            )
        })
        .await?
        .map_err(|error| match error {
            litradar_storage::BusinessRepositoryError::OutboundEndpointNotAllowed => {
                ApiError::bad_request("AI endpoint is not available")
            }
            litradar_storage::BusinessRepositoryError::InvalidInput(error) => {
                ApiError::bad_request(error.to_string())
            }
            _ => ApiError::internal_server_error(),
        })?;
    Ok(Json(NotificationSettingsResponse::from(&settings)))
}

fn validate_notification_update(body: &NotificationSettingsUpdate) -> Result<(), ApiError> {
    litradar_domain::validate_notification_settings(body)
        .map_err(|error| ApiError::bad_request(error.to_string()))
}

fn normalize_selected_ai_endpoint(
    value: &str,
    allowed_endpoints: &[String],
) -> Result<String, ApiError> {
    if value.trim().is_empty() {
        return Ok(String::new());
    }
    let endpoint = litradar_storage::canonicalize_outbound_base_url(value)
        .map_err(|_| ApiError::bad_request("AI endpoint is not available"))?;
    if allowed_endpoints.contains(&endpoint) {
        Ok(endpoint)
    } else {
        Err(ApiError::bad_request("AI endpoint is not available"))
    }
}

fn normalize_secret_update(update: Option<Option<String>>) -> Option<Option<String>> {
    update.map(|value| value.map(|secret| secret.trim().to_string()))
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

fn idle_manual_push_status() -> ManualWeeklyPushStatus {
    ManualWeeklyPushStatus {
        job_id: None,
        status: ManualPushState::Idle,
        message: MANUAL_PUSH_IDLE_MESSAGE.to_string(),
        started_at: None,
        finished_at: None,
        deadline_at: None,
        cancellation_requested: false,
        can_cancel: false,
        can_retry: false,
        pushed: 0,
        selected: 0,
        total_candidates: None,
        summary: String::new(),
        folder_id: None,
        folder_name: None,
    }
}

async fn run_storage<Output, StorageError, Work>(
    state: &ApiState,
    work: Work,
) -> Result<Output, ApiError>
where
    Work: FnOnce(StorageConfig) -> Result<Output, StorageError> + Send + 'static,
    Output: Send + 'static,
    StorageError: Send + 'static,
{
    let storage = state.storage_config().clone();
    state
        .run_blocking(move || work(storage))
        .await?
        .map_err(|_| ApiError::internal_server_error())
}

fn manual_push_status(run: &DeliveryRunRecord) -> ManualWeeklyPushStatus {
    let outcome = run
        .result_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<ManualWeeklyPushOutcome>(value).ok());
    let public_status = public_manual_status(run.status);
    let message = outcome
        .as_ref()
        .map(|outcome| outcome.message.clone())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| manual_status_message(run).to_string());
    ManualWeeklyPushStatus {
        job_id: Some(run.external_id.clone()),
        status: public_status,
        message,
        started_at: run.started_at,
        finished_at: run.finished_at,
        deadline_at: run.deadline_at,
        cancellation_requested: run.cancellation_requested,
        can_cancel: !run.status.is_terminal() && !run.cancellation_requested,
        can_retry: matches!(
            run.status,
            DeliveryRunStatus::Failed | DeliveryRunStatus::Cancelled | DeliveryRunStatus::TimedOut
        ),
        pushed: outcome.as_ref().map_or(0, |outcome| outcome.pushed),
        selected: outcome.as_ref().map_or(0, |outcome| outcome.selected),
        total_candidates: outcome
            .as_ref()
            .and_then(|outcome| outcome.total_candidates),
        summary: outcome
            .as_ref()
            .map(|outcome| outcome.summary.clone())
            .unwrap_or_default(),
        folder_id: outcome.as_ref().and_then(|outcome| outcome.folder_id),
        folder_name: outcome.and_then(|outcome| outcome.folder_name),
    }
}

fn public_manual_status(status: DeliveryRunStatus) -> ManualPushState {
    match status {
        DeliveryRunStatus::Queued => ManualPushState::Pending,
        DeliveryRunStatus::Claimed | DeliveryRunStatus::Running | DeliveryRunStatus::Cancelling => {
            ManualPushState::Running
        }
        DeliveryRunStatus::Completed | DeliveryRunStatus::Skipped => ManualPushState::Completed,
        DeliveryRunStatus::Failed => ManualPushState::Failed,
        DeliveryRunStatus::Cancelled => ManualPushState::Cancelled,
        DeliveryRunStatus::TimedOut => ManualPushState::TimedOut,
        DeliveryRunStatus::Unknown => ManualPushState::Unknown,
    }
}

fn manual_status_message(run: &DeliveryRunRecord) -> &'static str {
    match run.status {
        DeliveryRunStatus::Queued => "Manual push is queued",
        DeliveryRunStatus::Claimed | DeliveryRunStatus::Running => "Manual push is running",
        DeliveryRunStatus::Cancelling => "Manual push cancellation is pending",
        DeliveryRunStatus::Completed => "Manual push completed",
        DeliveryRunStatus::Skipped => "Manual push completed without applicable work",
        DeliveryRunStatus::Failed => "Manual push failed",
        DeliveryRunStatus::Cancelled => "Manual push was cancelled",
        DeliveryRunStatus::TimedOut => "Manual push exceeded its deadline",
        DeliveryRunStatus::Unknown => {
            "Manual push outcome is unknown; review delivery state before retrying"
        }
    }
}

async fn load_authorized_manual_run(
    state: &ApiState,
    user_id: i64,
    is_admin: bool,
    run_id: String,
) -> Result<DeliveryRunRecord, ApiError> {
    let run = run_storage(state, move |storage| {
        if is_admin {
            litradar_storage::load_manual_delivery_run_by_external_id_for_admin(
                storage.auth_db_path(),
                &run_id,
            )
        } else {
            litradar_storage::load_manual_delivery_run_by_external_id(
                storage.auth_db_path(),
                user_id,
                &run_id,
            )
        }
    })
    .await?;
    run.ok_or_else(|| ApiError::not_found("Manual push job not found"))
}

fn validate_manual_job_id(run_id: &str) -> Result<(), ApiError> {
    if run_id.len() == 32 && run_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(ApiError::not_found("Manual push job not found"))
    }
}

fn current_epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use litradar_domain::{ManualPushState, NotificationSettingsUpdate};
    use litradar_storage::{
        DeliveryRunMode, DeliveryRunRecord, DeliveryRunStatus, DeliveryTriggerKind,
        DeliveryWorkflow,
    };

    use super::{manual_push_status, validate_notification_update};

    #[test]
    fn notification_route_validation_uses_shared_unicode_and_item_limits() {
        let mut settings = serde_json::from_str::<NotificationSettingsUpdate>("{}")
            .expect("default notification settings should deserialize");
        settings.keywords = vec!["keyword".to_string(); litradar_domain::MAX_NOTIFICATION_KEYWORDS];
        settings.ai_system_prompt = "文".repeat(litradar_domain::MAX_NOTIFICATION_PROMPT_CHARS);
        validate_notification_update(&settings).expect("notification boundaries should pass");

        settings.keywords.push("overflow".to_string());
        assert!(validate_notification_update(&settings).is_err());
        settings.keywords.pop();
        settings.ai_system_prompt.push('文');
        assert!(validate_notification_update(&settings).is_err());
    }

    #[test]
    fn manual_push_status_maps_durable_states_and_safe_actions() {
        let queued = fixture_run(DeliveryRunStatus::Queued);
        let pending = manual_push_status(&queued);
        assert_eq!(pending.status, ManualPushState::Pending);
        assert!(pending.can_cancel);
        assert!(!pending.can_retry);

        let mut unknown = fixture_run(DeliveryRunStatus::Unknown);
        unknown.cancellation_requested = true;
        unknown.finished_at = Some(3.0);
        let unknown = manual_push_status(&unknown);
        assert_eq!(unknown.status, ManualPushState::Unknown);
        assert!(!unknown.can_cancel);
        assert!(!unknown.can_retry);
        assert!(!unknown.message.contains("upstream"));

        assert!(!manual_push_status(&fixture_run(DeliveryRunStatus::Completed)).can_retry);
        assert!(!manual_push_status(&fixture_run(DeliveryRunStatus::Skipped)).can_retry);
        assert!(manual_push_status(&fixture_run(DeliveryRunStatus::Failed)).can_retry);
        assert!(manual_push_status(&fixture_run(DeliveryRunStatus::Cancelled)).can_retry);
        assert!(manual_push_status(&fixture_run(DeliveryRunStatus::TimedOut)).can_retry);
    }

    fn fixture_run(status: DeliveryRunStatus) -> DeliveryRunRecord {
        DeliveryRunRecord {
            id: 1,
            external_id: "0123456789abcdef0123456789abcdef".to_string(),
            workflow: DeliveryWorkflow::Push,
            scope_key: "manual-user-1".to_string(),
            db_name: None,
            trigger_kind: DeliveryTriggerKind::Manual,
            mode: DeliveryRunMode::Execute,
            user_id: Some(1),
            status,
            legacy_status: None,
            owner_id: None,
            lease_expires_at: None,
            deadline_at: Some(600.0),
            cancellation_requested: false,
            result_json: None,
            error_code: None,
            revision: 0,
            created_at: 1.0,
            started_at: None,
            updated_at: 1.0,
            finished_at: None,
        }
    }
}
