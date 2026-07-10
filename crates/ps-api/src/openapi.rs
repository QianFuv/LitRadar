//! Generated OpenAPI document and Swagger UI routing.

use axum::Router;
use ps_auth::SESSION_COOKIE_NAME;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa::openapi::OpenApi as OpenApiDocument;
use utoipa::{Modify, OpenApi};
use utoipa_swagger_ui::SwaggerUi;

use crate::state::ApiState;

/// Browser path for generated Swagger UI.
pub const DOCS_PATH: &str = "/docs";

/// JSON path for the generated OpenAPI document.
pub const OPENAPI_JSON_PATH: &str = "/openapi.json";

/// Generated OpenAPI document for the Rust API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Paper Scanner API",
        version = "0.1.0",
        description = "Generated OpenAPI document for the Rust API."
    ),
    paths(
        crate::routes::health::health,
        crate::routes::health::worker_health,
        crate::routes::announcements::get_announcements,
        crate::routes::index::list_databases,
        crate::routes::index::list_areas,
        crate::routes::index::list_journal_options,
        crate::routes::index::list_sources,
        crate::routes::index::list_years,
        crate::routes::index::list_journals,
        crate::routes::index::get_journal,
        crate::routes::index::list_issues,
        crate::routes::index::get_issue,
        crate::routes::index::get_weekly_updates,
        crate::routes::index::list_articles,
        crate::routes::index::get_article,
        crate::routes::index::get_article_access,
        crate::routes::index::redirect_article_fulltext,
        crate::routes::auth::register,
        crate::routes::auth::login,
        crate::routes::auth::get_me,
        crate::routes::auth::change_password,
        crate::routes::auth::logout,
        crate::routes::auth::create_token,
        crate::routes::auth::get_tokens,
        crate::routes::auth::delete_token,
        crate::routes::auth::generate_invite_code,
        crate::routes::auth::get_invite_code,
        crate::routes::auth::check_invite_required,
        crate::routes::cnki::get_session,
        crate::routes::cnki::clear_session,
        crate::routes::cnki::start_login,
        crate::routes::cnki::poll_login,
        crate::routes::favorites::list_folders,
        crate::routes::favorites::create_folder,
        crate::routes::favorites::rename_folder,
        crate::routes::favorites::delete_folder,
        crate::routes::favorites::get_tracking,
        crate::routes::favorites::set_tracking,
        crate::routes::favorites::list_folder_articles,
        crate::routes::favorites::folder_count,
        crate::routes::favorites::export_folder,
        crate::routes::favorites::add_favorite,
        crate::routes::favorites::remove_favorite,
        crate::routes::favorites::bulk_add,
        crate::routes::favorites::bulk_remove,
        crate::routes::favorites::bulk_move,
        crate::routes::favorites::check_favorite,
        crate::routes::favorites::check_favorites_batch,
        crate::routes::tracking::status,
        crate::routes::tracking::push_weekly_to_tracking,
        crate::routes::tracking::get_push_weekly_status,
        crate::routes::tracking::get_notification_settings,
        crate::routes::tracking::update_notification_settings,
        crate::routes::admin::list_users,
        crate::routes::admin::set_admin,
        crate::routes::admin::reset_password,
        crate::routes::admin::delete_user,
        crate::routes::admin::list_invite_codes,
        crate::routes::admin::create_invite_code,
        crate::routes::admin::delete_invite_code,
        crate::routes::admin::stats,
        crate::routes::admin::list_scheduled_tasks,
        crate::routes::admin::create_scheduled_task,
        crate::routes::admin::update_scheduled_task,
        crate::routes::admin::delete_scheduled_task,
        crate::routes::admin::scheduler_status,
        crate::routes::admin::list_runtime_settings,
        crate::routes::admin::update_runtime_settings,
        crate::routes::admin::list_announcements,
        crate::routes::admin::create_announcement,
        crate::routes::admin::update_announcement,
        crate::routes::admin::delete_announcement
    ),
    components(schemas(
        ps_domain::AdminInviteCodeInfo,
        ps_domain::AdminResetPassword,
        ps_domain::AdminSetAdmin,
        ps_domain::AdminStatsResponse,
        ps_domain::AdminUserInfo,
        ps_domain::AnnouncementCreate,
        ps_domain::AnnouncementInfo,
        ps_domain::AnnouncementUpdate,
        ps_domain::ArticleAccessAction,
        ps_domain::ArticleAccessResponse,
        ps_domain::ArticleId,
        ps_domain::ArticlePage,
        ps_domain::ArticleRecord,
        ps_domain::AuthStats,
        ps_domain::ChangePasswordRequest,
        ps_domain::CnkiErrorDetail,
        ps_domain::CnkiLoginPollRequest,
        ps_domain::CnkiLoginPollResponse,
        ps_domain::CnkiLoginStartResponse,
        ps_domain::CnkiSessionStatusResponse,
        ps_domain::ErrorEnvelope,
        ps_domain::FavoriteAdd,
        ps_domain::FavoriteArticleRef,
        ps_domain::FavoriteArticleResponse,
        ps_domain::FavoriteBatchCheckRequest,
        ps_domain::FavoriteBatchCheckResponse,
        ps_domain::FavoriteBulkAdd,
        ps_domain::FavoriteBulkAddResult,
        ps_domain::FavoriteBulkMove,
        ps_domain::FavoriteBulkRemove,
        ps_domain::FavoriteBulkResult,
        ps_domain::FavoriteCheckResponse,
        ps_domain::FavoriteResponse,
        ps_domain::FavoriteTrackingResponse,
        ps_domain::FolderCreate,
        ps_domain::FolderRename,
        ps_domain::FolderResponse,
        ps_domain::HealthResponse,
        ps_domain::IndexDatabaseStats,
        ps_domain::IndexStats,
        ps_domain::InviteCodeResponse,
        ps_domain::InviteRequiredResponse,
        ps_domain::IssuePage,
        ps_domain::IssueRecord,
        ps_domain::JournalId,
        ps_domain::JournalOption,
        ps_domain::JournalPage,
        ps_domain::JournalRecord,
        ps_domain::LoginRequest,
        ps_domain::LoginResponse,
        ps_domain::LogoutResponse,
        ps_domain::ManualWeeklyPushStatus,
        ps_domain::NotificationSettingsResponse,
        ps_domain::NotificationSettingsUpdate,
        ps_domain::OkResponse,
        ps_domain::PageMeta,
        ps_domain::PushStats,
        ps_domain::RegisterRequest,
        ps_domain::RuntimeSecretItemInfo,
        ps_domain::RuntimeSecretPoolUpdate,
        ps_domain::RuntimeSettingInfo,
        ps_domain::RuntimeSettingsUpdate,
        ps_domain::ScheduledDeliveryJob,
        ps_domain::ScheduledIndexJob,
        ps_domain::ScheduledJobSpec,
        ps_domain::ScheduledTaskCreate,
        ps_domain::ScheduledTaskInfo,
        ps_domain::ScheduledTaskRunInfo,
        ps_domain::ScheduledTaskUpdate,
        ps_domain::SchedulerStatusResponse,
        ps_domain::SchedulerWorkerInfo,
        ps_domain::TokenCreateRequest,
        ps_domain::TokenCreateResponse,
        ps_domain::TokenInfo,
        ps_domain::TrackingFolderSummary,
        ps_domain::TrackingSetRequest,
        ps_domain::TrackingStatusResponse,
        ps_domain::UserId,
        ps_domain::UserResponse,
        ps_domain::ValueCount,
        ps_domain::WeeklyArticleRecord,
        ps_domain::WeeklyDatabaseUpdate,
        ps_domain::WeeklyJournalUpdate,
        ps_domain::WeeklyUpdatesResponse,
        ps_domain::YearSummary
    )),
    tags(
        (name = "health", description = "Service health endpoints."),
        (name = "announcements", description = "Public announcement endpoints."),
        (name = "index", description = "Index database read endpoints."),
        (name = "auth", description = "Authentication and access token endpoints."),
        (name = "cnki", description = "Zhejiang Library CNKI session endpoints."),
        (name = "favorites", description = "Favorite folder endpoints."),
        (name = "tracking", description = "Tracking and notification endpoints."),
        (name = "admin", description = "Administrative endpoints.")
    ),
    modifiers(&SecuritySchemeAddon)
)]
pub(crate) struct ApiDoc;

