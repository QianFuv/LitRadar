//! Shared response helpers for API-compatible error payloads.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// FastAPI-compatible error envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorEnvelope {
    /// Error detail payload.
    pub detail: String,
}

impl ErrorEnvelope {
    /// Create an error envelope from a detail message.
    ///
    /// # Arguments
    ///
    /// * `detail` - Error detail message.
    ///
    /// # Returns
    ///
    /// Error envelope with the provided detail.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}
