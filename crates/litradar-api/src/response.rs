//! HTTP response mapping helpers.

use std::panic::Location;
use std::path::Path;

use axum::http::header::RETRY_AFTER;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use litradar_domain::ErrorEnvelope;

use crate::state::BlockingTaskError;

const MAX_ERROR_SUMMARY_CHARACTERS: usize = 512;
const ARTICLE_PROVIDER_BAD_GATEWAY_DETAIL: &str = "Article provider request failed";
const ARTICLE_PROVIDER_RETRYABLE_DETAIL: &str = "Article provider temporarily unavailable";
const ARTICLE_PROVIDER_RETRY_AFTER_SECONDS: u64 = 5;
const SERVICE_UNAVAILABLE_RETRY_AFTER_SECONDS: u64 = 5;

/// API handler error mapped into FastAPI-compatible envelopes where possible.
#[derive(Debug)]
pub(crate) enum ApiError {
    Http {
        status: StatusCode,
        detail: String,
    },
    JsonDetail {
        status: StatusCode,
        detail: serde_json::Value,
    },
    TooManyRequests {
        status: StatusCode,
        detail: String,
        retry_after_seconds: u64,
    },
    Unexpected {
        cause: InternalErrorCause,
    },
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Http { status, detail } => generic_error_response(status, detail, None),
            Self::JsonDetail { status, detail } => {
                (status, Json(serde_json::json!({ "detail": detail }))).into_response()
            }
            Self::TooManyRequests {
                status,
                detail,
                retry_after_seconds,
            } => generic_error_response(status, detail, Some(retry_after_seconds)),
            Self::Unexpected { cause } => {
                cause.log();
                generic_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal Server Error".to_string(),
                    None,
                )
            }
        }
    }
}

impl ApiError {
    /// Build a bad request error.
    pub(crate) fn bad_request(detail: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::BAD_REQUEST,
            detail: detail.into(),
        }
    }

    /// Build an unauthorized error.
    pub(crate) fn unauthorized(detail: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::UNAUTHORIZED,
            detail: detail.into(),
        }
    }

    /// Build a not found error.
    pub(crate) fn not_found(detail: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::NOT_FOUND,
            detail: detail.into(),
        }
    }

    /// Build a conflict error.
    pub(crate) fn conflict(detail: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::CONFLICT,
            detail: detail.into(),
        }
    }

    /// Build a forbidden error.
    pub(crate) fn forbidden(detail: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::FORBIDDEN,
            detail: detail.into(),
        }
    }

    /// Build a rate-limit error with a Retry-After header.
    pub(crate) fn too_many_requests(detail: impl Into<String>, retry_after_seconds: u64) -> Self {
        Self::TooManyRequests {
            status: StatusCode::TOO_MANY_REQUESTS,
            detail: detail.into(),
            retry_after_seconds,
        }
    }

    /// Build an internal server error.
    #[track_caller]
    pub(crate) fn internal_server_error() -> Self {
        Self::unexpected(
            "unexpected_internal_failure",
            "internal operation failed",
            Location::caller(),
        )
    }

    /// Build a service-unavailable error without exposing executor details.
    pub(crate) fn service_unavailable() -> Self {
        Self::TooManyRequests {
            status: StatusCode::SERVICE_UNAVAILABLE,
            detail: "Service temporarily unavailable".to_string(),
            retry_after_seconds: SERVICE_UNAVAILABLE_RETRY_AFTER_SECONDS,
        }
    }

    /// Build a safe bad-gateway error for a failed article Provider request.
    pub(crate) fn article_provider_bad_gateway() -> Self {
        Self::Http {
            status: StatusCode::BAD_GATEWAY,
            detail: ARTICLE_PROVIDER_BAD_GATEWAY_DETAIL.to_string(),
        }
    }

    /// Build a retryable article Provider error with a fixed Retry-After header.
    pub(crate) fn article_provider_service_unavailable() -> Self {
        Self::TooManyRequests {
            status: StatusCode::SERVICE_UNAVAILABLE,
            detail: ARTICLE_PROVIDER_RETRYABLE_DETAIL.to_string(),
            retry_after_seconds: ARTICLE_PROVIDER_RETRY_AFTER_SECONDS,
        }
    }

    /// Build an error with a structured JSON detail payload.
    pub(crate) fn json_detail(status: StatusCode, detail: serde_json::Value) -> Self {
        Self::JsonDetail { status, detail }
    }

    fn unexpected(
        error_kind: &'static str,
        error_summary: &'static str,
        location: &'static Location<'static>,
    ) -> Self {
        Self::Unexpected {
            cause: InternalErrorCause::new(error_kind, error_summary, location),
        }
    }
}

