//! PushPlus delivery client for notification workers.

use std::error::Error;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::retry::{bounded_retry_attempts, retry_backoff_delay};

/// PushPlus send endpoint.
pub const PUSHPLUS_ENDPOINT: &str = "https://www.pushplus.plus/send";

const TRANSIENT_STATUS_CODES: [u16; 5] = [429, 500, 502, 503, 504];

/// Error returned by PushPlus delivery clients.
#[derive(Debug, Clone, PartialEq)]
pub enum PushPlusError {
    /// HTTP transport failed before a response payload was available.
    Transport(String),
    /// Upstream returned a non-success HTTP status.
    HttpStatus {
        /// HTTP status code.
        status_code: u16,
        /// Parsed response payload or raw body wrapper.
        body: Value,
    },
    /// PushPlus returned an application-level failure.
    Api {
        /// PushPlus response code.
        code: Option<i64>,
        /// PushPlus response message.
        message: String,
    },
    /// PushPlus response could not be parsed.
    InvalidResponse(String),
}

impl fmt::Display for PushPlusError {
    /// Format the PushPlus error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) => formatter.write_str(message),
            Self::HttpStatus { status_code, body } => {
                write!(
                    formatter,
                    "PushPlus request failed with HTTP {status_code}: {body}"
                )
            }
            Self::Api { code, message } => {
                write!(formatter, "PushPlus failed with code {code:?}: {message}")
            }
            Self::InvalidResponse(message) => formatter.write_str(message),
        }
    }
}

impl Error for PushPlusError {}

/// PushPlus message payload.
#[derive(Clone, PartialEq, Eq)]
pub struct PushPlusMessage {
    /// PushPlus token.
    pub token: String,
    /// Message title.
    pub title: String,
    /// Message content.
    pub content: String,
    /// PushPlus channel.
    pub channel: String,
    /// PushPlus template.
    pub template: String,
    /// Optional PushPlus topic.
    pub topic: Option<String>,
    /// Optional PushPlus channel option.
    pub option: Option<String>,
    /// Optional recipient value.
    pub to: Option<String>,
}

impl fmt::Debug for PushPlusMessage {
    /// Format a message without exposing the PushPlus token.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushPlusMessage")
            .field("token", &"[REDACTED]")
            .field("title", &self.title)
            .field("content", &self.content)
            .field("channel", &self.channel)
            .field("template", &self.template)
            .field("topic", &self.topic)
            .finish()
    }
}

/// HTTP request sent to PushPlus.
#[derive(Clone, PartialEq)]
pub struct PushPlusHttpRequest {
    /// Request URL.
    pub url: String,
    /// JSON request body.
    pub body: Value,
}

impl fmt::Debug for PushPlusHttpRequest {
    /// Format a PushPlus request without exposing token-bearing body data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushPlusHttpRequest")
            .field("url", &self.url)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// HTTP response returned by a PushPlus transport.
#[derive(Debug, Clone, PartialEq)]
pub struct PushPlusHttpResponse {
    /// HTTP status code.
    pub status_code: u16,
    /// JSON response body.
    pub body: Value,
}

struct PushPlusSendResponse {
    message_id: String,
    status_code: u16,
}

/// Transport boundary for PushPlus HTTP calls.
pub trait PushPlusTransport {
    /// Send one JSON POST request.
    ///
    /// # Arguments
    ///
    /// * `request` - HTTP request payload.
    ///
    /// # Returns
    ///
    /// HTTP response payload.
    fn post_json(
        &mut self,
        request: PushPlusHttpRequest,
    ) -> Result<PushPlusHttpResponse, PushPlusError>;
}

/// Reqwest-backed PushPlus transport.
#[derive(Debug, Clone)]
pub struct ReqwestPushPlusTransport {
    client: Client,
}

impl ReqwestPushPlusTransport {
    /// Build a reqwest-backed PushPlus transport.
    ///
    /// # Arguments
    ///
    /// * `timeout_seconds` - Request timeout in seconds.
    ///
    /// # Returns
    ///
    /// PushPlus transport.
    pub fn new(timeout_seconds: u64) -> Result<Self, PushPlusError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(1)))
            .build()
            .map_err(|error| PushPlusError::Transport(error.to_string()))?;
        Ok(Self { client })
    }
}

