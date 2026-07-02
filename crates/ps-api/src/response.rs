//! HTTP response mapping helpers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ps_domain::ErrorEnvelope;
use ps_storage::AnnouncementRepositoryError;

/// API handler error mapped into FastAPI-compatible envelopes where possible.
#[derive(Debug)]
pub(crate) enum ApiError {
    Storage,
}

impl From<AnnouncementRepositoryError> for ApiError {
    fn from(_error: AnnouncementRepositoryError) -> Self {
        Self::Storage
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Storage => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorEnvelope::new("Internal Server Error")),
            )
                .into_response(),
        }
    }
}