fn generic_error_response(
    status: StatusCode,
    detail: String,
    retry_after_seconds: Option<u64>,
) -> Response {
    let retryable = retry_after_seconds.is_some();
    let envelope = ErrorEnvelope::new(detail, generic_error_code(status), retryable);
    match retry_after_seconds {
        Some(seconds) => {
            (status, [(RETRY_AFTER, seconds.to_string())], Json(envelope)).into_response()
        }
        None => (status, Json(envelope)).into_response(),
    }
}

fn generic_error_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        StatusCode::BAD_GATEWAY => "bad_gateway",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        StatusCode::INTERNAL_SERVER_ERROR => "internal_server_error",
        _ => "request_failed",
    }
}

impl From<BlockingTaskError> for ApiError {
    fn from(error: BlockingTaskError) -> Self {
        match error {
            BlockingTaskError::Closed | BlockingTaskError::QueueTimedOut => {
                Self::service_unavailable()
            }
            BlockingTaskError::Join => Self::unexpected(
                "blocking_task_join_failed",
                "blocking task failed to join",
                Location::caller(),
            ),
        }
    }
}

/// Private safe metadata retained for an unexpected HTTP failure.
#[derive(Debug)]
pub(crate) struct InternalErrorCause {
    error_kind: &'static str,
    error_summary: String,
    source_file: String,
    source_line: u32,
}

impl InternalErrorCause {
    fn new(
        error_kind: &'static str,
        error_summary: &'static str,
        location: &'static Location<'static>,
    ) -> Self {
        Self {
            error_kind,
            error_summary: sanitize_error_summary(error_summary),
            source_file: Path::new(location.file())
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_string(),
            source_line: location.line(),
        }
    }

    fn log(&self) {
        tracing::error!(
            event = "http.request.error",
            component = "http",
            error_kind = self.error_kind,
            error_summary = %self.error_summary,
            error_source = %self.source_file,
            error_line = self.source_line,
        );
    }
}

