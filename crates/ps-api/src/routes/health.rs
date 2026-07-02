//! Health route handler.

use axum::Json;
use ps_domain::HealthResponse;

/// Return the public health status payload.
///
/// # Returns
///
/// JSON health response.
pub(super) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::ok())
}