impl PushPlusTransport for ReqwestPushPlusTransport {
    /// Send one JSON POST request through reqwest.
    fn post_json(
        &mut self,
        request: PushPlusHttpRequest,
    ) -> Result<PushPlusHttpResponse, PushPlusError> {
        let response = self
            .client
            .post(&request.url)
            .json(&request.body)
            .send()
            .map_err(|error| PushPlusError::Transport(error.to_string()))?;
        let status_code = response.status().as_u16();
        let text = response
            .text()
            .map_err(|error| PushPlusError::Transport(error.to_string()))?;
        let body =
            serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "error": text }));
        Ok(PushPlusHttpResponse { status_code, body })
    }
}

/// PushPlus delivery client.
pub struct PushPlusClient<T: PushPlusTransport> {
    transport: T,
    retry_attempts: usize,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
}

impl<T: PushPlusTransport> PushPlusClient<T> {
    /// Build a PushPlus client.
    ///
    /// # Arguments
    ///
    /// * `transport` - HTTP transport implementation.
    /// * `retry_attempts` - Retry attempts.
    ///
    /// # Returns
    ///
    /// PushPlus client.
    pub fn new(transport: T, retry_attempts: usize) -> Self {
        Self {
            transport,
            retry_attempts: bounded_retry_attempts(retry_attempts),
            sleep: Box::new(thread::sleep),
        }
    }

    /// Replace the sleep callback used between retry attempts.
    ///
    /// # Arguments
    ///
    /// * `sleep` - Replacement sleep callback.
    ///
    /// # Returns
    ///
    /// PushPlus client with the replacement callback.
    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    /// Return the underlying transport.
    ///
    /// # Returns
    ///
    /// Shared transport reference.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Send one PushPlus message.
    ///
    /// # Arguments
    ///
    /// * `message` - PushPlus message payload.
    ///
    /// # Returns
    ///
    /// PushPlus message id.
    pub fn send(&mut self, message: &PushPlusMessage) -> Result<String, PushPlusError> {
        let started_at = Instant::now();
        let send_span = tracing::info_span!(
            "pushplus.delivery",
            component = "delivery",
            provider = "pushplus",
            endpoint = "send",
        );
        send_span.in_scope(|| {
            tracing::info!(
                event = "pushplus.delivery.started",
                component = "delivery",
                outcome = "started",
            );
            let result = self.send_attempts(message);
            match &result {
                Ok(_) => tracing::info!(
                    event = "pushplus.delivery.completed",
                    component = "delivery",
                    outcome = "success",
                    duration_ms = elapsed_millis(started_at),
                ),
                Err(error) => tracing::warn!(
                    event = "pushplus.delivery.failed",
                    component = "delivery",
                    outcome = "failure",
                    error_kind = pushplus_error_kind(error),
                    duration_ms = elapsed_millis(started_at),
                ),
            }
            result
        })
    }

    fn send_attempts(&mut self, message: &PushPlusMessage) -> Result<String, PushPlusError> {
        let mut last_error =
            PushPlusError::InvalidResponse("PushPlus request was not attempted".into());
        for attempt in 0..=self.retry_attempts {
            let request = PushPlusHttpRequest {
                url: PUSHPLUS_ENDPOINT.to_string(),
                body: pushplus_body(message),
            };
            let attempt_started_at = Instant::now();
            match self.send_once(request) {
                Ok(response) => {
                    tracing::info!(
                        event = "pushplus.request.completed",
                        component = "delivery",
                        outcome = "success",
                        attempt = attempt + 1,
                        http_status = response.status_code,
                        duration_ms = elapsed_millis(attempt_started_at),
                    );
                    return Ok(response.message_id);
                }
                Err(error) => {
                    let can_retry = attempt < self.retry_attempts;
                    let should_retry = can_retry
                        && match &error {
                            PushPlusError::HttpStatus { status_code, .. } => {
                                TRANSIENT_STATUS_CODES.contains(status_code)
                            }
                            _ => true,
                        };
                    emit_pushplus_request_failure(
                        &error,
                        attempt + 1,
                        should_retry,
                        attempt_started_at,
                    );
                    last_error = error;
                    if should_retry {
                        (self.sleep)(retry_backoff_delay(attempt));
                        continue;
                    }
                    if can_retry && !matches!(last_error, PushPlusError::HttpStatus { .. }) {
                        (self.sleep)(retry_backoff_delay(attempt));
                        continue;
                    }
                    break;
                }
            }
        }
        Err(PushPlusError::InvalidResponse(format!(
            "PushPlus request failed: {last_error}"
        )))
    }

