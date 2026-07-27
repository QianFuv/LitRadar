//! Health response models shared by API handlers.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Application-owned service health state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// The requested health boundary is available.
    Ok,
    /// The requested health boundary is not currently available.
    Unhealthy,
}

/// Health check payload returned by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Service status value.
    pub status: HealthStatus,
}

impl HealthResponse {
    /// Build the Python-compatible healthy status payload.
    ///
    /// # Returns
    ///
    /// Health response with status `ok`.
    pub fn ok() -> Self {
        Self {
            status: HealthStatus::Ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HealthResponse, HealthStatus};

    #[test]
    fn health_status_uses_only_declared_wire_values() {
        assert_eq!(
            serde_json::to_string(&HealthResponse::ok()).expect("health should serialize"),
            r#"{"status":"ok"}"#
        );
        assert!(serde_json::from_str::<HealthStatus>(r#""invalid""#).is_err());
    }
}
