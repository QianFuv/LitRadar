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
            Self::Http { status, detail } => {
                (status, Json(ErrorEnvelope::new(detail))).into_response()
            }
            Self::JsonDetail { status, detail } => {
                (status, Json(serde_json::json!({ "detail": detail }))).into_response()
            }
            Self::TooManyRequests {
                detail,
                retry_after_seconds,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                [(RETRY_AFTER, retry_after_seconds.to_string())],
                Json(ErrorEnvelope::new(detail)),
            )
                .into_response(),
            Self::Unexpected { cause } => {
                cause.log();
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorEnvelope::new("Internal Server Error")),
                )
                    .into_response()
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
        Self::Http {
            status: StatusCode::SERVICE_UNAVAILABLE,
            detail: "Service temporarily unavailable".to_string(),
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

impl From<BlockingTaskError> for ApiError {
    fn from(error: BlockingTaskError) -> Self {
        match error {
            BlockingTaskError::Closed | BlockingTaskError::TimedOut => Self::service_unavailable(),
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
    use super::{sanitize_error_summary, MAX_ERROR_SUMMARY_CHARACTERS};

    #[test]
    fn internal_error_summary_is_single_line_and_bounded() {
        let summary = format!("safe\nsummary {}", "x".repeat(600));
        let sanitized = sanitize_error_summary(&summary);

        assert!(!sanitized.contains('\n'));
        assert!(sanitized.chars().count() <= MAX_ERROR_SUMMARY_CHARACTERS);
        assert!(sanitized.starts_with("safe summary"));
    }
}
