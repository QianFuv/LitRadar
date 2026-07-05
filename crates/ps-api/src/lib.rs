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
    use axum::http::header::{AUTHORIZATION, COOKIE};
    use axum::http::{Request, StatusCode};
    use rusqlite::Connection;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::test_support::TestBackend;

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
}