    fn send_once(
        &mut self,
        request: PushPlusHttpRequest,
    ) -> Result<PushPlusSendResponse, PushPlusError> {
        let response = self.transport.post_json(request)?;
        if !(200..300).contains(&response.status_code) {
            return Err(PushPlusError::HttpStatus {
                status_code: response.status_code,
                body: response.body,
            });
        }
        let object = response.body.as_object().ok_or_else(|| {
            PushPlusError::InvalidResponse("PushPlus response is not a JSON object".into())
        })?;
        let code = object.get("code").and_then(json_i64);
        if code != Some(200) {
            let message = object
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("Unknown PushPlus error")
                .to_string();
            return Err(PushPlusError::Api { code, message });
        }
        Ok(PushPlusSendResponse {
            message_id: object.get("data").map(json_string).unwrap_or_default(),
            status_code: response.status_code,
        })
    }
}

/// Build a live PushPlus delivery client.
///
/// # Arguments
///
/// * `timeout_seconds` - Request timeout in seconds.
/// * `retry_attempts` - Retry attempts.
///
/// # Returns
///
/// Live PushPlus client.
pub fn live_pushplus_client(
    timeout_seconds: u64,
    retry_attempts: usize,
) -> Result<PushPlusClient<ReqwestPushPlusTransport>, PushPlusError> {
    Ok(PushPlusClient::new(
        ReqwestPushPlusTransport::new(timeout_seconds)?,
        retry_attempts,
    ))
}

fn pushplus_body(message: &PushPlusMessage) -> Value {
    let mut body = json!({
        "token": message.token,
        "title": message.title,
        "content": message.content,
        "channel": message.channel,
        "template": message.template
    });
    let object = body
        .as_object_mut()
        .expect("PushPlus payload should be a JSON object");
    if let Some(to) = message
        .to
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        object.insert("to".into(), Value::String(to.to_string()));
    }
    if let Some(topic) = message
        .topic
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        object.insert("topic".into(), Value::String(topic.to_string()));
    }
    if let Some(option) = message
        .option
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        object.insert("option".into(), Value::String(option.to_string()));
    }
    body
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

fn json_string(value: &Value) -> String {
    value.as_str().map(str::to_string).unwrap_or_else(|| {
        if value.is_null() {
            String::new()
        } else {
            value.to_string()
        }
    })
}

fn emit_pushplus_request_failure(
    error: &PushPlusError,
    attempt: usize,
    will_retry: bool,
    started_at: Instant,
) {
    let duration_ms = elapsed_millis(started_at);
    match error {
        PushPlusError::HttpStatus { status_code, .. } => tracing::warn!(
            event = "pushplus.request.failed",
            component = "delivery",
            outcome = "failure",
            attempt,
            error_kind = "http_status",
            http_status = status_code,
            will_retry,
            duration_ms,
        ),
        _ => tracing::warn!(
            event = "pushplus.request.failed",
            component = "delivery",
            outcome = "failure",
            attempt,
            error_kind = pushplus_error_kind(error),
            will_retry,
            duration_ms,
        ),
    }
}

