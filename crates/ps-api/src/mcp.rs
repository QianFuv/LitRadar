//! Streamable HTTP MCP integration for the Rust API server.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::ServerHandler;
use tower::Service;

use crate::config::ApiConfig;
use crate::routes::auth;
use crate::state::ApiState;

type InnerMcpService = StreamableHttpService<PaperScannerMcp, LocalSessionManager>;

/// Build the authenticated Streamable HTTP MCP service.
///
/// # Arguments
///
/// * `config` - Runtime API configuration.
/// * `state` - Shared API state used for authentication.
///
/// # Returns
///
/// Tower service that rejects unauthenticated requests before MCP execution.
pub(crate) fn service(config: &ApiConfig, state: ApiState) -> AuthenticatedMcpService {
    let mcp_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(config.mcp_allowed_hosts.clone())
        .with_allowed_origins(config.mcp_allowed_origins.clone());
    let inner = StreamableHttpService::new(
        || Ok(PaperScannerMcp),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );

    AuthenticatedMcpService { state, inner }
}

#[derive(Clone)]
pub(crate) struct AuthenticatedMcpService {
    state: ApiState,
    inner: InnerMcpService,
}

impl Service<Request> for AuthenticatedMcpService {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let state = self.state.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            if let Err(error) = auth::require_current_user(&state, request.headers()) {
                return Ok(error.into_response());
            }

            match inner.call(request).await {
                Ok(response) => Ok(response.into_response()),
                Err(error) => match error {},
            }
        })
    }
}

struct PaperScannerMcp;

impl ServerHandler for PaperScannerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Use Paper Scanner tools to query indexed papers and favorites.")
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, HOST};
    use axum::http::{Method, Request, StatusCode};
    use axum::response::Response;
    use axum::Router;
    use tower::ServiceExt;

    use crate::test_support::TestBackend;

    const INITIALIZE_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ps-api-test","version":"0.1.0"}}}"#;
    const INITIALIZED_BODY: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    const TOOLS_LIST_BODY: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_initialize_requires_authentication() {
        let backend = TestBackend::new();
        let app = backend.router();

        let response = send_mcp_post(&app, "localhost", None, None, None, INITIALIZE_BODY).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_initialize_rejects_unconfigured_host() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_host_user", false);
        let app = backend.router();

        let response = send_mcp_post(
            &app,
            "paper.example",
            Some(&user.authorization_header()),
            None,
            None,
            INITIALIZE_BODY,
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_initialize_accepts_session_cookie_authentication() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_cookie_user", false);
        let app = backend.router();

        let response = send_mcp_post(
            &app,
            "localhost",
            None,
            Some(&user.cookie_header()),
            None,
            INITIALIZE_BODY,
        )
        .await;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response_body(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(headers.contains_key("mcp-session-id"));
        assert!(body.contains(r#""id":1"#));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_tools_list_accepts_bearer_authentication() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_bearer_user", false);
        let app = backend.router();
        let authorization = user.authorization_header();

        let initialize_response = send_mcp_post(
            &app,
            "localhost",
            Some(&authorization),
            None,
            None,
            INITIALIZE_BODY,
        )
        .await;
        let session_id = initialize_response
            .headers()
            .get("mcp-session-id")
            .expect("initialize response should include MCP session")
            .to_str()
            .expect("MCP session id should be visible ASCII")
            .to_string();
        assert_eq!(initialize_response.status(), StatusCode::OK);

        let initialized_response = send_mcp_post(
            &app,
            "localhost",
            Some(&authorization),
            None,
            Some(&session_id),
            INITIALIZED_BODY,
        )
        .await;
        assert_eq!(initialized_response.status(), StatusCode::ACCEPTED);

        let tools_response = send_mcp_post(
            &app,
            "localhost",
            Some(&authorization),
            None,
            Some(&session_id),
            TOOLS_LIST_BODY,
        )
        .await;
        let status = tools_response.status();
        let body = response_body(tools_response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""id":2"#));
        assert!(body.contains(r#""tools":[]"#));
    }

    async fn send_mcp_post(
        app: &Router,
        host: &str,
        authorization: Option<&str>,
        cookie: Option<&str>,
        session_id: Option<&str>,
        body: &str,
    ) -> Response {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(HOST, host)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream");
        if let Some(value) = authorization {
            builder = builder.header(AUTHORIZATION, value);
        }
        if let Some(value) = cookie {
            builder = builder.header(COOKIE, value);
        }
        if let Some(value) = session_id {
            builder = builder
                .header("mcp-session-id", value)
                .header("mcp-protocol-version", "2025-06-18");
        }

        app.clone()
            .oneshot(
                builder
                    .body(Body::from(body.to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned")
    }

    async fn response_body(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        String::from_utf8(body.to_vec()).expect("body should be UTF-8")
    }
}
