//! Generated OpenAPI document and Swagger UI routing.

use axum::Router;
use litradar_auth::SESSION_COOKIE_NAME;
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
        title = "LitRadar API",
        version = "0.1.0",
        description = "Generated OpenAPI document for the Rust API."
    ),
    paths(
        crate::routes::health::live,
        crate::routes::health::ready,
        crate::routes::announcements::get_announcements,
        crate::routes::index::list_databases,
        crate::routes::index::list_areas,
        crate::routes::index::list_journal_options,
        crate::routes::index::list_years,
        crate::routes::index::list_journals,
        crate::routes::index::get_journal,
        crate::routes::index::list_issues,
        crate::routes::index::get_issue,
        crate::routes::index::get_weekly_updates,
        crate::routes::index::list_articles,
        crate::routes::index::get_article,
        crate::routes::index::get_article_access,
        crate::routes::index::redirect_article_abstract,
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
        litradar_domain::AdminInviteCodeInfo,
        litradar_domain::AdminResetPassword,
        litradar_domain::AdminSetAdmin,
        litradar_domain::AdminStatsResponse,
        litradar_domain::AdminUserInfo,
        litradar_domain::AnnouncementCreate,
        litradar_domain::AnnouncementInfo,
        litradar_domain::AnnouncementUpdate,
        litradar_domain::ArticleAccessAction,
        litradar_domain::ArticleAccessResponse,
        litradar_domain::ArticleId,
        litradar_domain::ArticlePage,
        litradar_domain::ArticleRecord,
        litradar_domain::AuthStats,
        litradar_domain::ChangePasswordRequest,
        litradar_domain::CnkiErrorDetail,
        litradar_domain::CnkiLoginPollRequest,
        litradar_domain::CnkiLoginPollResponse,
        litradar_domain::CnkiLoginStartResponse,
        litradar_domain::CnkiSessionStatusResponse,
        litradar_domain::ErrorEnvelope,
        litradar_domain::FavoriteAdd,
        litradar_domain::FavoriteArticleRef,
        litradar_domain::FavoriteArticleResponse,
        litradar_domain::FavoriteBatchCheckRequest,
        litradar_domain::FavoriteBatchCheckResponse,
        litradar_domain::FavoriteBulkAdd,
        litradar_domain::FavoriteBulkAddResult,
        litradar_domain::FavoriteBulkMove,
        litradar_domain::FavoriteBulkRemove,
        litradar_domain::FavoriteBulkResult,
        litradar_domain::FavoriteCheckResponse,
        litradar_domain::FavoriteResponse,
        litradar_domain::FavoriteTrackingResponse,
        litradar_domain::FolderCreate,
        litradar_domain::FolderRename,
        litradar_domain::FolderResponse,
        litradar_domain::HealthResponse,
        litradar_domain::IndexDatabaseStats,
        litradar_domain::IndexStats,
        litradar_domain::InviteCodeResponse,
        litradar_domain::InviteRequiredResponse,
        litradar_domain::IssuePage,
        litradar_domain::IssueRecord,
        litradar_domain::JournalId,
        litradar_domain::JournalOption,
        litradar_domain::JournalPage,
        litradar_domain::JournalRecord,
        litradar_domain::LoginRequest,
        litradar_domain::LoginResponse,
        litradar_domain::LogoutResponse,
        litradar_domain::ManualWeeklyPushStatus,
        litradar_domain::NotificationSettingsResponse,
        litradar_domain::NotificationSettingsUpdate,
        litradar_domain::OkResponse,
        litradar_domain::PageMeta,
        litradar_domain::PushStats,
        litradar_domain::RegisterRequest,
        litradar_domain::RuntimeSecretItemInfo,
        litradar_domain::RuntimeSecretPoolUpdate,
        litradar_domain::RuntimeSettingInfo,
        litradar_domain::RuntimeSettingsUpdate,
        litradar_domain::ScheduledDeliveryJob,
        litradar_domain::ScheduledIndexJob,
        litradar_domain::ScheduledJobSpec,
        litradar_domain::ScheduledTaskCreate,
        litradar_domain::ScheduledTaskInfo,
        litradar_domain::ScheduledTaskRunInfo,
        litradar_domain::ScheduledTaskUpdate,
        litradar_domain::SchedulerStatusResponse,
        litradar_domain::SchedulerWorkerInfo,
        litradar_domain::TokenCreateRequest,
        litradar_domain::TokenCreateResponse,
        litradar_domain::TokenInfo,
        litradar_domain::TrackingFolderSummary,
        litradar_domain::TrackingSetRequest,
        litradar_domain::TrackingStatusResponse,
        litradar_domain::UserId,
        litradar_domain::UserResponse,
        litradar_domain::ValueCount,
        litradar_domain::WeeklyArticleRecord,
        litradar_domain::WeeklyDatabaseUpdate,
        litradar_domain::WeeklyJournalUpdate,
        litradar_domain::WeeklyUpdatesResponse,
        litradar_domain::YearSummary
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
    use litradar_auth::{
        ACCESS_TOKEN_LIMIT_DETAIL, ACCESS_TOKEN_NAME_LENGTH_DETAIL,
        ACCESS_TOKEN_NAME_MAX_CODE_POINTS, ACCESS_TOKEN_RESERVED_NAME_DETAIL,
        ACCESS_TOKEN_TTL_DETAIL, ACCESS_TOKEN_TTL_MAX_SECONDS, ACCESS_TOKEN_TTL_MIN_SECONDS,
        ACCESS_TOKEN_VALIDATION_ORDER,
    };
    use utoipa::openapi::path::PathItem;

    use super::{document, ApiDoc, OpenApi};

    const EXPECTED_OPERATIONS: &[(&str, &str)] = &[
        ("/health/live", "get"),
        ("/health/ready", "get"),
        ("/api/announcements", "get"),
        ("/api/meta/databases", "get"),
        ("/api/meta/areas", "get"),
        ("/api/meta/journals", "get"),
        ("/api/years", "get"),
        ("/api/journals", "get"),
        ("/api/journals/{journal_id}", "get"),
        ("/api/issues", "get"),
        ("/api/issues/{issue_id}", "get"),
        ("/api/weekly-updates", "get"),
        ("/api/articles", "get"),
        ("/api/articles/{article_id}", "get"),
        ("/api/articles/{article_id}/access", "get"),
        ("/api/articles/{article_id}/abstract", "get"),
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
    fn openapi_documents_access_token_limits() {
        let document =
            serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");
        let request = &document["components"]["schemas"]["TokenCreateRequest"]["properties"];
        let responses = &document["paths"]["/api/auth/tokens"]["post"]["responses"];
        let bad_request_description = responses["400"]["description"]
            .as_str()
            .expect("400 response description should exist");
        let conflict_description = responses["409"]["description"]
            .as_str()
            .expect("409 response description should exist");

        assert_eq!(
            request["name"]["maxLength"],
            serde_json::json!(ACCESS_TOKEN_NAME_MAX_CODE_POINTS)
        );
        assert_eq!(
            request["ttl"]["minimum"],
            serde_json::json!(ACCESS_TOKEN_TTL_MIN_SECONDS)
        );
        assert_eq!(
            request["ttl"]["maximum"],
            serde_json::json!(ACCESS_TOKEN_TTL_MAX_SECONDS)
        );
        assert_eq!(
            responses["400"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ErrorEnvelope"
        );
        assert_eq!(
            responses["409"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ErrorEnvelope"
        );
        assert!(bad_request_description.contains(ACCESS_TOKEN_VALIDATION_ORDER));
        assert!(bad_request_description.contains(ACCESS_TOKEN_NAME_LENGTH_DETAIL));
        assert!(bad_request_description.contains(ACCESS_TOKEN_RESERVED_NAME_DETAIL));
        assert!(bad_request_description.contains(ACCESS_TOKEN_TTL_DETAIL));
        assert!(conflict_description.contains(ACCESS_TOKEN_VALIDATION_ORDER));
        assert!(conflict_description.contains(ACCESS_TOKEN_LIMIT_DETAIL));
        let detail_positions = [
            ACCESS_TOKEN_NAME_LENGTH_DETAIL,
            ACCESS_TOKEN_RESERVED_NAME_DETAIL,
            ACCESS_TOKEN_TTL_DETAIL,
        ]
        .map(|detail| {
            bad_request_description
                .find(detail)
                .expect("exact validation detail should be documented")
        });
        assert!(detail_positions
            .windows(2)
            .all(|positions| positions[0] < positions[1]));
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
