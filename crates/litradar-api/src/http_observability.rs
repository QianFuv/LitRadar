//! Correlated and privacy-bounded HTTP request logging.

use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request};
use axum::http::header::HeaderName;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;
use tower_http::request_id::{MakeRequestUuid, RequestId, SetRequestIdLayer};
use tracing::Instrument;

/// Header used to return the server-generated request correlation identifier.
pub(crate) const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Wrap an Axum router with request ID replacement, propagation, and terminal logging.
///
/// # Arguments
///
/// * `router` - Router whose complete response surface should be observed.
///
/// # Returns
///
/// Router with the outer HTTP observability layers installed in security-sensitive order.
pub(crate) fn instrument_router<State>(router: Router<State>) -> Router<State>
where
    State: Clone + Send + Sync + 'static,
{
    router
        .layer(from_fn(observe_request))
        .layer(SetRequestIdLayer::new(X_REQUEST_ID, MakeRequestUuid))
        .layer(from_fn(remove_untrusted_request_id))
}

async fn remove_untrusted_request_id(mut request: Request, next: Next) -> Response {
    request.headers_mut().remove(X_REQUEST_ID);
    request.extensions_mut().remove::<RequestId>();
    next.run(request).await
}

async fn observe_request(request: Request, next: Next) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .expect("request ID generation must precede HTTP observation")
        .header_value()
        .clone();
    let request_id_text = request_id
        .to_str()
        .expect("server-generated request IDs must be visible ASCII")
        .to_string();
    let method = method_label(request.method());
    let route = route_label(&request);
    let is_quiet_success = is_quiet_success_route(&route);
    let span = tracing::info_span!(
        "http.request",
        component = "http",
        request_id = %request_id_text,
        method,
        route = %route,
    );

    async move {
        let started_at = Instant::now();
        let mut response = next.run(request).await;
        response.headers_mut().insert(X_REQUEST_ID, request_id);
        emit_completion(
            &request_id_text,
            method,
            &route,
            response.status(),
            started_at.elapsed(),
            is_quiet_success,
        );
        response
    }
    .instrument(span)
    .await
}

fn route_label(request: &Request) -> String {
    let path = request.uri().path();
    if path.starts_with("/_next/static/") {
        return "static.asset".to_string();
    }
    if let Some(matched_path) = request
        .extensions()
        .get::<MatchedPath>()
        .filter(|matched_path| !matched_path.as_str().contains("__private__"))
    {
        return matched_path.as_str().to_string();
    }

    if path == "/api" || path.starts_with("/api/") {
        "api.unmatched".to_string()
    } else if path == "/mcp" || path.starts_with("/mcp/") {
        "mcp.unmatched".to_string()
    } else if path == "/health" || path.starts_with("/health/") {
        "health.unmatched".to_string()
    } else {
        "static.frontend".to_string()
    }
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::CONNECT => "CONNECT",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::PATCH => "PATCH",
        _ => "OTHER",
    }
}

fn is_quiet_success_route(route: &str) -> bool {
    route == "/health/live"
        || route == "/health/ready"
        || route == "static.asset"
        || route == "static.frontend"
}