struct SecuritySchemeAddon;

impl Modify for SecuritySchemeAddon {
    fn modify(&self, openapi: &mut OpenApiDocument) {
        let Some(components) = openapi.components.as_mut() else {
            return;
        };

        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(SESSION_COOKIE_NAME))),
        );
    }
}

/// Build the generated OpenAPI document without starting the HTTP server.
///
/// # Returns
///
/// Complete OpenAPI document shared by the emitter and Swagger UI.
pub(crate) fn document() -> OpenApiDocument {
    ApiDoc::openapi()
}

/// Build the Swagger UI and OpenAPI JSON router.
///
/// # Returns
///
/// Router serving `/docs` and `/openapi.json`.
pub fn docs_router() -> Router<ApiState> {
    Router::from(SwaggerUi::new(DOCS_PATH).url(OPENAPI_JSON_PATH, document()))
}

#[cfg(test)]
mod tests {
    use utoipa::openapi::path::PathItem;

    use super::{document, ApiDoc, OpenApi};

    const EXPECTED_OPERATIONS: &[(&str, &str)] = &[
        ("/api/health", "get"),
        ("/api/health/worker", "get"),
        ("/api/announcements", "get"),
        ("/api/meta/databases", "get"),
        ("/api/meta/areas", "get"),
        ("/api/meta/journals", "get"),
        ("/api/meta/sources", "get"),
        ("/api/years", "get"),
        ("/api/journals", "get"),
        ("/api/journals/{journal_id}", "get"),
        ("/api/issues", "get"),
        ("/api/issues/{issue_id}", "get"),
        ("/api/weekly-updates", "get"),
        ("/api/articles", "get"),
        ("/api/articles/{article_id}", "get"),
        ("/api/articles/{article_id}/access", "get"),
        ("/api/articles/{article_id}/fulltext", "get"),
        ("/api/auth/register", "post"),
        ("/api/auth/login", "post"),
        ("/api/auth/me", "get"),
        ("/api/auth/change-password", "post"),
        ("/api/auth/logout", "post"),
        ("/api/auth/tokens", "post"),
        ("/api/auth/tokens", "get"),
        ("/api/auth/tokens/{token_id}", "delete"),
        ("/api/auth/invite-code", "post"),
        ("/api/auth/invite-code", "get"),
        ("/api/auth/invite-required", "get"),
        ("/api/cnki/session", "get"),
        ("/api/cnki/session", "delete"),
        ("/api/cnki/login/start", "post"),
        ("/api/cnki/login/poll", "post"),
        ("/api/favorites/folders", "get"),
        ("/api/favorites/folders", "post"),
        ("/api/favorites/folders/{folder_id}", "put"),
        ("/api/favorites/folders/{folder_id}", "delete"),
        ("/api/favorites/tracking", "get"),
        ("/api/favorites/tracking", "put"),
        ("/api/favorites/folders/{folder_id}/articles", "get"),
        ("/api/favorites/folders/{folder_id}/articles", "post"),
        ("/api/favorites/folders/{folder_id}/count", "get"),
        ("/api/favorites/folders/{folder_id}/export", "get"),
        (
            "/api/favorites/folders/{folder_id}/articles/{article_id}",
            "delete",
        ),
        ("/api/favorites/folders/{folder_id}/articles/bulk", "post"),
        (
            "/api/favorites/folders/{folder_id}/articles/bulk-remove",
            "post",
        ),
        (
            "/api/favorites/folders/{folder_id}/articles/bulk-move",
            "post",
        ),
        ("/api/favorites/check", "get"),
        ("/api/favorites/check/batch", "post"),
        ("/api/tracking/status", "get"),
        ("/api/tracking/push-weekly", "post"),
        ("/api/tracking/push-weekly/status", "get"),
        ("/api/tracking/notification-settings", "get"),
        ("/api/tracking/notification-settings", "put"),
        ("/api/admin/users", "get"),
        ("/api/admin/users/{user_id}/admin", "put"),
        ("/api/admin/users/{user_id}/reset-password", "post"),
        ("/api/admin/users/{user_id}", "delete"),
        ("/api/admin/invite-codes", "get"),
        ("/api/admin/invite-codes", "post"),
        ("/api/admin/invite-codes/{code_id}", "delete"),
        ("/api/admin/stats", "get"),
        ("/api/admin/scheduled-tasks", "get"),
        ("/api/admin/scheduled-tasks", "post"),
        ("/api/admin/scheduled-tasks/{task_id}", "put"),
        ("/api/admin/scheduled-tasks/{task_id}", "delete"),
        ("/api/admin/scheduler/status", "get"),
        ("/api/admin/runtime-settings", "get"),
        ("/api/admin/runtime-settings", "put"),
        ("/api/admin/announcements", "get"),
        ("/api/admin/announcements", "post"),
        ("/api/admin/announcements/{announcement_id}", "put"),
        ("/api/admin/announcements/{announcement_id}", "delete"),
    ];

