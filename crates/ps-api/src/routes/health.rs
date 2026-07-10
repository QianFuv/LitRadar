//! Health route handler.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ps_domain::HealthResponse;

use crate::state::ApiState;

/// Return the public health status payload.
///
/// # Returns
///
/// JSON health response.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "health",
    responses((status = 200, description = "Service is healthy.", body = HealthResponse))
)]
pub(crate) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::ok())
}

/// Return whether at least one scheduler worker has a recent persisted heartbeat.
///
/// # Arguments
///
/// * `state` - API state containing the shared scheduler database path.
///
/// # Returns
///
/// HTTP 200 for a healthy worker or 503 when no healthy heartbeat exists.
#[utoipa::path(
    get,
    path = "/api/health/worker",
    tag = "health",
    responses(
        (status = 200, description = "A scheduler worker heartbeat is healthy.", body = HealthResponse),
        (status = 503, description = "No scheduler worker heartbeat is healthy.", body = HealthResponse)
    )
)]
pub(crate) async fn worker_health(
    State(state): State<ApiState>,
) -> (StatusCode, Json<HealthResponse>) {
    let auth_db_path = state.storage_config().auth_db_path().to_path_buf();
    let now = current_unix_time();
    let status = state
        .run_blocking_with_timeout(Duration::from_secs(1), move || {
            ps_storage::get_scheduler_status(
                &auth_db_path,
                now,
                ps_worker::scheduler::SCHEDULER_HEALTH_WINDOW_SECONDS,
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
