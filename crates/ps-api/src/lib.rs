//! Rust API server skeleton for backend migration compatibility.

pub mod config;
mod mcp;
mod observability;
mod openapi;
mod response;
pub mod routes;
pub mod state;
#[cfg(test)]
pub(crate) mod test_support;

use std::error::Error;

use axum::extract::Request;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, COOKIE};
use axum::http::HeaderValue;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;
use config::ApiConfig;
use ps_auth::SESSION_COOKIE_NAME;
use ps_storage::StorageConfig;
use state::ApiState;
use tokio::net::TcpListener;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

/// API route prefix preserved from the Python backend.
pub const API_PREFIX: &str = "/api";

/// Cache-Control header for credentialed requests.
pub const AUTHENTICATED_CACHE_CONTROL: &str = "private, no-store";

/// Cache-Control header for unauthenticated index reads.
pub const PUBLIC_INDEX_CACHE_CONTROL: &str = "public, max-age=300, stale-while-revalidate=600";

/// Start the API server from environment configuration.
///
/// # Returns
///
/// Result indicating whether the server exited cleanly.
pub async fn serve_from_env() -> Result<(), Box<dyn Error>> {
    serve(ApiConfig::from_env()?).await
}

/// Start the API server with an explicit runtime configuration.
///
/// # Arguments
///
/// * `config` - Runtime API configuration.
///
/// # Returns
///
/// Result indicating whether the server exited cleanly.
pub async fn serve(config: ApiConfig) -> Result<(), Box<dyn Error>> {
    observability::init_tracing();

    let bind_address = config.bind_address();
    let listener = TcpListener::bind(&bind_address).await?;
    println!("ps-api listening on {}", listener.local_addr()?);

    axum::serve(listener, build_router(config))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Build the Rust API router for the current migration phase.
///
/// # Arguments
///
/// * `config` - Runtime API configuration.
///
/// # Returns
///
/// Axum router with only the currently migrated public endpoints.
pub fn build_router(config: ApiConfig) -> Router {
    let storage_config = StorageConfig::from_project_root(config.project_root.clone());
    let state = ApiState::new(storage_config);

    Router::new()
        .nest_service("/mcp", mcp::service(&config, state.clone()))
        .merge(openapi::docs_router())
        .nest(API_PREFIX, routes::public_routes())
        .layer(from_fn(cache_control_middleware))
        .layer(cors_layer(&config))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path()
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state)
}

/// Build a CORS layer compatible with the existing Python configuration.
///
/// # Arguments
///
/// * `config` - Runtime API configuration.
///
/// # Returns
///
/// CORS middleware layer.
pub fn cors_layer(config: &ApiConfig) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_credentials(true)
        .allow_headers(AllowHeaders::mirror_request())
        .allow_methods(AllowMethods::mirror_request());

    if config.cors_allowed_origins.is_empty() {
        layer
    } else {
        let origins = config
            .cors_allowed_origins
            .iter()
            .map(|origin| {
                HeaderValue::from_str(origin)
                    .expect("CORS origins are validated during config load")
            })
            .collect::<Vec<_>>();
        layer.allow_origin(AllowOrigin::list(origins))
    }
}