    #[test]
    fn openapi_contains_every_public_route_operation() {
        let openapi = ApiDoc::openapi();

        for (path, method) in EXPECTED_OPERATIONS {
            let path_item = openapi
                .paths
                .paths
                .get(*path)
                .unwrap_or_else(|| panic!("missing OpenAPI path {path}"));
            assert!(
                has_operation(path_item, method),
                "missing OpenAPI operation {method} {path}"
            );
        }
    }

    #[test]
    fn openapi_documents_auth_security_schemes() {
        let openapi = ApiDoc::openapi();
        let components = openapi.components.expect("components should exist");
        let security_schemes = components.security_schemes;

        assert!(security_schemes.contains_key("bearer_auth"));
        assert!(security_schemes.contains_key("session_cookie"));
    }

    #[test]
    fn openapi_documents_auth_bootstrap_and_rate_limit_contract() {
        let document =
            serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");

        assert!(
            document["components"]["schemas"]["InviteRequiredResponse"]["properties"]
                ["bootstrap_required"]
                .is_object()
        );
        assert!(document["paths"]["/api/auth/login"]["post"]["responses"]["429"].is_object());
        assert!(document["paths"]["/api/auth/register"]["post"]["responses"]["429"].is_object());
    }

    #[test]
    fn openapi_document_generation_is_deterministic() {
        let first = serde_json::to_string_pretty(&document())
            .expect("first OpenAPI document should serialize");
        let second = serde_json::to_string_pretty(&document())
            .expect("second OpenAPI document should serialize");

        assert_eq!(first, second);
    }

    fn has_operation(path_item: &PathItem, method: &str) -> bool {
        match method {
            "get" => path_item.get.is_some(),
            "post" => path_item.post.is_some(),
            "put" => path_item.put.is_some(),
            "delete" => path_item.delete.is_some(),
            _ => false,
        }
    }
}