fn emit_completion(
    request_id: &str,
    method: &str,
    route: &str,
    status: StatusCode,
    duration: Duration,
    is_quiet_success: bool,
) {
    if is_quiet_success && (status.is_success() || status == StatusCode::NOT_MODIFIED) {
        return;
    }

    let status = status.as_u16();
    let duration_ms = duration.as_millis().min(u128::from(u64::MAX)) as u64;
    if status >= 500 {
        tracing::error!(
            event = "http.request.completed",
            component = "http",
            request_id,
            method,
            route,
            status,
            outcome = "server_error",
            duration_ms,
        );
    } else if status >= 400 {
        tracing::warn!(
            event = "http.request.completed",
            component = "http",
            request_id,
            method,
            route,
            status,
            outcome = "client_error",
            duration_ms,
        );
    } else if status >= 300 {
        tracing::info!(
            event = "http.request.completed",
            component = "http",
            request_id,
            method,
            route,
            status,
            outcome = "redirect",
            duration_ms,
        );
    } else {
        tracing::info!(
            event = "http.request.completed",
            component = "http",
            request_id,
            method,
            route,
            status,
            outcome = "success",
            duration_ms,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use axum::body::{to_bytes, Body};
    use axum::extract::State;
    use axum::http::header::{ACCESS_CONTROL_EXPOSE_HEADERS, AUTHORIZATION, ORIGIN};
    use axum::http::{Request, StatusCode};
    use axum::routing::{get, post};
    use axum::Router;
    use litradar_storage::{SecretCodec, StorageConfig};
    use serde_json::Value;
    use tower::ServiceExt;
    use tracing::instrument::WithSubscriber;
    use tracing::Instrument;
    use tracing_subscriber::fmt::MakeWriter;

    use super::{instrument_router, X_REQUEST_ID};
    use crate::config::ApiConfig;
    use crate::response::ApiError;
    use crate::state::ApiState;

    const SECRET_SENTINEL: &str = "http-observability-secret-sentinel";

    #[tokio::test]
    async fn http_observability_replaces_ids_and_emits_safe_terminal_events() {
        let app = instrument_router(
            Router::new()
                .route("/items/{item_id}", post(|| async { StatusCode::OK }))
                .route("/bad-request", get(bad_request))
                .route("/internal-failure", get(internal_failure)),
        );
        let logs = CapturedLogs::default();
        let subscriber = logs.subscriber();

        let (success, client_failure, server_failure) = async {
            let success = app
                .clone()
                .oneshot(
                    Request::post(format!("/items/42?private_query={SECRET_SENTINEL}"))
                        .header(X_REQUEST_ID, SECRET_SENTINEL)
                        .header(AUTHORIZATION, format!("Bearer {SECRET_SENTINEL}"))
                        .body(Body::from(SECRET_SENTINEL))
                        .expect("success request should build"),
                )
                .await
                .expect("success response should be returned");
            let client_failure = app
                .clone()
                .oneshot(
                    Request::get("/bad-request")
                        .header(X_REQUEST_ID, SECRET_SENTINEL)
                        .body(Body::empty())
                        .expect("client-failure request should build"),
                )
                .await
                .expect("client-failure response should be returned");
            let server_failure = app
                .oneshot(
                    Request::get("/internal-failure")
                        .header(X_REQUEST_ID, SECRET_SENTINEL)
                        .body(Body::empty())
                        .expect("server-failure request should build"),
                )
                .await
                .expect("server-failure response should be returned");
            (success, client_failure, server_failure)
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(success.status(), StatusCode::OK);
        assert_eq!(client_failure.status(), StatusCode::BAD_REQUEST);
        assert_eq!(server_failure.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let success_request_id = response_request_id(&success);
        let client_request_id = response_request_id(&client_failure);
        let server_request_id = response_request_id(&server_failure);
        for request_id in [&success_request_id, &client_request_id, &server_request_id] {
            assert!(is_safe_request_id(request_id));
            assert_ne!(request_id, SECRET_SENTINEL);
        }
        assert_ne!(success_request_id, client_request_id);
        assert_ne!(client_request_id, server_request_id);

        let server_payload: Value = serde_json::from_slice(
            &to_bytes(server_failure.into_body(), 1_024)
                .await
                .expect("server response body should load"),
        )
        .expect("server response should remain JSON");
        assert_eq!(
            server_payload,
            serde_json::json!({"detail": "Internal Server Error"})
        );

        let raw_logs = logs.text();
        assert!(!raw_logs.contains(SECRET_SENTINEL));
        let events = logs.events();
        let completions = events
            .iter()
            .filter(|event| event["event"] == "http.request.completed")
            .collect::<Vec<_>>();
        assert_eq!(completions.len(), 3);
        assert_completion(
            &completions,
            StatusCode::OK,
            "INFO",
            "/items/{item_id}",
            &success_request_id,
        );
        assert_completion(
            &completions,
            StatusCode::BAD_REQUEST,
            "WARN",
            "/bad-request",
            &client_request_id,
        );
        assert_completion(
            &completions,
            StatusCode::INTERNAL_SERVER_ERROR,
            "ERROR",
            "/internal-failure",
            &server_request_id,
        );
        let causes = events
            .iter()
            .filter(|event| event["event"] == "http.request.error")
            .collect::<Vec<_>>();
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0]["error_kind"], "unexpected_internal_failure");
        assert!(
            causes[0]["error_summary"]
                .as_str()
                .expect("error summary should be text")
                .chars()
                .count()
                <= 512
        );
        assert_eq!(causes[0]["span"]["request_id"], server_request_id);
    }

    #[tokio::test]
    async fn successful_health_and_static_requests_are_suppressed_but_failures_are_visible() {
        let app = instrument_router(
            Router::new()
                .route("/health/live", get(|| async { StatusCode::OK }))
                .route(
                    "/health/ready",
                    get(|| async { ApiError::service_unavailable() }),
                )
                .fallback(|| async { StatusCode::OK }),
        );
        let logs = CapturedLogs::default();
        let subscriber = logs.subscriber();

        let responses = async {
            let live = app
                .clone()
                .oneshot(empty_request("/health/live"))
                .await
                .expect("live response should be returned");
            let ready = app
                .clone()
                .oneshot(empty_request("/health/ready"))
                .await
                .expect("ready response should be returned");
            let asset = app
                .oneshot(empty_request("/_next/static/chunks/app.js"))
                .await
                .expect("asset response should be returned");
            [live, ready, asset]
        }
        .with_subscriber(subscriber)
        .await;

        for response in &responses {
            assert!(is_safe_request_id(&response_request_id(response)));
        }
        let completions = logs
            .events()
            .into_iter()
            .filter(|event| event["event"] == "http.request.completed")
            .collect::<Vec<_>>();
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0]["status"], 503);
        assert_eq!(completions[0]["level"], "ERROR");
        assert_eq!(completions[0]["route"], "/health/ready");
    }

    #[tokio::test]
    async fn cors_exposes_only_the_generated_request_id_header() {
        let mut config = ApiConfig::new(
            PathBuf::from("cors-observability-root"),
            "127.0.0.1".to_string(),
            0,
            PathBuf::from("secret.key"),
        );
        config.cors_allowed_origins = vec!["https://papers.example".to_string()];
        let app = instrument_router(
            Router::new()
                .route("/cors", get(|| async { StatusCode::OK }))
                .layer(crate::cors_layer(&config)),
        );
        let logs = CapturedLogs::default();
        let subscriber = logs.subscriber();

        let response = app
            .oneshot(
                Request::get("/cors")
                    .header(ORIGIN, "https://papers.example")
                    .body(Body::empty())
                    .expect("CORS request should build"),
            )
            .with_subscriber(subscriber)
            .await
            .expect("CORS response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(is_safe_request_id(&response_request_id(&response)));
        assert_eq!(
            response
                .headers()
                .get(ACCESS_CONTROL_EXPOSE_HEADERS)
                .and_then(|value| value.to_str().ok()),
            Some("x-request-id")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_and_background_events_keep_the_request_span() {
        let state = ApiState::new(
            StorageConfig::from_project_root("http-observability-state-root"),
            SecretCodec::from_key([9_u8; 32]),
            false,
        );
        let app = instrument_router(Router::new().route("/span-probe", get(span_probe)))
            .with_state(state.clone());
        let logs = CapturedLogs::default();
        let subscriber = logs.subscriber();

        let response = app
            .oneshot(empty_request("/span-probe"))
            .with_subscriber(subscriber)
            .await
            .expect("span probe response should be returned");
        state.close_blocking_executor();

        assert_eq!(response.status(), StatusCode::OK);
        let request_id = response_request_id(&response);
        let events = logs.events();
        for event_name in ["test.blocking.event", "test.background.event"] {
            let event = events
                .iter()
                .find(|event| event["event"] == event_name)
                .unwrap_or_else(|| panic!("missing {event_name}"));
            assert_eq!(event["span"]["request_id"], request_id);
            assert_eq!(event["span"]["route"], "/span-probe");
        }
    }

    async fn bad_request() -> ApiError {
        ApiError::bad_request("Invalid request")
    }

    async fn internal_failure() -> ApiError {
        ApiError::internal_server_error()
    }

    async fn span_probe(State(state): State<ApiState>) -> Result<StatusCode, ApiError> {
        state
            .run_blocking(|| {
                tracing::info!(event = "test.blocking.event", component = "test");
            })
            .await?;
        let span = tracing::Span::current();
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        let background = tokio::spawn(
            async move {
                state
                    .run_background_blocking(|| {
                        tracing::info!(event = "test.background.event", component = "test");
                    })
                    .await
            }
            .instrument(span)
            .with_subscriber(subscriber),
        );
        background
            .await
            .map_err(|_| ApiError::internal_server_error())??;
        Ok(StatusCode::OK)
    }

    fn empty_request(uri: &str) -> Request<Body> {
        Request::get(uri)
            .body(Body::empty())
            .expect("request should build")
    }

    fn response_request_id(response: &axum::response::Response) -> String {
        response
            .headers()
            .get(X_REQUEST_ID)
            .and_then(|value| value.to_str().ok())
            .expect("response should contain a visible request ID")
            .to_string()
    }

    fn is_safe_request_id(value: &str) -> bool {
        value.len() == 36
            && value.chars().enumerate().all(|(index, character)| {
                if [8, 13, 18, 23].contains(&index) {
                    character == '-'
                } else {
                    character.is_ascii_hexdigit()
                }
            })
    }

    fn assert_completion(
        events: &[&Value],
        status: StatusCode,
        level: &str,
        route: &str,
        request_id: &str,
    ) {
        let event = events
            .iter()
            .find(|event| event["status"].as_u64() == Some(u64::from(status.as_u16())))
            .unwrap_or_else(|| panic!("missing completion for status {status}"));
        assert_eq!(event["level"], level);
        assert_eq!(event["route"], route);
        assert_eq!(event["request_id"], request_id);
        assert!(event["duration_ms"].as_u64().is_some());
    }

    #[derive(Clone, Default)]
    struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(self.clone())
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .finish()
        }

        fn text(&self) -> String {
            String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured logs should be UTF-8")
        }

        fn events(&self) -> Vec<Value> {
            self.text()
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_str(line).expect("captured log should be JSON"))
                .collect()
        }
    }

    struct CapturedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedLogs {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }
}