async fn cache_control_middleware(request: Request, next: Next) -> Response {
    let has_auth_credentials = request.headers().contains_key(AUTHORIZATION)
        || request
            .headers()
            .get(COOKIE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(has_session_cookie);
    let is_public_index_path = is_public_index_cache_path(request.uri().path());
    let mut response = next.run(request).await;

    if has_auth_credentials {
        response.headers_mut().insert(
            CACHE_CONTROL,
            HeaderValue::from_static(AUTHENTICATED_CACHE_CONTROL),
        );
    } else if is_public_index_path {
        response.headers_mut().insert(
            CACHE_CONTROL,
            HeaderValue::from_static(PUBLIC_INDEX_CACHE_CONTROL),
        );
    }

    response
}

fn has_session_cookie(cookie_header: &str) -> bool {
    cookie_header
        .split(';')
        .map(str::trim)
        .any(|cookie| cookie.starts_with(&format!("{SESSION_COOKIE_NAME}=")))
}

fn is_public_index_cache_path(path: &str) -> bool {
    path.starts_with(&format!("{API_PREFIX}/articles"))
        || path.starts_with(&format!("{API_PREFIX}/meta"))
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to install Ctrl+C handler: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::header::{
        AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_TYPE, COOKIE, SET_COOKIE,
    };
    use axum::http::{Method, Request, StatusCode};
    use axum::Router;
    use rusqlite::Connection;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::test_support::{json_request, FixtureIndexDatabase, JsonTestResponse, TestBackend};

    static TEST_ENV_LOCK: AtomicBool = AtomicBool::new(false);

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn health_route_matches_python_payload() {
        let backend = TestBackend::new();
        let app = backend.router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: Value = serde_json::from_slice(&body).expect("body should be JSON");

        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload, serde_json::json!({"status": "ok"}));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn openapi_json_route_serves_generated_document() {
        let backend = TestBackend::new();
        let app = backend.router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/openapi.json")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: Value = serde_json::from_slice(&body).expect("body should be JSON");

        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["openapi"], "3.1.0");
        assert!(payload["paths"]["/api/health"].is_object());
        assert!(payload["paths"]["/api/admin/scheduled-tasks"].is_object());
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn docs_route_serves_swagger_ui_html() {
        let backend = TestBackend::new();
        let app = backend.router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/docs/")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let html = String::from_utf8(body.to_vec()).expect("body should be UTF-8");

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Swagger UI"));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn announcements_route_reads_existing_auth_database() {
        let backend = TestBackend::new();
        let connection = Connection::open(backend.auth_db_path()).expect("auth db should open");
        connection
            .execute_batch(
                "
                INSERT INTO announcements
                    (title, message, priority, enabled, created_at, updated_at)
                VALUES
                    ('Normal', 'normal message', 'normal', 1, 20.0, 21.0);
                ",
            )
            .expect("announcement fixture should be created");
        drop(connection);
        let app = backend.router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/announcements")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: Value = serde_json::from_slice(&body).expect("body should be JSON");

        assert_eq!(
            payload,
            serde_json::json!([
                {
                    "id": 1,
                    "title": "Normal",
                    "message": "normal message",
                    "priority": "normal",
                    "enabled": true,
                    "created_at": 20.0,
                    "updated_at": 21.0
                }
            ])
        );
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn backend_fixtures_create_authenticated_router_and_index_database() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("fixture_admin", true);
        let index_database = backend.create_index_database("fixture.sqlite");
        let app = backend.router();

        let bearer_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/me")
                    .header(AUTHORIZATION, user.authorization_header())
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let bearer_body = to_bytes(bearer_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let bearer_payload: Value =
            serde_json::from_slice(&bearer_body).expect("body should be JSON");

        let cookie_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/me")
                    .header(COOKIE, user.cookie_header())
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let cookie_status = cookie_response.status();

        let database_names = ps_storage::list_index_database_names(backend.storage_config())
            .expect("database names should load");
        let journal = ps_storage::get_journal(
            backend.storage_config(),
            Some(&index_database.db_name),
            index_database.journal_id,
        )
        .expect("journal should load");
        let issue = ps_storage::get_issue(
            backend.storage_config(),
            Some(&index_database.db_name),
            index_database.issue_id,
        )
        .expect("issue should load");
        let article = ps_storage::get_article(
            backend.storage_config(),
            Some(&index_database.db_name),
            index_database.article_id,
        )
        .expect("article should load");
        let articles = ps_storage::list_articles(
            backend.storage_config(),
            Some(&index_database.db_name),
            &ps_storage::ArticleListParams {
                q: Some("Fixture".to_string()),
                ..Default::default()
            },
        )
        .expect("articles should load from listing and search fixtures");

        assert_eq!(bearer_payload["id"], user.user_id().value());
        assert_eq!(bearer_payload["username"], "fixture_admin");
        assert_eq!(bearer_payload["is_admin"], true);
        assert_eq!(cookie_status, StatusCode::OK);
        assert_eq!(database_names.len(), 1);
        assert_eq!(database_names[0], index_database.db_name);
        assert!(index_database.path.exists());
        assert_eq!(journal.journal_id.value(), index_database.journal_id);
        assert_eq!(issue.issue_id, index_database.issue_id);
        assert_eq!(article.article_id.value(), index_database.article_id);
        assert_eq!(articles.items.len(), 1);
        assert_eq!(
            articles.items[0].article_id.value(),
            index_database.article_id
        );
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn auth_routes_cover_registration_login_tokens_invites_and_logout() {
        let backend = TestBackend::new();
        let app = backend.router();

        let invite_before = json_request(
            &app,
            Method::GET,
            "/api/auth/invite-required",
            None,
            None,
            None,
        )
        .await;
        let register = json_request(
            &app,
            Method::POST,
            "/api/auth/register",
            None,
            None,
            Some(serde_json::json!({
                "username": "alice",
                "password": "secret123",
                "invite_code": ""
            })),
        )
        .await;
        let invite_after = json_request(
            &app,
            Method::GET,
            "/api/auth/invite-required",
            None,
            None,
            None,
        )
        .await;
        let missing_invite = json_request(
            &app,
            Method::POST,
            "/api/auth/register",
            None,
            None,
            Some(serde_json::json!({
                "username": "bob",
                "password": "secret123",
                "invite_code": ""
            })),
        )
        .await;
        let login = json_request(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            None,
            Some(serde_json::json!({
                "username": "alice",
                "password": "secret123"
            })),
        )
        .await;
        let session_cookie = set_cookie_header(&login);
        let me_from_cookie = json_request(
            &app,
            Method::GET,
            "/api/auth/me",
            None,
            Some(&session_cookie),
            None,
        )
        .await;
        let created_token = json_request(
            &app,
            Method::POST,
            "/api/auth/tokens",
            None,
            Some(&session_cookie),
            Some(serde_json::json!({
                "name": "fixture-api",
                "ttl": 60
            })),
        )
        .await;
        let raw_token = created_token.payload["token"]
            .as_str()
            .expect("created token should include raw token");
        let bearer = format!("Bearer {raw_token}");
        let listed_tokens = json_request(
            &app,
            Method::GET,
            "/api/auth/tokens",
            Some(&bearer),
            None,
            None,
        )
        .await;
        let token_id = created_token.payload["id"]
            .as_i64()
            .expect("token id should be numeric");
        let deleted_token = json_request(
            &app,
            Method::DELETE,
            &format!("/api/auth/tokens/{token_id}"),
            None,
            Some(&session_cookie),
            None,
        )
        .await;
        let invite = json_request(
            &app,
            Method::POST,
            "/api/auth/invite-code",
            None,
            Some(&session_cookie),
            None,
        )
        .await;
        let invite_lookup = json_request(
            &app,
            Method::GET,
            "/api/auth/invite-code",
            None,
            Some(&session_cookie),
            None,
        )
        .await;
        let invited_user = json_request(
            &app,
            Method::POST,
            "/api/auth/register",
            None,
            None,
            Some(serde_json::json!({
                "username": "bob",
                "password": "secret123",
                "invite_code": invite.payload["code"].as_str().expect("invite code should exist")
            })),
        )
        .await;
        let bad_password_change = json_request(
            &app,
            Method::POST,
            "/api/auth/change-password",
            None,
            Some(&session_cookie),
            Some(serde_json::json!({
                "old_password": "wrong-password",
                "new_password": "secret456"
            })),
        )
        .await;
        let password_change = json_request(
            &app,
            Method::POST,
            "/api/auth/change-password",
            None,
            Some(&session_cookie),
            Some(serde_json::json!({
                "old_password": "secret123",
                "new_password": "secret456"
            })),
        )
        .await;
        let revoked_cookie = json_request(
            &app,
            Method::GET,
            "/api/auth/me",
            None,
            Some(&session_cookie),
            None,
        )
        .await;
        let new_login = json_request(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            None,
            Some(serde_json::json!({
                "username": "alice",
                "password": "secret456"
            })),
        )
        .await;
        let new_session_cookie = set_cookie_header(&new_login);
        let logout = json_request(
            &app,
            Method::POST,
            "/api/auth/logout",
            None,
            Some(&new_session_cookie),
            None,
        )
        .await;
        let logged_out = json_request(
            &app,
            Method::GET,
            "/api/auth/me",
            None,
            Some(&new_session_cookie),
            None,
        )
        .await;

        assert_eq!(invite_before.status, StatusCode::OK);
        assert_eq!(invite_before.payload["required"], false);
        assert_eq!(register.status, StatusCode::OK);
        assert_eq!(register.payload["username"], "alice");
        assert_eq!(register.payload["is_admin"], true);
        assert_eq!(invite_after.payload["required"], true);
        assert_eq!(missing_invite.status, StatusCode::BAD_REQUEST);
        assert_eq!(missing_invite.payload["detail"], "Invite code is required");
        assert_eq!(login.status, StatusCode::OK);
        assert!(login.payload.get("token").is_none());
        assert!(session_cookie.starts_with("ps_session="));
        assert_eq!(me_from_cookie.status, StatusCode::OK);
        assert_eq!(me_from_cookie.payload["username"], "alice");
        assert_eq!(created_token.status, StatusCode::OK);
        assert_eq!(created_token.payload["name"], "fixture-api");
        assert_eq!(listed_tokens.status, StatusCode::OK);
        assert_eq!(listed_tokens.payload[0]["name"], "fixture-api");
        assert!(listed_tokens.payload[0].get("token").is_none());
        assert_eq!(deleted_token.status, StatusCode::OK);
        assert_eq!(deleted_token.payload["ok"], true);
        assert_eq!(invite.status, StatusCode::OK);
        assert_eq!(invite_lookup.payload["id"], invite.payload["id"]);
        assert_eq!(invited_user.status, StatusCode::OK);
        assert_eq!(invited_user.payload["username"], "bob");
        assert_eq!(invited_user.payload["is_admin"], false);
        assert_eq!(bad_password_change.status, StatusCode::BAD_REQUEST);
        assert_eq!(password_change.status, StatusCode::OK);
        assert_eq!(password_change.payload["ok"], true);
        assert_eq!(revoked_cookie.status, StatusCode::UNAUTHORIZED);
        assert_eq!(new_login.status, StatusCode::OK);
        assert_eq!(logout.status, StatusCode::OK);
        assert_eq!(logout.payload["ok"], true);
        assert!(set_cookie_header(&logout).contains("Max-Age=0"));
        assert_eq!(logged_out.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn admin_routes_cover_user_invite_stats_settings_tasks_and_announcements() {
        let backend = TestBackend::new();
        let admin = backend.authenticated_user("admin", true);
        let member = backend.authenticated_user("member", false);
        let index_database = backend.create_index_database("fixture.sqlite");
        let app = backend.router();
        let admin_auth = admin.authorization_header();
        let member_auth = member.authorization_header();

        let forbidden_users = json_request(
            &app,
            Method::GET,
            "/api/admin/users",
            Some(&member_auth),
            None,
            None,
        )
        .await;
        let users = json_request(
            &app,
            Method::GET,
            "/api/admin/users",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let promoted = json_request(
            &app,
            Method::PUT,
            &format!("/api/admin/users/{}/admin", member.user_id().value()),
            Some(&admin_auth),
            None,
            Some(serde_json::json!({ "is_admin": true })),
        )
        .await;
        let self_revoke = json_request(
            &app,
            Method::PUT,
            &format!("/api/admin/users/{}/admin", admin.user_id().value()),
            Some(&admin_auth),
            None,
            Some(serde_json::json!({ "is_admin": false })),
        )
        .await;
        let reset_password = json_request(
            &app,
            Method::POST,
            &format!(
                "/api/admin/users/{}/reset-password",
                member.user_id().value()
            ),
            Some(&admin_auth),
            None,
            Some(serde_json::json!({ "new_password": "reset123" })),
        )
        .await;
        let reset_login = json_request(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            None,
            Some(serde_json::json!({
                "username": "member",
                "password": "reset123"
            })),
        )
        .await;
        let invite = json_request(
            &app,
            Method::POST,
            "/api/admin/invite-codes",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let invite_id = invite.payload["id"]
            .as_i64()
            .expect("invite id should be numeric");
        let invite_codes = json_request(
            &app,
            Method::GET,
            "/api/admin/invite-codes",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let deleted_invite = json_request(
            &app,
            Method::DELETE,
            &format!("/api/admin/invite-codes/{invite_id}"),
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let missing_invite = json_request(
            &app,
            Method::DELETE,
            &format!("/api/admin/invite-codes/{invite_id}"),
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let runtime_settings = json_request(
            &app,
            Method::GET,
            "/api/admin/runtime-settings",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let runtime_update = json_request(
            &app,
            Method::PUT,
            "/api/admin/runtime-settings",
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "values": {
                    "openalex_api_key_pool": " key-one "
                }
            })),
        )
        .await;
        let runtime_error = json_request(
            &app,
            Method::PUT,
            "/api/admin/runtime-settings",
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "values": {
                    "unknown_setting": "value"
                }
            })),
        )
        .await;
        let task = json_request(
            &app,
            Method::POST,
            "/api/admin/scheduled-tasks",
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "name": "Nightly index",
                "command": "ps-cli index",
                "cron": "0 1 * * *",
                "enabled": true
            })),
        )
        .await;
        let task_id = task.payload["id"]
            .as_i64()
            .expect("task id should be numeric");
        let task_list = json_request(
            &app,
            Method::GET,
            "/api/admin/scheduled-tasks",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let task_error = json_request(
            &app,
            Method::POST,
            "/api/admin/scheduled-tasks",
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "name": "Bad cron",
                "command": "ps-cli index",
                "cron": "* * *",
                "enabled": true
            })),
        )
        .await;
        let task_update = json_request(
            &app,
            Method::PUT,
            &format!("/api/admin/scheduled-tasks/{task_id}"),
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "name": "Nightly index updated",
                "enabled": false
            })),
        )
        .await;
        let task_delete = json_request(
            &app,
            Method::DELETE,
            &format!("/api/admin/scheduled-tasks/{task_id}"),
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let announcement = json_request(
            &app,
            Method::POST,
            "/api/admin/announcements",
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "title": "Maintenance",
                "message": "Window tonight",
                "priority": "High",
                "enabled": true
            })),
        )
        .await;
        let announcement_id = announcement.payload["id"]
            .as_i64()
            .expect("announcement id should be numeric");
        let announcement_list = json_request(
            &app,
            Method::GET,
            "/api/admin/announcements",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let announcement_error = json_request(
            &app,
            Method::POST,
            "/api/admin/announcements",
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "title": "",
                "message": "Message",
                "priority": "urgent",
                "enabled": true
            })),
        )
        .await;
        let announcement_update = json_request(
            &app,
            Method::PUT,
            &format!("/api/admin/announcements/{announcement_id}"),
            Some(&admin_auth),
            None,
            Some(serde_json::json!({
                "message": "Updated window",
                "enabled": false
            })),
        )
        .await;
        let stats = json_request(
            &app,
            Method::GET,
            "/api/admin/stats",
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let announcement_delete = json_request(
            &app,
            Method::DELETE,
            &format!("/api/admin/announcements/{announcement_id}"),
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let deleted_user = json_request(
            &app,
            Method::DELETE,
            &format!("/api/admin/users/{}", member.user_id().value()),
            Some(&admin_auth),
            None,
            None,
        )
        .await;
        let self_delete = json_request(
            &app,
            Method::DELETE,
            &format!("/api/admin/users/{}", admin.user_id().value()),
            Some(&admin_auth),
            None,
            None,
        )
        .await;

        assert_eq!(forbidden_users.status, StatusCode::FORBIDDEN);
        assert_eq!(users.status, StatusCode::OK);
        assert_eq!(
            users
                .payload
                .as_array()
                .expect("users should be array")
                .len(),
            2
        );
        assert_eq!(promoted.status, StatusCode::OK);
        assert_eq!(promoted.payload["ok"], true);
        assert_eq!(self_revoke.status, StatusCode::BAD_REQUEST);
        assert_eq!(reset_password.status, StatusCode::OK);
        assert_eq!(reset_login.status, StatusCode::OK);
        assert_eq!(invite.status, StatusCode::OK);
        assert!(
            invite.payload["code"]
                .as_str()
                .expect("invite code should be a string")
                .len()
                >= 16
        );
        assert_eq!(invite_codes.status, StatusCode::OK);
        assert_eq!(invite_codes.payload[0]["id"], invite.payload["id"]);
        assert_eq!(deleted_invite.status, StatusCode::OK);
        assert_eq!(missing_invite.status, StatusCode::NOT_FOUND);
        assert_eq!(runtime_settings.status, StatusCode::OK);
        assert!(runtime_settings
            .payload
            .as_array()
            .expect("runtime settings should be array")
            .iter()
            .any(|setting| setting["field"] == "openalex_api_key_pool"));
        assert_eq!(runtime_update.status, StatusCode::OK);
        assert!(runtime_update
            .payload
            .as_array()
            .expect("runtime settings should be array")
            .iter()
            .any(|setting| {
                setting["field"] == "openalex_api_key_pool"
                    && setting["value"] == "key-one"
                    && setting["source"] == "database"
            }));
        assert_eq!(runtime_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(task.status, StatusCode::OK);
        assert_eq!(task.payload["name"], "Nightly index");
        assert_eq!(task_list.status, StatusCode::OK);
        assert_eq!(task_list.payload[0]["id"], task.payload["id"]);
        assert_eq!(task_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(task_update.status, StatusCode::OK);
        assert_eq!(task_update.payload["enabled"], false);
        assert_eq!(task_delete.status, StatusCode::OK);
        assert_eq!(announcement.status, StatusCode::OK);
        assert_eq!(announcement.payload["priority"], "high");
        assert_eq!(announcement_list.status, StatusCode::OK);
        assert_eq!(
            announcement_list.payload[0]["id"],
            announcement.payload["id"]
        );
        assert_eq!(announcement_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(announcement_update.status, StatusCode::OK);
        assert_eq!(announcement_update.payload["message"], "Updated window");
        assert_eq!(stats.status, StatusCode::OK);
        assert_eq!(stats.payload["auth"]["total_users"], 2);
        assert_eq!(stats.payload["index"]["total_articles"], 1);
        assert_eq!(
            stats.payload["index"]["databases"][0]["db_name"],
            index_database.db_name
        );
        assert_eq!(announcement_delete.status, StatusCode::OK);
        assert_eq!(deleted_user.status, StatusCode::OK);
        assert_eq!(self_delete.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn favorites_routes_cover_folder_article_batch_export_and_tracking_flows() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("reader", false);
        let index_database = backend.create_index_database("fixture.sqlite");
        let app = backend.router();
        let auth = user.authorization_header();

        let initial_folders = json_request(
            &app,
            Method::GET,
            "/api/favorites/folders",
            Some(&auth),
            None,
            None,
        )
        .await;
        let default_folder_id = initial_folders.payload[0]["id"]
            .as_i64()
            .expect("default folder id should be numeric");
        let created_folder = json_request(
            &app,
            Method::POST,
            "/api/favorites/folders",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "name": "Research",
                "is_tracking": false
            })),
        )
        .await;
        let folder_id = created_folder.payload["id"]
            .as_i64()
            .expect("folder id should be numeric");
        let duplicate_folder = json_request(
            &app,
            Method::POST,
            "/api/favorites/folders",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "name": "Research",
                "is_tracking": false
            })),
        )
        .await;
        let renamed_folder = json_request(
            &app,
            Method::PUT,
            &format!("/api/favorites/folders/{folder_id}"),
            Some(&auth),
            None,
            Some(serde_json::json!({ "name": "Research 2024" })),
        )
        .await;
        let tracking_folder = json_request(
            &app,
            Method::POST,
            "/api/favorites/folders",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "name": "Tracking",
                "is_tracking": true
            })),
        )
        .await;
        let tracking_folder_id = tracking_folder.payload["id"]
            .as_i64()
            .expect("tracking folder id should be numeric");
        let selected_tracking = json_request(
            &app,
            Method::PUT,
            "/api/favorites/tracking",
            Some(&auth),
            None,
            Some(serde_json::json!({ "folder_id": tracking_folder_id })),
        )
        .await;
        let missing_tracking = json_request(
            &app,
            Method::PUT,
            "/api/favorites/tracking",
            Some(&auth),
            None,
            Some(serde_json::json!({ "folder_id": 999999 })),
        )
        .await;
        let current_tracking = json_request(
            &app,
            Method::GET,
            "/api/favorites/tracking",
            Some(&auth),
            None,
            None,
        )
        .await;
        let favorite = json_request(
            &app,
            Method::POST,
            &format!("/api/favorites/folders/{folder_id}/articles"),
            Some(&auth),
            None,
            Some(serde_json::json!({
                "article_id": index_database.article_id,
                "db_name": index_database.db_name,
                "note": "Read later"
            })),
        )
        .await;
        let articles = json_request(
            &app,
            Method::GET,
            &format!("/api/favorites/folders/{folder_id}/articles?limit=10&offset=0"),
            Some(&auth),
            None,
            None,
        )
        .await;
        let invalid_articles = json_request(
            &app,
            Method::GET,
            &format!("/api/favorites/folders/{folder_id}/articles?limit=0"),
            Some(&auth),
            None,
            None,
        )
        .await;
        let count = json_request(
            &app,
            Method::GET,
            &format!("/api/favorites/folders/{folder_id}/count"),
            Some(&auth),
            None,
            None,
        )
        .await;
        let export_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/favorites/folders/{folder_id}/export?format=ris"
                    ))
                    .header(AUTHORIZATION, &auth)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let export_status = export_response.status();
        let export_headers = export_response.headers().clone();
        let export_body = to_bytes(export_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let export_text = String::from_utf8(export_body.to_vec()).expect("export should be UTF-8");
        let check = json_request(
            &app,
            Method::GET,
            &format!(
                "/api/favorites/check?article_id={}&db_name={}",
                index_database.article_id, index_database.db_name
            ),
            Some(&auth),
            None,
            None,
        )
        .await;
        let batch_check = json_request(
            &app,
            Method::POST,
            "/api/favorites/check/batch",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "article_ids": [
                    index_database.article_id,
                    index_database.article_id,
                    0
                ],
                "db_name": index_database.db_name
            })),
        )
        .await;
        let bulk_add = json_request(
            &app,
            Method::POST,
            &format!("/api/favorites/folders/{default_folder_id}/articles/bulk"),
            Some(&auth),
            None,
            Some(serde_json::json!({
                "articles": [
                    {
                        "article_id": index_database.article_id,
                        "db_name": index_database.db_name,
                        "note": "Tracking copy"
                    }
                ]
            })),
        )
        .await;
        let bulk_move = json_request(
            &app,
            Method::POST,
            &format!("/api/favorites/folders/{default_folder_id}/articles/bulk-move"),
            Some(&auth),
            None,
            Some(serde_json::json!({
                "target_folder_id": tracking_folder_id,
                "articles": [
                    {
                        "article_id": index_database.article_id,
                        "db_name": index_database.db_name
                    }
                ]
            })),
        )
        .await;
        let bulk_move_same = json_request(
            &app,
            Method::POST,
            &format!("/api/favorites/folders/{tracking_folder_id}/articles/bulk-move"),
            Some(&auth),
            None,
            Some(serde_json::json!({
                "target_folder_id": tracking_folder_id,
                "articles": [
                    {
                        "article_id": index_database.article_id,
                        "db_name": index_database.db_name
                    }
                ]
            })),
        )
        .await;
        let bulk_remove = json_request(
            &app,
            Method::POST,
            &format!("/api/favorites/folders/{tracking_folder_id}/articles/bulk-remove"),
            Some(&auth),
            None,
            Some(serde_json::json!({
                "articles": [
                    {
                        "article_id": index_database.article_id,
                        "db_name": index_database.db_name
                    }
                ]
            })),
        )
        .await;
        let removed = json_request(
            &app,
            Method::DELETE,
            &format!(
                "/api/favorites/folders/{folder_id}/articles/{}?db_name={}",
                index_database.article_id, index_database.db_name
            ),
            Some(&auth),
            None,
            None,
        )
        .await;
        let missing_favorite = json_request(
            &app,
            Method::DELETE,
            &format!(
                "/api/favorites/folders/{folder_id}/articles/{}?db_name={}",
                index_database.article_id, index_database.db_name
            ),
            Some(&auth),
            None,
            None,
        )
        .await;
        let deleted_folder = json_request(
            &app,
            Method::DELETE,
            &format!("/api/favorites/folders/{folder_id}"),
            Some(&auth),
            None,
            None,
        )
        .await;

        assert_eq!(initial_folders.status, StatusCode::OK);
        assert!(initial_folders.payload[0]["is_tracking"]
            .as_bool()
            .expect("tracking flag should be boolean"));
        assert_eq!(created_folder.status, StatusCode::OK);
        assert_eq!(created_folder.payload["name"], "Research");
        assert_eq!(duplicate_folder.status, StatusCode::CONFLICT);
        assert_eq!(renamed_folder.status, StatusCode::OK);
        assert_eq!(tracking_folder.status, StatusCode::OK);
        assert_eq!(selected_tracking.status, StatusCode::OK);
        assert_eq!(missing_tracking.status, StatusCode::NOT_FOUND);
        assert_eq!(current_tracking.payload["folder_id"], tracking_folder_id);
        assert_eq!(favorite.status, StatusCode::OK);
        assert_eq!(favorite.payload["note"], "Read later");
        assert_eq!(articles.status, StatusCode::OK);
        assert_eq!(articles.payload[0]["title"], "Fixture Article");
        assert_eq!(articles.payload[0]["journal_title"], "Fixture Journal");
        assert_eq!(invalid_articles.status, StatusCode::BAD_REQUEST);
        assert_eq!(count.payload["count"], 1);
        assert_eq!(export_status, StatusCode::OK);
        assert_eq!(
            export_headers
                .get("content-type")
                .expect("content type should exist"),
            "application/x-research-info-systems"
        );
        assert!(export_headers
            .get("content-disposition")
            .expect("content disposition should exist")
            .to_str()
            .expect("header should be visible ASCII")
            .contains("Research_2024.ris"));
        assert!(export_text.contains("Fixture Article"));
        assert_eq!(check.status, StatusCode::OK);
        assert_eq!(check.payload[0]["folder_id"], folder_id);
        assert_eq!(batch_check.status, StatusCode::OK);
        assert_eq!(
            batch_check
                .payload
                .as_array()
                .expect("batch should be array")
                .len(),
            1
        );
        assert_eq!(batch_check.payload[0]["folders"][0]["folder_id"], folder_id);
        assert_eq!(bulk_add.status, StatusCode::OK);
        assert_eq!(bulk_add.payload["added"], 1);
        assert_eq!(bulk_move.status, StatusCode::OK);
        assert_eq!(bulk_move.payload["count"], 1);
        assert_eq!(bulk_move_same.status, StatusCode::BAD_REQUEST);
        assert_eq!(bulk_remove.status, StatusCode::OK);
        assert_eq!(bulk_remove.payload["count"], 1);
        assert_eq!(removed.status, StatusCode::OK);
        assert_eq!(missing_favorite.status, StatusCode::NOT_FOUND);
        assert_eq!(deleted_folder.status, StatusCode::OK);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn tracking_routes_cover_status_and_notification_settings_validation() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("tracker", false);
        let index_database = backend.create_index_database("fixture.sqlite");
        let push_state_dir = backend.project_root().join("data").join("push_state");
        std::fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
        std::fs::write(
            push_state_dir.join("fixture.changes.json"),
            serde_json::json!({
                "db_name": index_database.db_name,
                "notifiable_article_ids": [index_database.article_id],
                "backfill_article_ids": [9002]
            })
            .to_string(),
        )
        .expect("push state fixture should be written");
        let app = backend.router();
        let auth = user.authorization_header();

        let initial_status = json_request(
            &app,
            Method::GET,
            "/api/tracking/status",
            Some(&auth),
            None,
            None,
        )
        .await;
        let empty_settings = json_request(
            &app,
            Method::GET,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            None,
        )
        .await;
        let invalid_delivery = json_request(
            &app,
            Method::PUT,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "delivery_method": "email"
            })),
        )
        .await;
        let missing_pushplus_token = json_request(
            &app,
            Method::PUT,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "delivery_method": "pushplus",
                "pushplus_token": ""
            })),
        )
        .await;
        let unknown_database = json_request(
            &app,
            Method::PUT,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "selected_databases": ["missing.sqlite"],
                "delivery_method": "folder"
            })),
        )
        .await;
        let folder_settings = json_request(
            &app,
            Method::PUT,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "keywords": [" ai ", "", "medicine"],
                "directions": [" screening "],
                "selected_databases": [index_database.db_name],
                "delivery_method": "folder",
                "enabled": true
            })),
        )
        .await;
        let stored_settings = json_request(
            &app,
            Method::GET,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            None,
        )
        .await;
        let configured_status = json_request(
            &app,
            Method::GET,
            "/api/tracking/status",
            Some(&auth),
            None,
            None,
        )
        .await;
        let pushplus_settings = json_request(
            &app,
            Method::PUT,
            "/api/tracking/notification-settings",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "keywords": ["ai"],
                "directions": ["screening"],
                "delivery_method": "pushplus",
                "pushplus_token": "token-1",
                "pushplus_template": "",
                "pushplus_channel": "wechat",
                "sync_to_tracking_folder": true,
                "enabled": true
            })),
        )
        .await;
        let subscribers = ps_storage::list_notification_subscribers(backend.auth_db_path())
            .expect("subscribers should load");

        assert_eq!(initial_status.status, StatusCode::OK);
        assert_eq!(initial_status.payload["total_folders"], 1);
        assert_eq!(initial_status.payload["weekly_articles_available"], 2);
        assert_eq!(initial_status.payload["notification_configured"], false);
        assert_eq!(empty_settings.status, StatusCode::OK);
        assert!(empty_settings.payload.is_null());
        assert_eq!(invalid_delivery.status, StatusCode::BAD_REQUEST);
        assert_eq!(missing_pushplus_token.status, StatusCode::BAD_REQUEST);
        assert_eq!(unknown_database.status, StatusCode::BAD_REQUEST);
        assert_eq!(folder_settings.status, StatusCode::OK);
        assert_eq!(
            folder_settings.payload["keywords"],
            serde_json::json!(["ai", "medicine"])
        );
        assert_eq!(
            folder_settings.payload["directions"],
            serde_json::json!(["screening"])
        );
        assert_eq!(
            folder_settings.payload["selected_databases"],
            serde_json::json!([])
        );
        assert_eq!(stored_settings.status, StatusCode::OK);
        assert_eq!(stored_settings.payload["delivery_method"], "folder");
        assert_eq!(configured_status.status, StatusCode::OK);
        assert_eq!(configured_status.payload["notification_configured"], true);
        assert_eq!(pushplus_settings.status, StatusCode::OK);
        assert_eq!(pushplus_settings.payload["delivery_method"], "pushplus");
        assert_eq!(pushplus_settings.payload["pushplus_template"], "markdown");
        assert_eq!(subscribers.len(), 1);
        assert_eq!(subscribers[0].user_id, user.user_id().value());
        assert_eq!(subscribers[0].tracking_folder_id, Some(1));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn tracking_manual_weekly_push_routes_cover_status_and_duplicate_start() {
        let _env_guard =
            EnvVarGuard::configured(&[("PAPER_SCANNER_MANUAL_PUSH_TEST_DELAY_MS", Some("120"))]);
        let backend = TestBackend::new();
        let user = backend.authenticated_user("manual_push", false);
        let app = backend.router();
        let auth = user.authorization_header();

        let idle = json_request(
            &app,
            Method::GET,
            "/api/tracking/push-weekly/status",
            Some(&auth),
            None,
            None,
        )
        .await;
        let started = json_request(
            &app,
            Method::POST,
            "/api/tracking/push-weekly",
            Some(&auth),
            None,
            None,
        )
        .await;
        let duplicate = json_request(
            &app,
            Method::POST,
            "/api/tracking/push-weekly",
            Some(&auth),
            None,
            None,
        )
        .await;
        let running = json_request(
            &app,
            Method::GET,
            "/api/tracking/push-weekly/status",
            Some(&auth),
            None,
            None,
        )
        .await;
        let finished = wait_for_manual_push_completion(&app, &auth).await;

        assert_eq!(idle.status, StatusCode::OK);
        assert_eq!(idle.payload["status"], "idle");
        assert_eq!(idle.payload["message"], "No manual push task is running");
        assert!(idle.payload["job_id"].is_null());
        assert_eq!(started.status, StatusCode::OK);
        assert_eq!(started.payload["status"], "running");
        assert!(started.payload["job_id"].is_string());
        assert!(started.payload["started_at"].is_number());
        assert!(started.payload["finished_at"].is_null());
        assert_eq!(started.payload["pushed"], 0);
        assert_eq!(started.payload["selected"], 0);
        assert!(started.payload["total_candidates"].is_null());
        assert_eq!(duplicate.status, StatusCode::OK);
        assert_eq!(duplicate.payload["status"], "running");
        assert_eq!(duplicate.payload["job_id"], started.payload["job_id"]);
        assert_eq!(running.status, StatusCode::OK);
        assert_eq!(running.payload["job_id"], started.payload["job_id"]);
        assert_eq!(finished.status, StatusCode::OK);
        assert_eq!(finished.payload["status"], "completed");
        assert_eq!(finished.payload["job_id"], started.payload["job_id"]);
        assert_eq!(
            finished.payload["message"],
            "Recommendation settings are not enabled; skipped push"
        );
        assert!(finished.payload["finished_at"].is_number());
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn index_routes_cover_fixture_database_queries_and_resolution_errors() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("index_reader", false);
        let app = backend.router();
        let auth = user.authorization_header();

        let no_database =
            json_request(&app, Method::GET, "/api/years", Some(&auth), None, None).await;
        let index_database = backend.create_index_database("fixture.sqlite");
        let databases = json_request(
            &app,
            Method::GET,
            "/api/meta/databases",
            Some(&auth),
            None,
            None,
        )
        .await;
        let years = json_request(&app, Method::GET, "/api/years", Some(&auth), None, None).await;
        let areas = json_request(
            &app,
            Method::GET,
            "/api/meta/areas?db=fixture",
            Some(&auth),
            None,
            None,
        )
        .await;
        let journal_options = json_request(
            &app,
            Method::GET,
            "/api/meta/journals?db=fixture.sqlite",
            Some(&auth),
            None,
            None,
        )
        .await;
        let sources = json_request(
            &app,
            Method::GET,
            "/api/meta/sources?db=fixture",
            Some(&auth),
            None,
            None,
        )
        .await;
        let journals = json_request(
            &app,
            Method::GET,
            "/api/journals?db=fixture&area=Medicine&limit=1&offset=0",
            Some(&auth),
            None,
            None,
        )
        .await;
        let journal = json_request(
            &app,
            Method::GET,
            &format!("/api/journals/{}?db=fixture", index_database.journal_id),
            Some(&auth),
            None,
            None,
        )
        .await;
        let issues = json_request(
            &app,
            Method::GET,
            &format!(
                "/api/issues?db=fixture&journal_id={}&year=2024&limit=1",
                index_database.journal_id
            ),
            Some(&auth),
            None,
            None,
        )
        .await;
        let issue = json_request(
            &app,
            Method::GET,
            &format!("/api/issues/{}?db=fixture", index_database.issue_id),
            Some(&auth),
            None,
            None,
        )
        .await;
        let articles = json_request(
            &app,
            Method::GET,
            &format!(
                "/api/articles?db=fixture&journal_id={}&journal_id=999&area=Medicine&q=Fixture&limit=1&include_total=true",
                index_database.journal_id
            ),
            Some(&auth),
            None,
            None,
        )
        .await;
        let invalid_articles = json_request(
            &app,
            Method::GET,
            "/api/articles?db=fixture&limit=0",
            Some(&auth),
            None,
            None,
        )
        .await;
        let article = json_request(
            &app,
            Method::GET,
            &format!("/api/articles/{}?db=fixture", index_database.article_id),
            Some(&auth),
            None,
            None,
        )
        .await;
        let missing_article = json_request(
            &app,
            Method::GET,
            "/api/articles/404?db=fixture",
            Some(&auth),
            None,
            None,
        )
        .await;
        let access = json_request(
            &app,
            Method::GET,
            &format!(
                "/api/articles/{}/access?db=fixture",
                index_database.article_id
            ),
            Some(&auth),
            None,
            None,
        )
        .await;
        let fulltext_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/articles/{}/fulltext?db=fixture",
                        index_database.article_id
                    ))
                    .header(AUTHORIZATION, &auth)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let fulltext_status = fulltext_response.status();
        let fulltext_location = fulltext_response
            .headers()
            .get("location")
            .expect("location should exist")
            .to_str()
            .expect("location should be visible ASCII")
            .to_string();
        backend.create_index_database("second.sqlite");
        let ambiguous =
            json_request(&app, Method::GET, "/api/years", Some(&auth), None, None).await;
        let missing_database = json_request(
            &app,
            Method::GET,
            "/api/years?db=missing",
            Some(&auth),
            None,
            None,
        )
        .await;

        assert_eq!(no_database.status, StatusCode::NOT_FOUND);
        assert_eq!(databases.status, StatusCode::OK);
        assert_eq!(databases.payload, serde_json::json!(["fixture.sqlite"]));
        assert_eq!(years.status, StatusCode::OK);
        assert_eq!(years.payload[0]["year"], 2024);
        assert_eq!(areas.payload[0]["value"], "Medicine");
        assert_eq!(journal_options.payload[0]["title"], "Fixture Journal");
        assert_eq!(sources.payload[0]["value"], "Library A");
        assert_eq!(journals.status, StatusCode::OK);
        assert_eq!(journals.payload["items"][0]["title"], "Fixture Journal");
        assert_eq!(journal.status, StatusCode::OK);
        assert_eq!(
            journal.payload["journal_id"],
            index_database.journal_id.to_string()
        );
        assert_eq!(issues.status, StatusCode::OK);
        assert_eq!(
            issues.payload["items"][0]["issue_id"],
            index_database.issue_id
        );
        assert_eq!(issue.status, StatusCode::OK);
        assert_eq!(issue.payload["title"], "Volume 1 Issue 1");
        assert_eq!(articles.status, StatusCode::OK);
        assert_eq!(articles.payload["page"]["total"], 1);
        assert_eq!(
            articles.payload["items"][0]["article_id"],
            index_database.article_id.to_string()
        );
        assert_eq!(invalid_articles.status, StatusCode::BAD_REQUEST);
        assert_eq!(article.status, StatusCode::OK);
        assert_eq!(article.payload["doi"], "10.1234/fixture");
        assert_eq!(missing_article.status, StatusCode::NOT_FOUND);
        assert_eq!(access.status, StatusCode::OK);
        assert_eq!(access.payload["detail"]["available"], true);
        assert_eq!(access.payload["fulltext"]["provider"], "stored_url");
        assert_eq!(fulltext_status, StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(fulltext_location, "https://example.test/fulltext.pdf");
        assert_eq!(ambiguous.status, StatusCode::BAD_REQUEST);
        assert_eq!(missing_database.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn article_fulltext_downloads_cnki_pdf_with_live_fixture_session() {
        const LIVE_FIXTURE_MODE: &str = "PAPER_SCANNER_ZJLIB_CNKI_FIXTURE_MODE";
        const CNKI_PDF_REPLAY_MODE: &str = "PAPER_SCANNER_CNKI_PDF_REPLAY_MODE";
        const CNKI_PDF_REPLAY_PATH: &str = "PAPER_SCANNER_CNKI_PDF_REPLAY_PATH";
        const CNKI_PDF_REPLAY_FILENAME: &str = "PAPER_SCANNER_CNKI_PDF_REPLAY_FILENAME";

        let _env_guard = EnvVarGuard::configured(&[
            (LIVE_FIXTURE_MODE, Some("success")),
            (CNKI_PDF_REPLAY_MODE, None),
            (CNKI_PDF_REPLAY_PATH, None),
            (CNKI_PDF_REPLAY_FILENAME, None),
        ]);
        let backend = TestBackend::new();
        let user = backend.authenticated_user("cnki_fulltext_user", false);
        let index_database = backend.create_index_database("fixture.sqlite");
        let cnki_article_id = insert_cnki_fulltext_article(&index_database);
        ps_storage::upsert_cnki_session(
            backend.auth_db_path(),
            user.user_id(),
            &serde_json::json!({
                "bff_user_token": "x.eyJleHAiOjQxMDI0NDQ4MDB9.y",
                "qr_uuid": "qr-fulltext-fixture",
                "cookies": [
                    {"name": "userToken", "value": "SECRET_TOKEN_COOKIE"}
                ]
            }),
            "active",
            Some("qr-fulltext-fixture"),
        )
        .expect("CNKI session should upsert");
        let app = backend.router();
        let auth = user.authorization_header();

        let access = json_request(
            &app,
            Method::GET,
            &format!("/api/articles/{cnki_article_id}/access?db=fixture"),
            Some(&auth),
            None,
            None,
        )
        .await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/articles/{cnki_article_id}/fulltext?db=fixture"
                    ))
                    .header(AUTHORIZATION, &auth)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned");
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let session_status =
            ps_storage::get_cnki_session_status(backend.auth_db_path(), user.user_id())
                .expect("CNKI session status should load");

        assert_eq!(access.status, StatusCode::OK);
        assert_eq!(access.payload["fulltext"]["available"], true);
        assert_eq!(access.payload["fulltext"]["provider"], "zjlib_cnki");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get(CONTENT_TYPE)
                .expect("content-type should exist")
                .to_str()
                .expect("content-type should be ASCII"),
            "application/pdf"
        );
        assert!(headers
            .get(CONTENT_DISPOSITION)
            .expect("content disposition should exist")
            .to_str()
            .expect("content disposition should be ASCII")
            .contains("Fixture%20CNKI%20Article.pdf"));
        assert!(body.starts_with(b"%PDF"));
        assert!(session_status.last_used_at.is_some());

        std::env::set_var(LIVE_FIXTURE_MODE, "fulltext_mismatch");
        let mismatch = json_request(
            &app,
            Method::GET,
            &format!("/api/articles/{cnki_article_id}/fulltext?db=fixture"),
            Some(&auth),
            None,
            None,
        )
        .await;

        assert_eq!(mismatch.status, StatusCode::NOT_FOUND);
        assert!(mismatch.payload["detail"]
            .as_str()
            .expect("detail should be a string")
            .contains("No exact CNKI full-text match found"));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn cnki_routes_cover_replay_session_status_and_clear_without_secret_leaks() {
        const REPLAY_MODE: &str = "PAPER_SCANNER_CNKI_REPLAY_MODE";
        const LIVE_FIXTURE_MODE: &str = "PAPER_SCANNER_ZJLIB_CNKI_FIXTURE_MODE";

        let _env_guard = EnvVarGuard::configured(&[(REPLAY_MODE, None), (LIVE_FIXTURE_MODE, None)]);
        let backend = TestBackend::new();
        let user = backend.authenticated_user("cnki_user", false);
        let app = backend.router();
        let auth = user.authorization_header();

        let empty_session = json_request(
            &app,
            Method::GET,
            "/api/cnki/session",
            Some(&auth),
            None,
            None,
        )
        .await;
        let poll_before_start = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;
        let invalid_poll = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 0,
                "interval_seconds": 0.1
            })),
        )
        .await;
        std::env::set_var(REPLAY_MODE, "start_success");
        let waiting_start = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/start",
            Some(&auth),
            None,
            None,
        )
        .await;
        let timeout_poll = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;

        std::env::set_var(REPLAY_MODE, "warmup_failure");
        let warmup_poll = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;

        std::env::set_var(REPLAY_MODE, "poll_success");
        let success_start = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/start",
            Some(&auth),
            None,
            None,
        )
        .await;
        let success_poll = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;
        let active_session = json_request(
            &app,
            Method::GET,
            "/api/cnki/session",
            Some(&auth),
            None,
            None,
        )
        .await;
        let storage_session =
            ps_storage::get_cnki_session_status(backend.auth_db_path(), user.user_id())
                .expect("CNKI session status should load");
        let cleared = json_request(
            &app,
            Method::DELETE,
            "/api/cnki/session",
            Some(&auth),
            None,
            None,
        )
        .await;

        assert_eq!(empty_session.status, StatusCode::OK);
        assert_eq!(empty_session.payload["status"], "empty");
        assert_eq!(empty_session.payload["configured"], false);
        assert_eq!(poll_before_start.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            poll_before_start.payload["detail"]["code"],
            "cnki_login_not_started"
        );
        assert_eq!(invalid_poll.status, StatusCode::BAD_REQUEST);
        assert_eq!(waiting_start.status, StatusCode::OK);
        assert_eq!(waiting_start.payload["status"], "WAITING_SCAN");
        assert_eq!(waiting_start.payload["session"]["status"], "waiting_scan");
        assert_eq!(timeout_poll.status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(timeout_poll.payload["detail"]["code"], "cnki_login_timeout");
        assert_eq!(warmup_poll.status, StatusCode::BAD_GATEWAY);
        assert_eq!(warmup_poll.payload["detail"]["code"], "cnki_warmup_failed");
        assert_eq!(success_start.status, StatusCode::OK);
        assert_eq!(success_poll.status, StatusCode::OK);
        assert_eq!(success_poll.payload["status"], "COMPLETE");
        assert_eq!(success_poll.payload["session"]["status"], "active");
        assert_eq!(
            success_poll.payload["session"]["cookie_names"],
            serde_json::json!(["userToken", "vpn358_sid"])
        );
        assert!(!success_poll
            .payload
            .to_string()
            .contains("SECRET_COOKIE_VALUE"));
        assert!(!success_poll
            .payload
            .to_string()
            .contains("SECRET_VPN_VALUE"));
        assert_eq!(active_session.status, StatusCode::OK);
        assert_eq!(active_session.payload["has_bff_user_token"], true);
        assert_eq!(storage_session.status, "active");
        assert_eq!(storage_session.cookie_names, ["userToken", "vpn358_sid"]);
        assert_eq!(cleared.status, StatusCode::OK);
        assert_eq!(cleared.payload["status"], "empty");
        assert_eq!(cleared.payload["configured"], false);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn cnki_routes_use_live_fixture_without_replay_mode() {
        const REPLAY_MODE: &str = "PAPER_SCANNER_CNKI_REPLAY_MODE";
        const LIVE_FIXTURE_MODE: &str = "PAPER_SCANNER_ZJLIB_CNKI_FIXTURE_MODE";

        let _env_guard =
            EnvVarGuard::configured(&[(REPLAY_MODE, None), (LIVE_FIXTURE_MODE, Some("success"))]);
        let backend = TestBackend::new();
        let user = backend.authenticated_user("cnki_live_user", false);
        let app = backend.router();
        let auth = user.authorization_header();

        let start = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/start",
            Some(&auth),
            None,
            None,
        )
        .await;
        let poll = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;
        let session_data =
            ps_storage::get_cnki_session_data(backend.auth_db_path(), user.user_id())
                .expect("CNKI session data should load")
                .expect("CNKI session data should exist");

        assert_eq!(start.status, StatusCode::OK);
        assert_eq!(start.payload["uuid"], "qr-rust-live-fixture");
        assert_eq!(start.payload["session"]["status"], "waiting_scan");
        assert_eq!(poll.status, StatusCode::OK);
        assert_eq!(poll.payload["status"], "COMPLETE");
        assert_eq!(poll.payload["session"]["status"], "active");
        assert_eq!(
            poll.payload["session"]["cookie_names"],
            serde_json::json!(["userToken", "vpn358_sid"])
        );
        let token = session_data.session_data["bff_user_token"]
            .as_str()
            .expect("token should be persisted");
        assert!(poll.payload.to_string().contains("cookie_names"));
        assert!(!poll.payload.to_string().contains(token));
        assert!(!poll.payload.to_string().contains("SECRET_VPN_VALUE"));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn cnki_routes_map_live_fixture_failures_to_stable_error_codes() {
        const REPLAY_MODE: &str = "PAPER_SCANNER_CNKI_REPLAY_MODE";
        const LIVE_FIXTURE_MODE: &str = "PAPER_SCANNER_ZJLIB_CNKI_FIXTURE_MODE";

        let _env_guard = EnvVarGuard::configured(&[
            (REPLAY_MODE, None),
            (LIVE_FIXTURE_MODE, Some("start_failure")),
        ]);
        let backend = TestBackend::new();
        let user = backend.authenticated_user("cnki_failure_user", false);
        let app = backend.router();
        let auth = user.authorization_header();

        let start_failure = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/start",
            Some(&auth),
            None,
            None,
        )
        .await;

        std::env::set_var(LIVE_FIXTURE_MODE, "success");
        let started = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/start",
            Some(&auth),
            None,
            None,
        )
        .await;

        std::env::set_var(LIVE_FIXTURE_MODE, "poll_timeout");
        let timeout = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;

        std::env::set_var(LIVE_FIXTURE_MODE, "poll_failure");
        let poll_failure = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;

        std::env::set_var(LIVE_FIXTURE_MODE, "warmup_failure");
        let warmup_failure = json_request(
            &app,
            Method::POST,
            "/api/cnki/login/poll",
            Some(&auth),
            None,
            Some(serde_json::json!({
                "timeout_seconds": 1,
                "interval_seconds": 0.1
            })),
        )
        .await;

        assert_eq!(start_failure.status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            start_failure.payload["detail"]["code"],
            "cnki_login_start_failed"
        );
        assert_eq!(started.status, StatusCode::OK);
        assert_eq!(timeout.status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(timeout.payload["detail"]["code"], "cnki_login_timeout");
        assert_eq!(poll_failure.status, StatusCode::BAD_REQUEST);
        assert_eq!(poll_failure.payload["detail"]["code"], "cnki_login_failed");
        assert_eq!(warmup_failure.status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            warmup_failure.payload["detail"]["code"],
            "cnki_warmup_failed"
        );
    }

    fn set_cookie_header(response: &JsonTestResponse) -> String {
        response
            .headers
            .get(SET_COOKIE)
            .expect("set-cookie header should exist")
            .to_str()
            .expect("set-cookie should be visible ASCII")
            .to_string()
    }

    fn insert_cnki_fulltext_article(index_database: &FixtureIndexDatabase) -> i64 {
        let article_id = 9100;
        insert_cnki_fulltext_article_at(&index_database.path, article_id);
        article_id
    }

    fn insert_cnki_fulltext_article_at(path: &Path, article_id: i64) {
        let connection = Connection::open(path).expect("fixture index database should open");
        connection
            .execute_batch(
                "
                INSERT INTO journals (
                    journal_id, library_id, platform_journal_id, title, issn, eissn,
                    scimago_rank, cover_url, available, toc_data_approved_and_live,
                    has_articles
                ) VALUES (
                    303, 'cnki', 'CNKI-303', 'Fixture CNKI Journal', '2233-4455',
                    NULL, 2.5, NULL, 1, 1, 1
                );

                INSERT INTO issues (
                    issue_id, journal_id, publication_year, title, volume, number, date,
                    is_valid_issue, suppressed, embargoed, within_subscription
                ) VALUES (
                    202402, 303, 2024, 'CNKI Fixture Issue', '2', '1', '2024-02-01',
                    1, 0, 0, 0
                );
                ",
            )
            .expect("CNKI journal and issue should insert");
        connection
            .execute(
                "
                INSERT INTO articles (
                    article_id, journal_id, issue_id, title, date, authors, start_page,
                    end_page, abstract, doi, pmid, permalink, suppressed, in_press,
                    open_access, platform_id, retraction_doi, within_library_holdings,
                    content_location, full_text_file
                ) VALUES (
                    ?1, 303, 202402, 'Fixture CNKI Article', '2024-02-02',
                    'Ada Lovelace; Grace Hopper', '1', '8',
                    'CNKI fulltext fixture abstract.', NULL, NULL,
                    'https://oversea.cnki.net/kcms/detail/fulltext-fixture',
                    0, 0, 0, 'CNKI-FULLTEXT', NULL, 0, 'remote',
                    'https://o.oversea.cnki.net/barnew/download/order?id=fixture'
                )
                ",
                [article_id],
            )
            .expect("CNKI article should insert");
    }

    async fn wait_for_manual_push_completion(app: &Router, auth: &str) -> JsonTestResponse {
        for _ in 0..100 {
            let response = json_request(
                app,
                Method::GET,
                "/api/tracking/push-weekly/status",
                Some(auth),
                None,
                None,
            )
            .await;
            if response.payload["status"] != "running" {
                return response;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("manual push job did not finish");
    }

    struct EnvVarGuard {
        originals: Vec<(&'static str, Option<String>)>,
        _lock: EnvLockGuard,
    }

    impl EnvVarGuard {
        fn configured(values: &[(&'static str, Option<&'static str>)]) -> Self {
            let lock = EnvLockGuard::acquire();
            let originals = values
                .iter()
                .map(|(name, value)| {
                    let original = std::env::var(name).ok();
                    if let Some(value) = value {
                        std::env::set_var(name, value);
                    } else {
                        std::env::remove_var(name);
                    }
                    (*name, original)
                })
                .collect();
            Self {
                originals,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (name, original) in self.originals.iter().rev() {
                if let Some(value) = original {
                    std::env::set_var(name, value);
                } else {
                    std::env::remove_var(name);
                }
            }
        }
    }

    struct EnvLockGuard;

    impl EnvLockGuard {
        fn acquire() -> Self {
            while TEST_ENV_LOCK
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                std::thread::yield_now();
            }
            Self
        }
    }

    impl Drop for EnvLockGuard {
        fn drop(&mut self) {
            TEST_ENV_LOCK.store(false, Ordering::Release);
        }
    }
}
