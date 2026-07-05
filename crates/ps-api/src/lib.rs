//! Rust API server skeleton for backend migration compatibility.

pub mod config;
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
    use axum::body::{to_bytes, Body};
    use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
    use axum::http::{Method, Request, StatusCode};
    use rusqlite::Connection;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::test_support::{json_request, JsonTestResponse, TestBackend};

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
        assert_eq!(database_names, [index_database.db_name.clone()]);
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

    fn set_cookie_header(response: &JsonTestResponse) -> String {
        response
            .headers
            .get(SET_COOKIE)
            .expect("set-cookie header should exist")
            .to_str()
            .expect("set-cookie should be visible ASCII")
            .to_string()
    }
}
