//! Shared response helpers for API-compatible error payloads.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// FastAPI-compatible error envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorEnvelope {
    /// Error detail payload.
    pub detail: String,
    /// Stable machine-readable error category.
    pub code: String,
    /// Whether retrying the request may succeed without user correction.
    pub retryable: bool,
}

impl ErrorEnvelope {
    /// Create a generic error envelope with recovery metadata.
    ///
    /// # Arguments
    ///
    /// * `detail` - Error detail message.
    /// * `code` - Stable machine-readable error category.
    /// * `retryable` - Whether retrying may succeed without user correction.
    ///
    /// # Returns
    ///
    /// Error envelope with compatible detail and recovery metadata.
    pub fn new(detail: impl Into<String>, code: impl Into<String>, retryable: bool) -> Self {
        Self {
            detail: detail.into(),
            code: code.into(),
            retryable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorEnvelope;

    #[test]
    fn generic_error_envelope_retains_detail_and_requires_recovery_metadata() {
        let envelope = ErrorEnvelope::new("Authentication required", "unauthorized", false);
        let payload = serde_json::to_value(&envelope).expect("error envelope should serialize");

        assert_eq!(
            payload,
            serde_json::json!({
                "detail": "Authentication required",
                "code": "unauthorized",
                "retryable": false
            })
        );
        assert!(!envelope.code.is_empty());
    }
}
