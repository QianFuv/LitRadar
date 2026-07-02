//! Health response models shared by API handlers.

use serde::{Deserialize, Serialize};

/// Health check payload returned by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Service status value.
    pub status: String,
}

impl HealthResponse {
    /// Build the Python-compatible healthy status payload.
    ///
    /// # Returns
    ///
    /// Health response with status `ok`.
    pub fn ok() -> Self {
        Self {
            status: "ok".to_string(),
        }
    }
}
