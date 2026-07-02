//! Rust API server skeleton for backend migration compatibility.

pub mod config;
mod response;
pub mod routes;
pub mod state;

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
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

/// API route prefix preserved from the Python backend.
pub const API_PREFIX: &str = "/api";

/// Cache-Control header for credentialed requests.
pub const AUTHENTICATED_CACHE_CONTROL: &str = "private, no-store";

/// Cache-Control header for unauthenticated index reads.
pub const PUBLIC_INDEX_CACHE_CONTROL: &str = "public, max-age=300, stale-while-revalidate=600";

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
        .nest(API_PREFIX, routes::public_routes())
        .layer(from_fn(cache_control_middleware))
        .layer(cors_layer(&config))
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

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use rusqlite::Connection;
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use super::{build_router, ApiConfig};

    #[tokio::test]
    async fn health_route_matches_python_payload() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let app = build_router(ApiConfig {
            project_root: temp_dir.path().to_path_buf(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cors_allowed_origins: Vec::new(),
        });

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
    async fn announcements_route_reads_existing_auth_database() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("data").join("auth.sqlite");
        std::fs::create_dir_all(auth_db_path.parent().expect("data dir should exist"))
            .expect("data dir should be created");
        let connection = Connection::open(&auth_db_path).expect("auth db should open");
        connection
            .execute_batch(
                "
                CREATE TABLE announcements (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    message TEXT NOT NULL,
                    priority TEXT NOT NULL DEFAULT 'normal',
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at REAL NOT NULL,
                    updated_at REAL NOT NULL
                );
                INSERT INTO announcements
                    (title, message, priority, enabled, created_at, updated_at)
                VALUES
                    ('Normal', 'normal message', 'normal', 1, 20.0, 21.0);
                ",
            )
            .expect("announcement fixture should be created");
        drop(connection);
        let app = build_router(ApiConfig {
            project_root: temp_dir.path().to_path_buf(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cors_allowed_origins: Vec::new(),
        });

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
}
