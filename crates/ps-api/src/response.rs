//! HTTP response mapping helpers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ps_domain::ErrorEnvelope;

/// API handler error mapped into FastAPI-compatible envelopes where possible.
#[derive(Debug)]
pub(crate) enum ApiError {
    Http { status: StatusCode, detail: String },
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Http { status, detail } => {
                (status, Json(ErrorEnvelope::new(detail))).into_response()
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

    /// Build an internal server error.
    pub(crate) fn internal_server_error() -> Self {
        Self::Http {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            detail: "Internal Server Error".to_string(),
        }
    }
}
