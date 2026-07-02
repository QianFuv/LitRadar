//! Zhejiang Library CNKI session API models.

use serde::{Deserialize, Serialize};

/// Safe per-user Zhejiang Library CNKI session status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CnkiSessionStatusResponse {
    /// Whether a session-like row is configured.
    pub configured: bool,
    /// Safe status label.
    pub status: String,
    /// Whether a BFF user token is present.
    pub has_bff_user_token: bool,
    /// Token expiration timestamp.
    pub expires_at: Option<f64>,
    /// Seconds remaining until expiration.
    pub seconds_remaining: Option<i64>,
    /// Stored cookie names without cookie values.
    pub cookie_names: Vec<String>,
    /// Row update timestamp.
    pub updated_at: Option<f64>,
    /// Last-use timestamp.
    pub last_used_at: Option<f64>,
}

/// Zhejiang Library QR login challenge response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CnkiLoginStartResponse {
    /// QR UUID.
    pub uuid: String,
    /// Upstream login status.
    pub status: String,
    /// QR code URL or payload.
    pub qr_code: String,
    /// Safe session status.
    pub session: CnkiSessionStatusResponse,
}

/// Zhejiang Library QR login polling parameters.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CnkiLoginPollRequest {
    /// Poll timeout in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: i64,
    /// Poll interval in seconds.
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: f64,
}

impl Default for CnkiLoginPollRequest {
    /// Build Python-compatible default polling parameters.
    ///
    /// # Returns
    ///
    /// Default polling request.
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout_seconds(),
            interval_seconds: default_interval_seconds(),
        }
    }
}

/// Zhejiang Library QR login polling response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CnkiLoginPollResponse {
    /// Poll status.
    pub status: String,
    /// Safe session status.
    pub session: CnkiSessionStatusResponse,
}

/// Structured CNKI route error detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CnkiErrorDetail {
    /// Stable error code.
    pub code: String,
    /// CNKI phase that failed.
    pub phase: String,
    /// Human-readable message.
    pub message: String,
}

fn default_timeout_seconds() -> i64 {
    180
}

fn default_interval_seconds() -> f64 {
    2.0
}
