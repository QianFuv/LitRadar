//! Health route handler.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use litradar_domain::HealthResponse;

use crate::state::ApiState;

/// Return application event-loop liveness.
///
/// # Returns
///
/// JSON health response.
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    responses((status = 200, description = "The application event loop is live.", body = HealthResponse))
)]
pub(crate) async fn live() -> Json<HealthResponse> {
    Json(HealthResponse::ok())
}

/// Return whether the embedded scheduler has a recent persisted heartbeat.
///
/// # Arguments
///
/// * `state` - API state containing the shared scheduler database path.
///
/// # Returns
///
/// HTTP 200 when ready or 503 until the embedded scheduler heartbeat is healthy.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "The embedded scheduler heartbeat is healthy.", body = HealthResponse),
        (status = 503, description = "The embedded scheduler is not ready.", body = HealthResponse)
    )
)]
pub(crate) async fn ready(State(state): State<ApiState>) -> (StatusCode, Json<HealthResponse>) {
    let auth_db_path = state.storage_config().auth_db_path().to_path_buf();
    let now = current_unix_time();
    let status = state
        .run_blocking_with_timeout(Duration::from_secs(1), move || {
            litradar_storage::get_scheduler_status(
                &auth_db_path,
                now,
                litradar_worker::scheduler::SCHEDULER_HEALTH_WINDOW_SECONDS,
                0,
            )
        })
        .await;
    let is_healthy = status
        .ok()
        .and_then(Result::ok)
        .is_some_and(|status| status.workers.iter().any(|worker| worker.is_healthy));
    if is_healthy {
        (StatusCode::OK, Json(HealthResponse::ok()))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "unhealthy".to_string(),
            }),
        )
    }
}

fn current_unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}