fn sanitize_error_summary(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_ERROR_SUMMARY_CHARACTERS)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::header::RETRY_AFTER;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::{
        sanitize_error_summary, ApiError, ARTICLE_PROVIDER_BAD_GATEWAY_DETAIL,
        ARTICLE_PROVIDER_RETRYABLE_DETAIL, MAX_ERROR_SUMMARY_CHARACTERS,
    };

    #[test]
    fn internal_error_summary_is_single_line_and_bounded() {
        let summary = format!("safe\nsummary {}", "x".repeat(600));
        let sanitized = sanitize_error_summary(&summary);

        assert!(!sanitized.contains('\n'));
        assert!(sanitized.chars().count() <= MAX_ERROR_SUMMARY_CHARACTERS);
        assert!(sanitized.starts_with("safe summary"));
    }

    #[tokio::test]
    async fn article_provider_errors_use_safe_stable_responses() {
        let bad_gateway = ApiError::article_provider_bad_gateway().into_response();
        assert_eq!(bad_gateway.status(), StatusCode::BAD_GATEWAY);
        assert!(bad_gateway.headers().get(RETRY_AFTER).is_none());
        let bad_gateway_body = to_bytes(bad_gateway.into_body(), usize::MAX)
            .await
            .expect("bad-gateway body should read");
        let bad_gateway_payload: serde_json::Value =
            serde_json::from_slice(&bad_gateway_body).expect("bad-gateway body should be JSON");
        assert_eq!(
            bad_gateway_payload["detail"],
            ARTICLE_PROVIDER_BAD_GATEWAY_DETAIL
        );
        assert_eq!(bad_gateway_payload["code"], "bad_gateway");
        assert_eq!(bad_gateway_payload["retryable"], false);

        let unavailable = ApiError::article_provider_service_unavailable().into_response();
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            unavailable
                .headers()
                .get(RETRY_AFTER)
                .expect("retryable response should include Retry-After"),
            "5"
        );
        let unavailable_body = to_bytes(unavailable.into_body(), usize::MAX)
            .await
            .expect("service-unavailable body should read");
        let unavailable_payload: serde_json::Value = serde_json::from_slice(&unavailable_body)
            .expect("service-unavailable body should be JSON");
        assert_eq!(
            unavailable_payload["detail"],
            ARTICLE_PROVIDER_RETRYABLE_DETAIL
        );
        assert_eq!(unavailable_payload["code"], "service_unavailable");
        assert_eq!(unavailable_payload["retryable"], true);
    }

    #[tokio::test]
    async fn generic_errors_expose_stable_recovery_categories() {
        for (error, status, code) in [
            (
                ApiError::bad_request("Invalid request"),
                StatusCode::BAD_REQUEST,
                "bad_request",
            ),
            (
                ApiError::unauthorized("Authentication required"),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                ApiError::forbidden("Admin access required"),
                StatusCode::FORBIDDEN,
                "forbidden",
            ),
            (
                ApiError::not_found("Article not found"),
                StatusCode::NOT_FOUND,
                "not_found",
            ),
            (
                ApiError::conflict("State changed"),
                StatusCode::CONFLICT,
                "conflict",
            ),
        ] {
            let response = error.into_response();
            assert_eq!(response.status(), status);
            assert!(response.headers().get(RETRY_AFTER).is_none());
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("generic response body should read");
            let payload: serde_json::Value =
                serde_json::from_slice(&body).expect("generic response should be JSON");
            assert_eq!(payload["code"], code);
            assert_eq!(payload["retryable"], false);
            assert!(payload["detail"].is_string());
        }

        let unavailable = ApiError::service_unavailable().into_response();
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            unavailable
                .headers()
                .get(RETRY_AFTER)
                .expect("service-unavailable response should include Retry-After")
                .to_str()
                .expect("service-unavailable Retry-After should be text"),
            "5"
        );
        let body = to_bytes(unavailable.into_body(), usize::MAX)
            .await
            .expect("service-unavailable body should read");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("service-unavailable response should be JSON");
        assert_eq!(payload["code"], "service_unavailable");
        assert_eq!(payload["retryable"], true);

        let limited = ApiError::too_many_requests("Slow down", 12).into_response();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            limited
                .headers()
                .get(RETRY_AFTER)
                .expect("rate-limit response should include Retry-After"),
            "12"
        );
        let body = to_bytes(limited.into_body(), usize::MAX)
            .await
            .expect("rate-limit body should read");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("rate-limit response should be JSON");
        assert_eq!(payload["code"], "rate_limited");
        assert_eq!(payload["retryable"], true);
    }

    #[tokio::test]
    async fn unexpected_errors_expose_only_safe_nonretryable_metadata() {
        let response = ApiError::internal_server_error().into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.headers().get(RETRY_AFTER).is_none());
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("internal response body should read");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("internal response should be JSON");
        assert_eq!(
            payload,
            serde_json::json!({
                "detail": "Internal Server Error",
                "code": "internal_server_error",
                "retryable": false
            })
        );
    }
}
