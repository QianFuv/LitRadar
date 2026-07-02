//! Public route registration for the Rust API.

pub mod announcements;
pub mod health;

use axum::Router;

use crate::state::ApiState;

/// Build the public route set for the current migration phase.
///
/// # Returns
///
/// Router containing only public compatibility endpoints.
pub fn public_routes() -> Router<ApiState> {
    Router::new()
        .route("/health", axum::routing::get(health::health))
        .route(
            "/announcements",
            axum::routing::get(announcements::get_announcements),
        )
}