fn pushplus_error_kind(error: &PushPlusError) -> &'static str {
    match error {
        PushPlusError::Transport(_) => "transport",
        PushPlusError::HttpStatus { .. } => "http_status",
        PushPlusError::Api { .. } => "api_error",
        PushPlusError::InvalidResponse(_) => "invalid_response",
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use crate::ai::test_support::CapturedLogs;

    use super::*;

    #[derive(Debug, Default)]
    struct FixturePushPlusTransport {
        responses: Vec<Result<PushPlusHttpResponse, PushPlusError>>,
        requests: Vec<PushPlusHttpRequest>,
    }

    impl FixturePushPlusTransport {
        fn new(responses: Vec<Result<PushPlusHttpResponse, PushPlusError>>) -> Self {
            Self {
                responses: responses.into_iter().rev().collect(),
                requests: Vec::new(),
            }
        }
    }

    impl PushPlusTransport for FixturePushPlusTransport {
        fn post_json(
            &mut self,
            request: PushPlusHttpRequest,
        ) -> Result<PushPlusHttpResponse, PushPlusError> {
            self.requests.push(request);
            self.responses
                .pop()
                .unwrap_or_else(|| Err(PushPlusError::Transport("missing fixture response".into())))
        }
    }

    #[test]
    fn pushplus_oversized_retry_counts_are_bounded() {
        let client = PushPlusClient::new(FixturePushPlusTransport::default(), usize::MAX);

        assert_eq!(
            client.retry_attempts,
            litradar_domain::DELIVERY_RETRY_ATTEMPTS_MAX
        );
    }

    #[test]
    fn send_posts_pushplus_payload_and_returns_message_id() {
        let mut client = PushPlusClient::new(
            FixturePushPlusTransport::new(vec![ok_response(json!({
                "code": 200,
                "data": "msg-1"
            }))]),
            0,
        )
        .with_sleep(|_| {});
        let message_id = client
            .send(&message())
            .expect("PushPlus send should succeed");

        assert_eq!(message_id, "msg-1");
        let request = &client.transport().requests[0];
        assert_eq!(request.url, PUSHPLUS_ENDPOINT);
        assert_eq!(request.body["token"], "token");
        assert_eq!(request.body["title"], "Title");
        assert_eq!(request.body["topic"], "topic");
        assert_eq!(request.body["option"], "option");
    }

    #[test]
    fn send_retries_transient_status() {
        let responses = vec![
            Ok(PushPlusHttpResponse {
                status_code: 503,
                body: json!({"error": "busy"}),
            }),
            ok_response(json!({
                "code": 200,
                "data": "msg-2"
            })),
        ];
        let mut client =
            PushPlusClient::new(FixturePushPlusTransport::new(responses), 1).with_sleep(|_| {});
        let message_id = client
            .send(&message())
            .expect("PushPlus send should retry transient failure");

        assert_eq!(message_id, "msg-2");
        assert_eq!(client.transport().requests.len(), 2);
    }

    #[test]
    fn pushplus_attempt_events_omit_token_message_and_response_material() {
        let sentinel = "pushplus-token-message-response-sentinel";
        let responses = vec![
            Ok(PushPlusHttpResponse {
                status_code: 503,
                body: json!({"error": sentinel}),
            }),
            ok_response(json!({
                "code": 200,
                "data": sentinel
            })),
        ];
        let mut message = message();
        message.token = sentinel.to_string();
        message.title = sentinel.to_string();
        message.content = sentinel.to_string();
        let logs = CapturedLogs::default();
        let mut client =
            PushPlusClient::new(FixturePushPlusTransport::new(responses), 1).with_sleep(|_| {});

        let message_id = logs
            .capture(|| client.send(&message))
            .expect("PushPlus retry should succeed");

        assert_eq!(message_id, sentinel);
        let events = logs.events();
        let failed = events
            .iter()
            .find(|event| event["event"] == "pushplus.request.failed")
            .expect("failed attempt should be logged");
        assert_eq!(failed["attempt"], 1);
        assert_eq!(failed["http_status"], 503);
        assert_eq!(failed["will_retry"], true);
        assert_eq!(failed["span"]["endpoint"], "send");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "pushplus.delivery.completed")
                .count(),
            1,
            "{}",
            logs.text()
        );
        assert!(!logs.text().contains(sentinel));
    }

    #[test]
    fn send_rejects_pushplus_error_code() {
        let mut client = PushPlusClient::new(
            FixturePushPlusTransport::new(vec![ok_response(json!({
                "code": 400,
                "msg": "bad token"
            }))]),
            0,
        )
        .with_sleep(|_| {});
        let error = client
            .send(&message())
            .expect_err("PushPlus API error should fail");

        assert!(error.to_string().contains("PushPlus request failed"));
        assert!(error.to_string().contains("bad token"));
    }

    fn ok_response(body: Value) -> Result<PushPlusHttpResponse, PushPlusError> {
        Ok(PushPlusHttpResponse {
            status_code: 200,
            body,
        })
    }

    fn message() -> PushPlusMessage {
        PushPlusMessage {
            token: "token".to_string(),
            title: "Title".to_string(),
            content: "Content".to_string(),
            channel: "wechat".to_string(),
            template: "markdown".to_string(),
            topic: Some("topic".to_string()),
            option: Some("option".to_string()),
            to: None,
        }
    }

    #[test]
    fn message_debug_redacts_token() {
        let mut message = message();
        message.token = "pushplus-secret".to_string();

        let debug = format!("{message:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("pushplus-secret"));
    }

    #[test]
    fn request_debug_redacts_token_bearing_body() {
        let request = PushPlusHttpRequest {
            url: PUSHPLUS_ENDPOINT.to_string(),
            body: json!({"token": "request-secret", "content": "message"}),
        };

        let debug = format!("{request:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("request-secret"));
    }
}
