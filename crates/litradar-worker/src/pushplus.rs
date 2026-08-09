//! PushPlus delivery client for notification workers.

use std::error::Error;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::delivery::{DeliveryExecutionControl, DeliveryExecutionControlError};
use crate::http::{redacted_url, BoundedHttpClient, OutboundHttpError};
use crate::retry::{bounded_retry_attempts, retry_delay};

/// PushPlus send endpoint.
pub const PUSHPLUS_ENDPOINT: &str = "https://www.pushplus.plus/send";

/// Error returned by PushPlus delivery clients.
#[derive(Debug, Clone, PartialEq)]
pub enum PushPlusError {
    /// Connection establishment failed before a response was received.
    ConnectFailed,
    /// The individual HTTP request timed out.
    TimedOut,
    /// HTTP transport failed before a response payload was available.
    Transport(String),
    /// Durable job cancellation or deadline stopped execution.
    Control(DeliveryExecutionControlError),
    /// Upstream returned a non-success HTTP status.
    HttpStatus {
        /// HTTP status code.
        status_code: u16,
        /// Safe upstream request identifier when supplied.
        request_id: Option<String>,
        /// Numeric Retry-After delay when supplied.
        retry_after_seconds: Option<u64>,
    },
    /// PushPlus returned an application-level failure.
    Api {
        /// PushPlus response code.
        code: Option<i64>,
    },
    /// PushPlus response could not be parsed.
    InvalidResponse(String),
}

impl fmt::Display for PushPlusError {
    /// Format the PushPlus error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectFailed => formatter.write_str("PushPlus connection failed"),
            Self::TimedOut => formatter.write_str("PushPlus request timed out"),
            Self::Transport(message) => formatter.write_str(message),
            Self::Control(error) => write!(formatter, "{error}"),
            Self::HttpStatus {
                status_code,
                request_id: Some(request_id),
                ..
            } => write!(
                formatter,
                "PushPlus request failed with HTTP {status_code} (request ID: {request_id})"
            ),
            Self::HttpStatus {
                status_code,
                request_id: None,
                ..
            } => write!(formatter, "PushPlus request failed with HTTP {status_code}"),
            Self::Api { code } => write!(formatter, "PushPlus failed with code {code:?}"),
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
    /// Format a message without exposing tokens or user-controlled content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushPlusMessage")
            .field("token", &"[REDACTED]")
            .field("title_bytes", &self.title.len())
            .field("content_bytes", &self.content.len())
            .field("channel", &self.channel)
            .field("template", &self.template)
            .field("topic", &self.topic.as_ref().map(|_| "[REDACTED]"))
            .field("option", &self.option.as_ref().map(|_| "[REDACTED]"))
            .field("to", &self.to.as_ref().map(|_| "[REDACTED]"))
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
    /// Optional total-job-capped timeout for this attempt.
    pub timeout: Option<Duration>,
}

impl fmt::Debug for PushPlusHttpRequest {
    /// Format a PushPlus request without exposing token-bearing body data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushPlusHttpRequest")
            .field("endpoint", &redacted_url(&self.url))
            .field("payload_bytes", &self.body.to_string().len())
            .field("timeout", &self.timeout)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// HTTP response returned by a PushPlus transport.
#[derive(Clone, PartialEq)]
pub struct PushPlusHttpResponse {
    /// HTTP status code.
    pub status_code: u16,
    /// Safe upstream request identifier when supplied.
    pub request_id: Option<String>,
    /// Numeric Retry-After delay when supplied.
    pub retry_after_seconds: Option<u64>,
    /// JSON response body.
    pub body: Value,
}

impl fmt::Debug for PushPlusHttpResponse {
    /// Format a PushPlus response without exposing upstream content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushPlusHttpResponse")
            .field("status_code", &self.status_code)
            .field("request_id", &self.request_id)
            .field("retry_after_seconds", &self.retry_after_seconds)
            .field("body", &"[REDACTED]")
            .finish()
    }
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
    client: BoundedHttpClient,
    default_timeout: Duration,
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
        Ok(Self {
            client: BoundedHttpClient::new(timeout_seconds),
            default_timeout: Duration::from_secs(timeout_seconds.max(1)),
        })
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
            .post_json_with_timeout(
                &request.url,
                &[],
                &request.body,
                request.timeout.unwrap_or(self.default_timeout),
            )
            .map_err(pushplus_transport_error)?;
        Ok(PushPlusHttpResponse {
            status_code: response.status_code,
            request_id: response.request_id,
            retry_after_seconds: response.retry_after_seconds,
            body: response.body.unwrap_or(Value::Null),
        })
    }
}

