//! Zhejiang Library CNKI session API models.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

/// Known CNKI session states with a lossless branch for upstream additions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CnkiStatus {
    /// No session material is configured.
    Empty,
    /// A QR challenge is waiting to be scanned.
    WaitingScan,
    /// The stored session is active.
    Active,
    /// The stored session has expired.
    Expired,
    /// Upstream or legacy state not owned by LitRadar.
    Unknown(String),
}

impl CnkiStatus {
    /// Return the original or canonical wire value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Empty => "empty",
            Self::WaitingScan => "waiting_scan",
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Unknown(value) => value,
        }
    }
}

impl From<String> for CnkiStatus {
    /// Classify a status while retaining unrecognized upstream text.
    fn from(value: String) -> Self {
        match value.as_str() {
            "empty" => Self::Empty,
            "waiting_scan" => Self::WaitingScan,
            "active" => Self::Active,
            "expired" => Self::Expired,
            _ => Self::Unknown(value),
        }
    }
}

impl From<&str> for CnkiStatus {
    /// Classify borrowed status text.
    fn from(value: &str) -> Self {
        Self::from(value.to_string())
    }
}

impl Serialize for CnkiStatus {
    /// Serialize the status as its stable or original string value.
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CnkiStatus {
    /// Deserialize a known state or retain the exact unknown string.
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::from)
    }
}

/// Safe per-user Zhejiang Library CNKI session status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CnkiSessionStatusResponse {
    /// Whether a session-like row is configured.
    pub configured: bool,
    /// Safe status label.
    #[schema(value_type = String)]
    pub status: CnkiStatus,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CnkiLoginStartResponse {
    /// QR UUID.
    pub uuid: String,
    /// Upstream login status.
    #[schema(value_type = String)]
    pub status: CnkiStatus,
    /// QR code URL or payload.
    pub qr_code: String,
    /// Safe session status.
    pub session: CnkiSessionStatusResponse,
}

/// Zhejiang Library QR login polling parameters.
#[derive(Debug, Clone, PartialEq, Deserialize, ToSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CnkiLoginPollResponse {
    /// Poll status.
    #[schema(value_type = String)]
    pub status: CnkiStatus,
    /// Safe session status.
    pub session: CnkiSessionStatusResponse,
}

/// Structured CNKI route error detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

#[cfg(test)]
mod tests {
    use super::{CnkiLoginPollRequest, CnkiStatus};

    #[test]
    fn login_poll_request_uses_python_compatible_defaults() {
        let request = CnkiLoginPollRequest::default();

        assert_eq!(request.timeout_seconds, 180);
        assert_eq!(request.interval_seconds, 2.0);
    }

    #[test]
    fn login_poll_request_deserializes_default_fields() {
        let request: CnkiLoginPollRequest =
            serde_json::from_str("{}").expect("request should deserialize");

        assert_eq!(request, CnkiLoginPollRequest::default());
    }

    #[test]
    fn unknown_cnki_status_round_trips_without_coercion() {
        let status = serde_json::from_str::<CnkiStatus>(r#""等待确认""#)
            .expect("unknown upstream state should deserialize");

        assert_eq!(status, CnkiStatus::Unknown("等待确认".to_string()));
        assert_eq!(status.as_str(), "等待确认");
        assert_eq!(
            serde_json::to_string(&status).expect("unknown state should serialize"),
            r#""等待确认""#
        );
    }
}
