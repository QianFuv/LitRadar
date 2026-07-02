//! Public route registration for the Rust API.

pub mod announcements;
pub mod auth;
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
        .route("/auth/register", axum::routing::post(auth::register))
        .route("/auth/login", axum::routing::post(auth::login))
        .route("/auth/me", axum::routing::get(auth::get_me))
        .route(
            "/auth/change-password",
            axum::routing::post(auth::change_password),
        )
        .route("/auth/logout", axum::routing::post(auth::logout))
        .route(
            "/auth/tokens",
            axum::routing::post(auth::create_token).get(auth::get_tokens),
        )
        .route(
            "/auth/tokens/{token_id}",
            axum::routing::delete(auth::delete_token),
        )
        .route(
            "/auth/invite-code",
            axum::routing::post(auth::generate_invite_code).get(auth::get_invite_code),
        )
        .route(
            "/auth/invite-required",
            axum::routing::get(auth::check_invite_required),
        )
}