/// PushPlus delivery client.
pub struct PushPlusClient<T: PushPlusTransport> {
    transport: T,
    retry_attempts: usize,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
    execution_control: Option<DeliveryExecutionControl>,
    default_timeout: Duration,
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
        Self::new_with_control(transport, retry_attempts, Duration::from_secs(60), None)
    }

    /// Build a PushPlus client sharing an optional durable execution boundary.
    ///
    /// # Arguments
    ///
    /// * `transport` - HTTP transport implementation.
    /// * `retry_attempts` - Retry attempts.
    /// * `default_timeout` - Normal timeout before the total deadline cap.
    /// * `execution_control` - Optional durable job control.
    ///
    /// # Returns
    ///
    /// PushPlus client.
    pub fn new_with_control(
        transport: T,
        retry_attempts: usize,
        default_timeout: Duration,
        execution_control: Option<DeliveryExecutionControl>,
    ) -> Self {
        Self {
            transport,
            retry_attempts: bounded_retry_attempts(retry_attempts),
            sleep: Box::new(thread::sleep),
            execution_control,
            default_timeout: default_timeout.max(Duration::from_millis(1)),
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
            let timeout = self
                .execution_control
                .as_ref()
                .map(|control| control.begin_external_request(self.default_timeout))
                .transpose()
                .map_err(PushPlusError::Control)?;
            let request = PushPlusHttpRequest {
                url: PUSHPLUS_ENDPOINT.to_string(),
                body: pushplus_body(message),
                timeout,
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
                    let should_retry = can_retry && is_retryable_pushplus_error(&error);
                    emit_pushplus_request_failure(
                        &error,
                        attempt + 1,
                        should_retry,
                        attempt_started_at,
                    );
                    last_error = error;
                    if should_retry {
                        let delay = retry_delay(attempt, None);
                        if let Some(control) = self.execution_control.as_ref() {
                            control.wait(delay).map_err(PushPlusError::Control)?;
                        } else {
                            (self.sleep)(delay);
                        }
                        continue;
                    }
                    break;
                }
            }
        }
        Err(last_error)
    }

    fn send_once(
        &mut self,
        request: PushPlusHttpRequest,
    ) -> Result<PushPlusSendResponse, PushPlusError> {
        let response = self.transport.post_json(request)?;
        if !(200..300).contains(&response.status_code) {
            return Err(PushPlusError::HttpStatus {
                status_code: response.status_code,
                request_id: response.request_id,
                retry_after_seconds: response.retry_after_seconds,
            });
        }
        let object = response.body.as_object().ok_or_else(|| {
            PushPlusError::InvalidResponse("PushPlus response is not a JSON object".into())
        })?;
        let code = object.get("code").and_then(json_i64);
        if code != Some(200) {
            return Err(PushPlusError::Api { code });
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
    live_pushplus_client_with_control(timeout_seconds, retry_attempts, None)
}

/// Build a live PushPlus client sharing an optional durable execution boundary.
///
/// # Arguments
///
/// * `timeout_seconds` - Normal per-request timeout in seconds.
/// * `retry_attempts` - Retry attempts.
/// * `execution_control` - Optional total deadline and cancellation probe.
///
/// # Returns
///
/// Live PushPlus client.
pub fn live_pushplus_client_with_control(
    timeout_seconds: u64,
    retry_attempts: usize,
    execution_control: Option<DeliveryExecutionControl>,
) -> Result<PushPlusClient<ReqwestPushPlusTransport>, PushPlusError> {
    Ok(PushPlusClient::new_with_control(
        ReqwestPushPlusTransport::new(timeout_seconds)?,
        retry_attempts,
        Duration::from_secs(timeout_seconds.max(1)),
        execution_control,
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
        PushPlusError::ConnectFailed => "connect_failed",
        PushPlusError::TimedOut => "timeout",
        PushPlusError::Transport(_) => "transport",
        PushPlusError::Control(error) => error.as_str(),
        PushPlusError::HttpStatus { .. } => "http_status",
        PushPlusError::Api { .. } => "api_error",
        PushPlusError::InvalidResponse(_) => "invalid_response",
    }
}

fn pushplus_transport_error(error: OutboundHttpError) -> PushPlusError {
    match error {
        OutboundHttpError::ConnectFailed => PushPlusError::ConnectFailed,
        OutboundHttpError::TimedOut => PushPlusError::TimedOut,
        error => PushPlusError::Transport(error.to_string()),
    }
}

fn is_retryable_pushplus_error(error: &PushPlusError) -> bool {
    matches!(error, PushPlusError::ConnectFailed)
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
    fn send_does_not_retry_after_any_http_response() {
        for status_code in [400_u16, 401, 403, 429, 500, 502, 503, 504] {
            let responses = vec![
                Ok(PushPlusHttpResponse {
                    status_code,
                    request_id: None,
                    retry_after_seconds: Some(1),
                    body: json!({"error": "redacted"}),
                }),
                ok_response(json!({"code": 200, "data": "must-not-send"})),
            ];
            let mut client = PushPlusClient::new(FixturePushPlusTransport::new(responses), 10)
                .with_sleep(|_| {});

            let error = client
                .send(&message())
                .expect_err("an HTTP response must end the sending attempt");

            assert!(matches!(
                error,
                PushPlusError::HttpStatus {
                    status_code: actual,
                    ..
                } if actual == status_code
            ));
            assert_eq!(client.transport().requests.len(), 1);
        }
    }

    #[test]
    fn send_retries_connection_failure_before_request_delivery() {
        let responses = vec![
            Err(PushPlusError::ConnectFailed),
            ok_response(json!({"code": 200, "data": "msg-retried"})),
        ];
        let mut client =
            PushPlusClient::new(FixturePushPlusTransport::new(responses), 1).with_sleep(|_| {});

        assert_eq!(
            client
                .send(&message())
                .expect("connection establishment failure should retry"),
            "msg-retried"
        );
        assert_eq!(client.transport().requests.len(), 2);
    }

    #[test]
    fn send_does_not_retry_timeout_after_request_started() {
        let responses = vec![
            Err(PushPlusError::TimedOut),
            ok_response(json!({"code": 200, "data": "must-not-send"})),
        ];
        let mut client =
            PushPlusClient::new(FixturePushPlusTransport::new(responses), 10).with_sleep(|_| {});

        let error = client
            .send(&message())
            .expect_err("request timeout must remain an ambiguous single attempt");

        assert_eq!(error, PushPlusError::TimedOut);
        assert_eq!(client.transport().requests.len(), 1);
    }

    #[test]
    fn send_does_not_wait_for_retry_after_after_request_started() {
        let responses = vec![
            Ok(PushPlusHttpResponse {
                status_code: 503,
                request_id: None,
                retry_after_seconds: Some(300),
                body: json!({"error": "busy"}),
            }),
            ok_response(json!({"code": 200, "data": "msg-after-delay"})),
        ];
        let delays = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_delays = delays.clone();
        let mut client = PushPlusClient::new(FixturePushPlusTransport::new(responses), 1)
            .with_sleep(move |delay| {
                captured_delays
                    .lock()
                    .expect("retry delay lock should not be poisoned")
                    .push(delay);
            });

        let error = client
            .send(&message())
            .expect_err("HTTP response must not schedule another PushPlus request");

        assert!(matches!(
            error,
            PushPlusError::HttpStatus {
                status_code: 503,
                ..
            }
        ));
        assert!(delays
            .lock()
            .expect("retry delay lock should not be poisoned")
            .is_empty());
        assert_eq!(client.transport().requests.len(), 1);
    }

    #[test]
    fn pushplus_attempt_events_omit_token_message_and_response_material() {
        let sentinel = "pushplus-token-message-response-sentinel";
        let responses = vec![
            Ok(PushPlusHttpResponse {
                status_code: 503,
                request_id: Some("request-456".to_string()),
                retry_after_seconds: Some(2),
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

        let error = logs
            .capture(|| client.send(&message))
            .expect_err("ambiguous HTTP failure must stop the sending attempt");

        assert!(matches!(
            error,
            PushPlusError::HttpStatus {
                status_code: 503,
                ..
            }
        ));
        let events = logs.events();
        let failed = events
            .iter()
            .find(|event| event["event"] == "pushplus.request.failed")
            .expect("failed attempt should be logged");
        assert_eq!(failed["attempt"], 1);
        assert_eq!(failed["http_status"], 503);
        assert_eq!(failed["will_retry"], false);
        assert_eq!(failed["span"]["endpoint"], "send");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "pushplus.delivery.failed")
                .count(),
            1,
            "{}",
            logs.text()
        );
        assert_eq!(client.transport().requests.len(), 1);
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

        assert!(error.to_string().contains("PushPlus failed with code"));
        assert!(!error.to_string().contains("bad token"));
    }

    fn ok_response(body: Value) -> Result<PushPlusHttpResponse, PushPlusError> {
        Ok(PushPlusHttpResponse {
            status_code: 200,
            request_id: None,
            retry_after_seconds: None,
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
    fn message_debug_redacts_token_and_user_content() {
        let mut message = message();
        message.token = "pushplus-secret".to_string();
        message.title = "sensitive-title".to_string();
        message.content = "sensitive-content".to_string();

        let debug = format!("{message:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("pushplus-secret"));
        assert!(!debug.contains("sensitive-title"));
        assert!(!debug.contains("sensitive-content"));
    }

    #[test]
    fn request_debug_redacts_token_bearing_body() {
        let request = PushPlusHttpRequest {
            url: PUSHPLUS_ENDPOINT.to_string(),
            body: json!({"token": "request-secret", "content": "message"}),
            timeout: None,
        };

        let debug = format!("{request:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("request-secret"));
    }

    #[test]
    fn response_debug_redacts_upstream_content() {
        let response = PushPlusHttpResponse {
            status_code: 200,
            request_id: Some("request-456".to_string()),
            retry_after_seconds: None,
            body: json!({"msg": "response-sentinel"}),
        };

        let debug = format!("{response:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("response-sentinel"));
    }
}
