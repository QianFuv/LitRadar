//! Health route handler.

use axum::Json;
use ps_domain::HealthResponse;

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
