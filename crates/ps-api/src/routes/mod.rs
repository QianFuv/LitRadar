//! Public route registration for the Rust API.

pub mod admin;
pub mod announcements;
pub mod auth;
pub mod favorites;
pub mod health;
pub mod index;
pub mod tracking;

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
        .route("/meta/databases", axum::routing::get(index::list_databases))
        .route("/meta/areas", axum::routing::get(index::list_areas))
        .route(
            "/meta/journals",
            axum::routing::get(index::list_journal_options),
        )
        .route("/meta/sources", axum::routing::get(index::list_sources))
        .route("/years", axum::routing::get(index::list_years))
        .route("/journals", axum::routing::get(index::list_journals))
        .route(
            "/journals/{journal_id}",
            axum::routing::get(index::get_journal),
        )
        .route("/issues", axum::routing::get(index::list_issues))
        .route("/issues/{issue_id}", axum::routing::get(index::get_issue))
        .route(
            "/weekly-updates",
            axum::routing::get(index::get_weekly_updates),
        )
        .route("/articles", axum::routing::get(index::list_articles))
        .route(
            "/articles/{article_id}",
            axum::routing::get(index::get_article),
        )
        .route(
            "/articles/{article_id}/access",
            axum::routing::get(index::get_article_access),
        )
        .route(
            "/articles/{article_id}/fulltext",
            axum::routing::get(index::redirect_article_fulltext),
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
        .route(
            "/favorites/folders",
            axum::routing::get(favorites::list_folders).post(favorites::create_folder),
        )
        .route(
            "/favorites/folders/{folder_id}",
            axum::routing::put(favorites::rename_folder).delete(favorites::delete_folder),
        )
        .route(
            "/favorites/tracking",
            axum::routing::get(favorites::get_tracking).put(favorites::set_tracking),
        )
        .route(
            "/favorites/folders/{folder_id}/articles",
            axum::routing::get(favorites::list_folder_articles).post(favorites::add_favorite),
        )
        .route(
            "/favorites/folders/{folder_id}/count",
            axum::routing::get(favorites::folder_count),
        )
        .route(
            "/favorites/folders/{folder_id}/export",
            axum::routing::get(favorites::export_folder),
        )
        .route(
            "/favorites/folders/{folder_id}/articles/{article_id}",
            axum::routing::delete(favorites::remove_favorite),
        )
        .route(
            "/favorites/folders/{folder_id}/articles/bulk",
            axum::routing::post(favorites::bulk_add),
        )
        .route(
            "/favorites/folders/{folder_id}/articles/bulk-remove",
            axum::routing::post(favorites::bulk_remove),
        )
        .route(
            "/favorites/folders/{folder_id}/articles/bulk-move",
            axum::routing::post(favorites::bulk_move),
        )
        .route(
            "/favorites/check",
            axum::routing::get(favorites::check_favorite),
        )
        .route(
            "/favorites/check/batch",
            axum::routing::post(favorites::check_favorites_batch),
        )
        .route("/tracking/status", axum::routing::get(tracking::status))
        .route(
            "/tracking/notification-settings",
            axum::routing::get(tracking::get_notification_settings)
                .put(tracking::update_notification_settings),
        )
        .route("/admin/users", axum::routing::get(admin::list_users))
        .route(
            "/admin/users/{user_id}/admin",
            axum::routing::put(admin::set_admin),
        )
        .route(
            "/admin/users/{user_id}/reset-password",
            axum::routing::post(admin::reset_password),
        )
        .route(
            "/admin/users/{user_id}",
            axum::routing::delete(admin::delete_user),
        )
        .route(
            "/admin/invite-codes",
            axum::routing::get(admin::list_invite_codes).post(admin::create_invite_code),
        )
        .route(
            "/admin/invite-codes/{code_id}",
            axum::routing::delete(admin::delete_invite_code),
        )
        .route("/admin/stats", axum::routing::get(admin::stats))
        .route(
            "/admin/scheduled-tasks",
            axum::routing::get(admin::list_scheduled_tasks).post(admin::create_scheduled_task),
        )
        .route(
            "/admin/scheduled-tasks/{task_id}",
            axum::routing::put(admin::update_scheduled_task).delete(admin::delete_scheduled_task),
        )
        .route(
            "/admin/runtime-settings",
            axum::routing::get(admin::list_runtime_settings).put(admin::update_runtime_settings),
        )
        .route(
            "/admin/announcements",
            axum::routing::get(admin::list_announcements).post(admin::create_announcement),
        )
        .route(
            "/admin/announcements/{announcement_id}",
            axum::routing::put(admin::update_announcement).delete(admin::delete_announcement),
        )
}
