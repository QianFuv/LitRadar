//! Tracking status and notification settings route handlers.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use ps_domain::{
    ManualWeeklyPushStatus, NotificationSettingsResponse, NotificationSettingsUpdate,
    TrackingFolderSummary, TrackingStatusResponse, NOTIFICATION_AI_RETRY_ATTEMPTS_MAX,
    NOTIFICATION_AI_RETRY_ATTEMPTS_MIN,
};
use ps_storage::StorageConfig;
use ps_worker::delivery::{
    run_manual_weekly_push, ManualWeeklyPushConfig, ManualWeeklyPushOutcome,
};

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

const ALLOWED_DELIVERY_METHODS: [&str; 2] = ["folder", "pushplus"];
const MANUAL_PUSH_STARTED_MESSAGE: &str = "Manual push started and is running in the background";
const MANUAL_PUSH_IDLE_MESSAGE: &str = "No manual push task is running";

static MANUAL_PUSH_JOBS: OnceLock<Mutex<HashMap<String, ManualWeeklyPushStatus>>> = OnceLock::new();
#[cfg(test)]
static MANUAL_PUSH_TEST_DELAY_MS: OnceLock<Mutex<Option<u64>>> = OnceLock::new();

/// Start one manual weekly-push job for the authenticated user.
#[utoipa::path(
    post,
    path = "/api/tracking/push-weekly",
    tag = "tracking",
    responses((status = 200, description = "Manual weekly push status.", body = ManualWeeklyPushStatus)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn push_weekly_to_tracking(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<ManualWeeklyPushStatus>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let key = manual_push_key(&state, user.id);
    if let Some(status) = current_manual_push_status(&key) {
        if status.status == "running" {
            return Ok(Json(status));
        }
    }

    let job_id = run_storage(&state, move |storage| {
        ps_storage::random_hex(storage.auth_db_path(), 16)
    })
    .await?;
    let started_at = current_epoch_seconds();
    let status = manual_push_status(
        Some(job_id.clone()),
        "running",
        MANUAL_PUSH_STARTED_MESSAGE,
        Some(started_at),
        None,
        ManualWeeklyPushOutcome {
            status: "running".to_string(),
            message: MANUAL_PUSH_STARTED_MESSAGE.to_string(),
            pushed: 0,
            selected: 0,
            total_candidates: None,
            summary: String::new(),
            folder_id: None,
            folder_name: None,
        },
    );
    set_manual_push_status(key.clone(), status.clone());
    let config = ManualWeeklyPushConfig {
        storage_config: state.storage_config().clone(),
        secret_codec: state.secret_codec().clone(),
        user_id: user.id,
        ai_model: None,
        max_candidates: None,
        timeout_seconds: 120,
        retry_attempts: 3,
        dedupe_retention_days: 60,
    };
    spawn_manual_push_job(state, key, job_id, started_at, config);
    Ok(Json(status))
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
    let key = manual_push_key(&state, user.id);
    Ok(Json(
        current_manual_push_status(&key).unwrap_or_else(idle_manual_push_status),
    ))
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
            let folder = ps_storage::get_tracking_folder(storage.auth_db_path(), user.id)?;
            let folders = ps_storage::list_folders(storage.auth_db_path(), user.id)?;
            let settings = ps_storage::get_notification_settings(
                storage.auth_db_path(),
                &secret_codec,
                user.id,
            )?;
            let selected_databases = settings
                .as_ref()
                .map(|item| item.selected_databases.as_slice())
                .unwrap_or_default();
            let weekly_articles_available =
                ps_storage::count_weekly_articles(&storage, selected_databases)?;
            Ok::<_, ps_storage::BusinessRepositoryError>((
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
        ps_storage::get_notification_settings(storage.auth_db_path(), &secret_codec, user.id)
    })
    .await?;
    Ok(Json(
        settings.as_ref().map(NotificationSettingsResponse::from),
    ))
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
    if !(NOTIFICATION_AI_RETRY_ATTEMPTS_MIN..=NOTIFICATION_AI_RETRY_ATTEMPTS_MAX)
        .contains(&body.ai_retry_attempts)
    {
        return Err(ApiError::bad_request(format!(
            "ai_retry_attempts must be between {NOTIFICATION_AI_RETRY_ATTEMPTS_MIN} and {NOTIFICATION_AI_RETRY_ATTEMPTS_MAX}"
        )));
    }
    let requested_databases = body.selected_databases;
    let (available_databases, mut selected_databases) = run_storage(&state, move |storage| {
        let available_databases = ps_storage::list_available_database_names(&storage)?;
        let selected_databases = ps_storage::normalize_database_names(&requested_databases);
        Ok::<_, ps_storage::BusinessRepositoryError>((available_databases, selected_databases))
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
        ps_storage::get_notification_settings(
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
            ps_storage::get_tracking_folder(storage.auth_db_path(), user.id)
        })
        .await?
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
        pushplus_token: normalize_secret_update(body.pushplus_token),
        pushplus_template: nonempty_or_default(body.pushplus_template, "markdown"),
        pushplus_topic: body.pushplus_topic.trim().to_string(),
        pushplus_channel: body.pushplus_channel.trim().to_string(),
        sync_to_tracking_folder: body.sync_to_tracking_folder,
        ai_base_url: body.ai_base_url.trim().to_string(),
        ai_api_key: normalize_secret_update(body.ai_api_key),
        ai_model: body.ai_model.trim().to_string(),
        ai_system_prompt: body.ai_system_prompt.trim().to_string(),
        ai_backup_base_url: body.ai_backup_base_url.trim().to_string(),
        ai_backup_api_key: normalize_secret_update(body.ai_backup_api_key),
        ai_backup_model: body.ai_backup_model.trim().to_string(),
        ai_backup_system_prompt: body.ai_backup_system_prompt.trim().to_string(),
        ai_retry_attempts: body.ai_retry_attempts,
        enabled: body.enabled,
    };
    let secret_codec = state.secret_codec().clone();
    let settings = run_storage(&state, move |storage| {
        ps_storage::upsert_notification_settings(
            storage.auth_db_path(),
            &secret_codec,
            user.id,
            &normalized,
        )
    })
    .await?;
    Ok(Json(NotificationSettingsResponse::from(&settings)))
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

fn manual_push_jobs() -> &'static Mutex<HashMap<String, ManualWeeklyPushStatus>> {
    MANUAL_PUSH_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn manual_push_key(state: &ApiState, user_id: ps_domain::UserId) -> String {
    format!(
        "{}:{}",
        state.storage_config().auth_db_path().display(),
        user_id.value()
    )
}

fn current_manual_push_status(key: &str) -> Option<ManualWeeklyPushStatus> {
    manual_push_jobs()
        .lock()
        .expect("manual push jobs lock should not be poisoned")
        .get(key)
        .cloned()
}

fn set_manual_push_status(key: String, status: ManualWeeklyPushStatus) {
    manual_push_jobs()
        .lock()
        .expect("manual push jobs lock should not be poisoned")
        .insert(key, status);
}

fn idle_manual_push_status() -> ManualWeeklyPushStatus {
    ManualWeeklyPushStatus {
        job_id: None,
        status: "idle".to_string(),
        message: MANUAL_PUSH_IDLE_MESSAGE.to_string(),
        started_at: None,
        finished_at: None,
        pushed: 0,
        selected: 0,
        total_candidates: None,
        summary: String::new(),
        folder_id: None,
        folder_name: None,
    }
}

fn spawn_manual_push_job(
    state: ApiState,
    key: String,
    job_id: String,
    started_at: f64,
    config: ManualWeeklyPushConfig,
) {
    tokio::spawn(async move {
        let finished = state
            .run_background_blocking(move || {
                delay_manual_push_for_tests();
                run_manual_weekly_push(&config)
            })
            .await;
        let finished_at = current_epoch_seconds();
        let status = match finished {
            Ok(Ok(outcome)) => {
                let outcome_status = outcome.status.clone();
                let outcome_message = outcome.message.clone();
                manual_push_status(
                    Some(job_id.clone()),
                    &outcome_status,
                    &outcome_message,
                    Some(started_at),
                    Some(finished_at),
                    outcome,
                )
            }
            Ok(Err(error)) => failed_manual_push_status(
                Some(job_id.clone()),
                started_at,
                finished_at,
                &format!("Manual push failed: {error}"),
            ),
            Err(error) => failed_manual_push_status(
                Some(job_id.clone()),
                started_at,
                finished_at,
                &format!("Manual push failed: {error}"),
            ),
        };
        update_manual_push_status_if_current(key, &job_id, status);
    });
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

fn update_manual_push_status_if_current(key: String, job_id: &str, status: ManualWeeklyPushStatus) {
    let mut jobs = manual_push_jobs()
        .lock()
        .expect("manual push jobs lock should not be poisoned");
    let Some(current) = jobs.get(&key) else {
        return;
    };
    if current.job_id.as_deref() == Some(job_id) {
        jobs.insert(key, status);
    }
}

fn manual_push_status(
    job_id: Option<String>,
    status: &str,
    message: &str,
    started_at: Option<f64>,
    finished_at: Option<f64>,
    outcome: ManualWeeklyPushOutcome,
) -> ManualWeeklyPushStatus {
    ManualWeeklyPushStatus {
        job_id,
        status: status.to_string(),
        message: message.to_string(),
        started_at,
        finished_at,
        pushed: outcome.pushed,
        selected: outcome.selected,
        total_candidates: outcome.total_candidates,
        summary: outcome.summary,
        folder_id: outcome.folder_id,
        folder_name: outcome.folder_name,
    }
}

fn failed_manual_push_status(
    job_id: Option<String>,
    started_at: f64,
    finished_at: f64,
    message: &str,
) -> ManualWeeklyPushStatus {
    ManualWeeklyPushStatus {
        job_id,
        status: "failed".to_string(),
        message: message.to_string(),
        started_at: Some(started_at),
        finished_at: Some(finished_at),
        pushed: 0,
        selected: 0,
        total_candidates: None,
        summary: String::new(),
        folder_id: None,
        folder_name: None,
    }
}

fn current_epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
fn delay_manual_push_for_tests() {
    if let Some(delay_millis) = manual_push_test_delay_ms() {
        std::thread::sleep(std::time::Duration::from_millis(delay_millis));
    }
}

#[cfg(not(test))]
fn delay_manual_push_for_tests() {}

#[cfg(test)]
fn manual_push_test_delay_ms() -> Option<u64> {
    *MANUAL_PUSH_TEST_DELAY_MS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("manual push test delay lock should not be poisoned")
}

/// Set the manual push background delay for route tests.
///
/// # Arguments
///
/// * `delay_millis` - Optional artificial delay in milliseconds.
#[cfg(test)]
pub(crate) fn set_manual_push_test_delay_ms(delay_millis: Option<u64>) {
    *MANUAL_PUSH_TEST_DELAY_MS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("manual push test delay lock should not be poisoned") = delay_millis;
}
