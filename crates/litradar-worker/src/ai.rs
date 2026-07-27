//! OpenAI-compatible AI selection client for delivery workers.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use litradar_domain::{
    ArticleCandidateInfo, NotificationSubscriberInfo, RankedSelectionInfo, SelectionResultInfo,
};
use litradar_recommend::{
    extract_response_payload, AiPayloadKind, AiRuntimeConfig, NotificationDefaults,
};
use reqwest::Url;
use serde_json::{json, Value};

use crate::delivery::{DeliveryExecutionControl, DeliveryExecutionControlError};
use crate::http::{redacted_url, BoundedHttpClient, OutboundHttpError};
use crate::retry::{bounded_retry_attempts, retry_delay};

const CHAT_COMPLETIONS_PATH: &str = "chat/completions";
const HTTP_REFERER: &str = "https://github.com/openai/codex";
const X_TITLE: &str = "LitRadar";
const SUMMARY_PROMPT_SUFFIX: &str = "Only summarize the supplied selected papers. Focus on major research themes, methods, and findings.";
const SELECTION_OUTPUT_CONTRACT: &str = "Return exactly one JSON object with keys \"summary\" and \"selected\". \"selected\" must be an array of objects that each contain \"article_id\" and \"score\". Do not wrap JSON in markdown fences.";
const SUMMARY_OUTPUT_CONTRACT: &str =
    "Return exactly one JSON object with the key \"summary\". Do not wrap JSON in markdown fences.";

/// Default system prompt used by the Python notification selector.
pub const DEFAULT_SELECTION_SYSTEM_PROMPT: &str = "You are a precise academic recommender. Use two-stage selection: directions-first filtering, then keyword-based ranking in the filtered set. Return relevant candidates ranked by score. Order selected items from highest to lowest. Judge by article content quality and topic relevance only. Ignore journal quality, prestige, and ranking completely. Do not invent article ids.";

/// Error returned by AI delivery clients.
#[derive(Debug, Clone, PartialEq)]
pub enum AiClientError {
    /// Connection establishment failed before a response was received.
    ConnectFailed,
    /// The individual HTTP request timed out.
    TimedOut,
    /// HTTP transport failed before a response payload was available.
    Transport(String),
    /// Durable job cancellation, deadline, or request budget stopped execution.
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
    /// Upstream response could not be parsed into the expected payload.
    InvalidResponse(String),
}

impl fmt::Display for AiClientError {
    /// Format the AI client error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectFailed => formatter.write_str("AI endpoint connection failed"),
            Self::TimedOut => formatter.write_str("AI request timed out"),
            Self::Transport(message) => formatter.write_str(message),
            Self::Control(error) => write!(formatter, "{error}"),
            Self::HttpStatus {
                status_code,
                request_id: Some(request_id),
                ..
            } => write!(
                formatter,
                "AI request failed with HTTP {status_code} (request ID: {request_id})"
            ),
            Self::HttpStatus {
                status_code,
                request_id: None,
                ..
            } => write!(formatter, "AI request failed with HTTP {status_code}"),
            Self::InvalidResponse(message) => formatter.write_str(message),
        }
    }
}

impl Error for AiClientError {}

/// HTTP header sent by an AI transport.
#[derive(Clone, PartialEq, Eq)]
pub struct AiHttpHeader {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
}

impl fmt::Debug for AiHttpHeader {
    /// Format an HTTP header without exposing its value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiHttpHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// HTTP request sent to an OpenAI-compatible endpoint.
#[derive(Clone, PartialEq)]
pub struct AiHttpRequest {
    /// Request URL.
    pub url: String,
    /// Request headers.
    pub headers: Vec<AiHttpHeader>,
    /// JSON request body.
    pub body: Value,
    /// Optional total-job-capped timeout for this attempt.
    pub timeout: Option<Duration>,
}

impl fmt::Debug for AiHttpRequest {
    /// Format an AI request without exposing prompts, articles, or credentials.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiHttpRequest")
            .field("endpoint", &redacted_url(&self.url))
            .field("headers", &self.headers)
            .field("payload_bytes", &self.body.to_string().len())
            .field("timeout", &self.timeout)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// HTTP response returned by an AI transport.
#[derive(Clone, PartialEq)]
pub struct AiHttpResponse {
    /// HTTP status code.
    pub status_code: u16,
    /// Safe upstream request identifier when supplied.
    pub request_id: Option<String>,
    /// Numeric Retry-After delay when supplied.
    pub retry_after_seconds: Option<u64>,
    /// JSON response body.
    pub body: Value,
}

impl fmt::Debug for AiHttpResponse {
    /// Format an AI response without exposing model output.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiHttpResponse")
            .field("status_code", &self.status_code)
            .field("request_id", &self.request_id)
            .field("retry_after_seconds", &self.retry_after_seconds)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

struct AiCompletionResponse {
    status_code: u16,
    payload: Value,
}

struct ResponseFormatVariant {
    kind: &'static str,
    value: Option<Value>,
}

/// Transport boundary for OpenAI-compatible HTTP calls.
pub trait AiTransport {
    /// Send one JSON POST request.
    ///
    /// # Arguments
    ///
    /// * `request` - HTTP request payload.
    ///
    /// # Returns
    ///
    /// HTTP response payload.
    fn post_json(&mut self, request: AiHttpRequest) -> Result<AiHttpResponse, AiClientError>;
}

/// Reqwest-backed AI transport.
#[derive(Clone)]
pub struct ReqwestAiTransport {
    client: BoundedHttpClient,
    default_timeout: Duration,
    auth_db_path: PathBuf,
}

impl fmt::Debug for ReqwestAiTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReqwestAiTransport")
            .field("client", &self.client)
            .field("default_timeout", &self.default_timeout)
            .field("endpoint_catalog", &"[CONFIGURED]")
            .finish()
    }
}

impl ReqwestAiTransport {
    /// Build a reqwest-backed AI transport.
    ///
    /// # Arguments
    ///
    /// * `timeout_seconds` - Request timeout in seconds.
    /// * `auth_db_path` - Database containing the current AI endpoint catalog.
    ///
    /// # Returns
    ///
    /// Reqwest AI transport.
    pub fn new(
        timeout_seconds: u64,
        auth_db_path: impl Into<PathBuf>,
    ) -> Result<Self, AiClientError> {
        Ok(Self {
            client: BoundedHttpClient::new(timeout_seconds),
            default_timeout: Duration::from_secs(timeout_seconds.max(1)),
            auth_db_path: auth_db_path.into(),
        })
    }

    fn ensure_request_is_allowed(&self, request_url: &str) -> Result<(), AiClientError> {
        ensure_ai_request_is_allowed(&self.auth_db_path, request_url)
    }
}

impl AiTransport for ReqwestAiTransport {
    /// Send one JSON POST request through reqwest.
    fn post_json(&mut self, request: AiHttpRequest) -> Result<AiHttpResponse, AiClientError> {
        self.ensure_request_is_allowed(&request.url)?;
        let headers = request
            .headers
            .iter()
            .map(|header| (header.name.clone(), header.value.clone()))
            .collect::<Vec<_>>();
        let response = self
            .client
            .post_json_with_timeout(
                &request.url,
                &headers,
                &request.body,
                request.timeout.unwrap_or(self.default_timeout),
            )
            .map_err(ai_transport_error)?;
        Ok(AiHttpResponse {
            status_code: response.status_code,
            request_id: response.request_id,
            retry_after_seconds: response.retry_after_seconds,
            body: response.body.unwrap_or(Value::Null),
        })
    }
}

/// OpenAI-compatible completion client.
pub struct AiCompletionClient<T: AiTransport> {
    transport: T,
    retry_attempts: usize,
    temperature: f64,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
    execution_control: Option<DeliveryExecutionControl>,
    default_timeout: Duration,
}

impl<T: AiTransport> AiCompletionClient<T> {
    /// Build an AI completion client.
    ///
    /// # Arguments
    ///
    /// * `transport` - HTTP transport implementation.
    /// * `retry_attempts` - Retry attempts per response format variant.
    /// * `temperature` - Model temperature.
    ///
    /// # Returns
    ///
    /// Completion client.
    pub fn new(transport: T, retry_attempts: usize, temperature: f64) -> Self {
        Self::new_with_control(
            transport,
            retry_attempts,
            temperature,
            Duration::from_secs(60),
            None,
        )
    }

    /// Build an AI client sharing one durable job deadline, cancellation probe, and budget.
    ///
    /// # Arguments
    ///
    /// * `transport` - HTTP transport implementation.
    /// * `retry_attempts` - Retry attempts per response format variant.
    /// * `temperature` - Model temperature.
    /// * `default_timeout` - Normal timeout before the job deadline cap.
    /// * `execution_control` - Optional durable job control.
    ///
    /// # Returns
    ///
    /// Completion client.
    pub fn new_with_control(
        transport: T,
        retry_attempts: usize,
        temperature: f64,
        default_timeout: Duration,
        execution_control: Option<DeliveryExecutionControl>,
    ) -> Self {
        Self {
            transport,
            retry_attempts: bounded_retry_attempts(retry_attempts),
            temperature,
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
    /// Completion client with the replacement callback.
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

    /// Select articles through an OpenAI-compatible endpoint.
    ///
    /// # Arguments
    ///
    /// * `config` - AI runtime configuration.
    /// * `subscriber` - Subscriber settings.
    /// * `defaults` - Notification defaults.
    /// * `candidates` - Candidate articles sent to the model.
    ///
    /// # Returns
    ///
    /// Structured model selection.
    pub fn select_articles(
        &mut self,
        config: &AiRuntimeConfig,
        subscriber: &NotificationSubscriberInfo,
        defaults: &NotificationDefaults,
        candidates: &[ArticleCandidateInfo],
    ) -> Result<SelectionResultInfo, AiClientError> {
        let schema = json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string" },
                "selected": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "article_id": { "type": "integer" },
                            "score": { "type": "number" }
                        },
                        "required": ["article_id", "score"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["summary", "selected"],
            "additionalProperties": false
        });
        let user_payload = json!({
            "subscriber": {
                "id": subscriber.subscriber_id,
                "name": subscriber.name,
                "keywords": subscriber.keywords,
                "directions": subscriber.directions
            },
            "summary_requirement": "Summary must focus on the content of selected papers. Describe major research themes, methods, or findings in 2-4 sentences. Avoid generic recommendation language.",
            "selection_rules": {
                "goal": "Return ranked relevant candidates for this subscriber",
                "score_definition": "0 to 100, higher means better match and quality",
                "priority_order": [
                    "First pass: directions-first filtering. When directions are provided, only keep candidates that clearly match at least one direction.",
                    "Second pass: within the direction-matched subset, rank by keyword relevance.",
                    "Third pass: break ties by methodological rigor, recency, and practical or theoretical contribution."
                ],
                "must_follow": [
                    "Directions have higher priority than keywords. Do not elevate a keyword-only paper over a weaker direction-matched paper.",
                    "If directions are non-empty and at least one candidate matches directions, do not select direction-mismatched papers.",
                    "If directions are empty or no candidate matches directions, fallback to keyword relevance."
                ],
                "prefer": [
                    "Article quality and methodological rigor",
                    "Recent papers",
                    "High conceptual overlap with subscriber goals",
                    "Clear practical or theoretical contribution"
                ],
                "avoid": [
                    "Low topical relevance",
                    "Any preference based on journal prestige or ranking"
                ]
            },
            "limits": {
                "max_candidates_input": defaults.max_candidates
            },
            "candidates": candidates.iter().map(selection_candidate_payload).collect::<Vec<_>>(),
            "output_instruction": "Return JSON only and strictly follow schema."
        });
        let response_payload = self.create_completion(
            config,
            "paper_selection",
            schema,
            vec![
                ai_message("system", &selection_system_prompt(config)),
                ai_message("user", &user_payload.to_string()),
            ],
            AiPayloadKind::Selection,
        )?;
        let mut selections = response_payload
            .get("selected")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(RankedSelectionInfo {
                            article_id: json_i64(item.get("article_id")?)?,
                            score: json_f64(item.get("score")).unwrap_or(0.0),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        selections.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(SelectionResultInfo {
            summary: response_payload
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            selections,
        })
    }

    /// Summarize selected articles through an OpenAI-compatible endpoint.
    ///
    /// # Arguments
    ///
    /// * `config` - AI runtime configuration.
    /// * `subscriber` - Subscriber settings.
    /// * `selected_candidates` - Final selected article candidates.
    ///
    /// # Returns
    ///
    /// Summary text.
    pub fn summarize_selected_articles(
        &mut self,
        config: &AiRuntimeConfig,
        subscriber: &NotificationSubscriberInfo,
        selected_candidates: &[ArticleCandidateInfo],
    ) -> Result<String, AiClientError> {
        if selected_candidates.is_empty() {
            return Ok(String::new());
        }
        let schema = json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string" }
            },
            "required": ["summary"],
            "additionalProperties": false
        });
        let user_payload = json!({
            "subscriber": {
                "id": subscriber.subscriber_id,
                "name": subscriber.name,
                "keywords": subscriber.keywords,
                "directions": subscriber.directions
            },
            "selected_articles": selected_candidates.iter().map(summary_candidate_payload).collect::<Vec<_>>(),
            "instruction": "Summarize the content of these selected papers in 2-4 sentences. Focus on major research themes, methods, and findings. Avoid generic recommendation language."
        });
        let response_payload = self.create_completion(
            config,
            "selected_paper_summary",
            schema,
            vec![
                ai_message("system", &summary_system_prompt(config)),
                ai_message("user", &user_payload.to_string()),
            ],
            AiPayloadKind::Summary,
        )?;
        Ok(response_payload
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string())
    }

    fn create_completion(
        &mut self,
        config: &AiRuntimeConfig,
        schema_name: &str,
        schema: Value,
        messages: Vec<Value>,
        payload_kind: AiPayloadKind,
    ) -> Result<Value, AiClientError> {
        let started_at = Instant::now();
        let operation = ai_payload_kind(payload_kind);
        let completion_span = tracing::info_span!(
            "ai.completion",
            component = "delivery",
            provider = "openai_compatible",
            endpoint = "chat_completions",
            operation,
        );
        completion_span.in_scope(|| {
            tracing::info!(
                event = "ai.completion.started",
                component = "delivery",
                outcome = "started",
            );
            let result = self.create_completion_attempts(
                config,
                schema_name,
                schema,
                messages,
                payload_kind,
            );
            match &result {
                Ok(_) => tracing::info!(
                    event = "ai.completion.completed",
                    component = "delivery",
                    outcome = "success",
                    duration_ms = elapsed_millis(started_at),
                ),
                Err(error) => tracing::warn!(
                    event = "ai.completion.failed",
                    component = "delivery",
                    outcome = "failure",
                    error_kind = ai_error_kind(error),
                    duration_ms = elapsed_millis(started_at),
                ),
            }
            result
        })
    }

    fn create_completion_attempts(
        &mut self,
        config: &AiRuntimeConfig,
        schema_name: &str,
        schema: Value,
        messages: Vec<Value>,
        payload_kind: AiPayloadKind,
    ) -> Result<Value, AiClientError> {
        let mut last_error = AiClientError::InvalidResponse("AI request was not attempted".into());
        let response_formats = response_format_variants(schema_name, &schema);
        let completion_url = chat_completions_url(&config.base_url)?;
        for (format_index, response_format) in response_formats.iter().enumerate() {
            if format_index > 0 {
                tracing::warn!(
                    event = "ai.response_format.fallback",
                    component = "delivery",
                    outcome = "fallback",
                    from_format = response_formats[format_index - 1].kind,
                    to_format = response_format.kind,
                );
            }
            for attempt in 0..=self.retry_attempts {
                let timeout = self
                    .execution_control
                    .as_ref()
                    .map(|control| control.begin_ai_request(self.default_timeout))
                    .transpose()
                    .map_err(AiClientError::Control)?;
                let body = completion_body(
                    config,
                    self.temperature,
                    &messages,
                    response_format.value.clone(),
                );
                let request = AiHttpRequest {
                    url: completion_url.clone(),
                    headers: ai_headers(&config.api_key),
                    body,
                    timeout,
                };
                let attempt_started_at = Instant::now();
                match self.send_completion(request, payload_kind) {
                    Ok(response) => {
                        tracing::info!(
                            event = "ai.request.completed",
                            component = "delivery",
                            outcome = "success",
                            response_format = response_format.kind,
                            attempt = attempt + 1,
                            http_status = response.status_code,
                            duration_ms = elapsed_millis(attempt_started_at),
                        );
                        return Ok(response.payload);
                    }
                    Err(error) => {
                        let will_retry =
                            attempt < self.retry_attempts && is_retryable_ai_error(&error);
                        let will_fallback = !will_retry
                            && is_format_compatibility_error(&error)
                            && format_index + 1 < response_formats.len();
                        emit_ai_request_failure(
                            &error,
                            response_format.kind,
                            attempt + 1,
                            will_retry,
                            will_fallback,
                            attempt_started_at,
                        );
                        let retry_after_seconds = ai_retry_after_seconds(&error);
                        last_error = error;
                        if will_retry {
                            let delay = retry_delay(attempt, retry_after_seconds);
                            if let Some(control) = self.execution_control.as_ref() {
                                control.wait(delay).map_err(AiClientError::Control)?;
                            } else {
                                (self.sleep)(delay);
                            }
                            continue;
                        }
                        if !will_fallback {
                            return Err(last_error);
                        }
                        break;
                    }
                }
            }
        }
        Err(last_error)
    }

    fn send_completion(
        &mut self,
        request: AiHttpRequest,
        payload_kind: AiPayloadKind,
    ) -> Result<AiCompletionResponse, AiClientError> {
        let response = self.transport.post_json(request)?;
        if !(200..300).contains(&response.status_code) {
            return Err(AiClientError::HttpStatus {
                status_code: response.status_code,
                request_id: response.request_id,
                retry_after_seconds: response.retry_after_seconds,
            });
        }
        let payload = extract_response_payload(&response.body, payload_kind)
            .map_err(|error| AiClientError::InvalidResponse(error.to_string()))?;
        Ok(AiCompletionResponse {
            status_code: response.status_code,
            payload,
        })
    }
}

/// Build a live OpenAI-compatible completion client.
///
/// # Arguments
///
/// * `timeout_seconds` - Request timeout in seconds.
/// * `retry_attempts` - Retry attempts per response format variant.
/// * `temperature` - Model temperature.
/// * `auth_db_path` - Database containing the current AI endpoint catalog.
///
/// # Returns
///
/// Live completion client.
pub fn live_ai_client(
    timeout_seconds: u64,
    retry_attempts: usize,
    temperature: f64,
    auth_db_path: impl AsRef<Path>,
) -> Result<AiCompletionClient<ReqwestAiTransport>, AiClientError> {
    live_ai_client_with_control(
        timeout_seconds,
        retry_attempts,
        temperature,
        auth_db_path,
        None,
    )
}

/// Build a live AI client sharing an optional durable job execution boundary.
///
/// # Arguments
///
/// * `timeout_seconds` - Normal per-request timeout in seconds.
/// * `retry_attempts` - Retry attempts per response format variant.
/// * `temperature` - Model temperature.
/// * `auth_db_path` - Database containing the current AI endpoint catalog.
/// * `execution_control` - Optional total deadline, cancellation probe, and AI budget.
///
/// # Returns
///
/// Live completion client.
pub fn live_ai_client_with_control(
    timeout_seconds: u64,
    retry_attempts: usize,
    temperature: f64,
    auth_db_path: impl AsRef<Path>,
    execution_control: Option<DeliveryExecutionControl>,
) -> Result<AiCompletionClient<ReqwestAiTransport>, AiClientError> {
    Ok(AiCompletionClient::new_with_control(
        ReqwestAiTransport::new(timeout_seconds, auth_db_path.as_ref().to_path_buf())?,
        retry_attempts,
        temperature,
        Duration::from_secs(timeout_seconds.max(1)),
        execution_control,
    ))
}

fn selection_system_prompt(config: &AiRuntimeConfig) -> String {
    let base_prompt = if config.system_prompt.trim().is_empty() {
        DEFAULT_SELECTION_SYSTEM_PROMPT
    } else {
        config.system_prompt.trim()
    };
    format!("{base_prompt}\n\n{SELECTION_OUTPUT_CONTRACT}")
}

fn summary_system_prompt(config: &AiRuntimeConfig) -> String {
    if config.system_prompt.trim().is_empty() {
        format!("You are a precise academic summarizer. Only summarize the supplied selected papers. {SUMMARY_OUTPUT_CONTRACT}")
    } else {
        format!(
            "{}\n\n{SUMMARY_PROMPT_SUFFIX}\n\n{SUMMARY_OUTPUT_CONTRACT}",
            config.system_prompt.trim()
        )
    }
}

fn ai_message(role: &str, content: &str) -> Value {
    json!({
        "role": role,
        "content": content
    })
}

fn completion_body(
    config: &AiRuntimeConfig,
    temperature: f64,
    messages: &[Value],
    response_format: Option<Value>,
) -> Value {
    let mut body = json!({
        "model": config.model,
        "temperature": temperature,
        "messages": messages
    });
    if let Some(response_format) = response_format {
        body.as_object_mut()
            .expect("completion body should be an object")
            .insert("response_format".into(), response_format);
    }
    body
}

fn response_format_variants(schema_name: &str, schema: &Value) -> Vec<ResponseFormatVariant> {
    vec![
        ResponseFormatVariant {
            kind: "json_schema",
            value: Some(json!({
                "type": "json_schema",
                "json_schema": {
                    "name": schema_name,
                    "strict": true,
                    "schema": schema
                }
            })),
        },
        ResponseFormatVariant {
            kind: "json_object",
            value: Some(json!({ "type": "json_object" })),
        },
        ResponseFormatVariant {
            kind: "plain_json",
            value: None,
        },
    ]
}

fn emit_ai_request_failure(
    error: &AiClientError,
    response_format: &str,
    attempt: usize,
    will_retry: bool,
    will_fallback: bool,
    started_at: Instant,
) {
    let duration_ms = elapsed_millis(started_at);
    match error {
        AiClientError::HttpStatus { status_code, .. } => tracing::warn!(
            event = "ai.request.failed",
            component = "delivery",
            outcome = "failure",
            response_format,
            attempt,
            error_kind = "http_status",
            http_status = status_code,
            will_retry,
            will_fallback,
            duration_ms,
        ),
        _ => tracing::warn!(
            event = "ai.request.failed",
            component = "delivery",
            outcome = "failure",
            response_format,
            attempt,
            error_kind = ai_error_kind(error),
            will_retry,
            will_fallback,
            duration_ms,
        ),
    }
}

fn ai_error_kind(error: &AiClientError) -> &'static str {
    match error {
        AiClientError::ConnectFailed => "connect_failed",
        AiClientError::TimedOut => "timeout",
        AiClientError::Transport(_) => "transport",
        AiClientError::Control(error) => error.as_str(),
        AiClientError::HttpStatus { .. } => "http_status",
        AiClientError::InvalidResponse(_) => "invalid_response",
    }
}

fn ai_transport_error(error: OutboundHttpError) -> AiClientError {
    match error {
        OutboundHttpError::ConnectFailed => AiClientError::ConnectFailed,
        OutboundHttpError::TimedOut => AiClientError::TimedOut,
        error => AiClientError::Transport(error.to_string()),
    }
}

fn is_retryable_ai_error(error: &AiClientError) -> bool {
    matches!(
        error,
        AiClientError::ConnectFailed | AiClientError::TimedOut
    ) || matches!(
        error,
        AiClientError::HttpStatus {
            status_code: 429 | 502 | 503 | 504,
            ..
        }
    )
}

fn is_format_compatibility_error(error: &AiClientError) -> bool {
    matches!(error, AiClientError::InvalidResponse(_))
}

fn ai_retry_after_seconds(error: &AiClientError) -> Option<u64> {
    match error {
        AiClientError::HttpStatus {
            retry_after_seconds,
            ..
        } => *retry_after_seconds,
        _ => None,
    }
}

fn ai_payload_kind(payload_kind: AiPayloadKind) -> &'static str {
    match payload_kind {
        AiPayloadKind::Selection => "selection",
        AiPayloadKind::Summary => "summary",
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn ensure_ai_request_is_allowed(
    auth_db_path: &Path,
    request_url: &str,
) -> Result<(), AiClientError> {
    let request_url = Url::parse(request_url)
        .map_err(|_| AiClientError::Transport("AI endpoint is not approved".to_string()))?;
    let allowed_base_urls = litradar_storage::load_ai_allowed_base_urls(auth_db_path)
        .map_err(|_| AiClientError::Transport("AI endpoint is not approved".to_string()))?;
    let is_allowed = allowed_base_urls.iter().any(|base_url| {
        Url::parse(base_url)
            .and_then(|url| url.join(CHAT_COMPLETIONS_PATH))
            .is_ok_and(|allowed_url| allowed_url == request_url)
    });
    if is_allowed {
        Ok(())
    } else {
        Err(AiClientError::Transport(
            "AI endpoint is not approved".to_string(),
        ))
    }
}

fn chat_completions_url(base_url: &str) -> Result<String, AiClientError> {
    Url::parse(base_url)
        .and_then(|url| url.join(CHAT_COMPLETIONS_PATH))
        .map(|url| url.to_string())
        .map_err(|_| AiClientError::Transport("AI endpoint URL is invalid".to_string()))
}

fn ai_headers(api_key: &str) -> Vec<AiHttpHeader> {
    vec![
        AiHttpHeader {
            name: "Authorization".into(),
            value: format!("Bearer {api_key}"),
        },
        AiHttpHeader {
            name: "Content-Type".into(),
            value: "application/json".into(),
        },
        AiHttpHeader {
            name: "HTTP-Referer".into(),
            value: HTTP_REFERER.into(),
        },
        AiHttpHeader {
            name: "X-Title".into(),
            value: X_TITLE.into(),
        },
    ]
}

fn selection_candidate_payload(candidate: &ArticleCandidateInfo) -> Value {
    json!({
        "article_id": candidate.article_id,
        "journal_id": candidate.journal_id,
        "issue_id": candidate.issue_id,
        "title": candidate.title,
        "abstract": truncate_text(&candidate.abstract_text, 1200),
        "date": candidate.date,
        "journal_title": candidate.journal_title,
        "open_access": candidate.open_access,
        "in_press": candidate.in_press
    })
}

fn summary_candidate_payload(candidate: &ArticleCandidateInfo) -> Value {
    json!({
        "article_id": candidate.article_id,
        "title": candidate.title,
        "abstract": truncate_text(&candidate.abstract_text, 1200),
        "journal_title": candidate.journal_title,
        "date": candidate.date
    })
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_i64().map(|number| number as f64))
            .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
    })
}

#[cfg(test)]
/// Shared structured-log capture helpers for worker module tests.
pub(crate) mod test_support {
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, Once, OnceLock};

    use serde_json::Value;
    use tracing_subscriber::fmt::MakeWriter;

    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());
    static CAPTURE_BYTES: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    static CAPTURE_SUBSCRIBER: Once = Once::new();
    static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

    /// Thread-safe byte buffer used as a tracing test writer.
    #[derive(Clone)]
    pub(crate) struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
        capture_id: u64,
    }

    impl Default for CapturedLogs {
        fn default() -> Self {
            let bytes = Arc::clone(CAPTURE_BYTES.get_or_init(|| Arc::new(Mutex::new(Vec::new()))));
            CAPTURE_SUBSCRIBER.call_once(|| {
                let subscriber = tracing_subscriber::fmt()
                    .with_ansi(false)
                    .with_max_level(tracing::Level::TRACE)
                    .with_writer(CapturedSink {
                        bytes: Arc::clone(&bytes),
                    })
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .finish();
                tracing::subscriber::set_global_default(subscriber)
                    .expect("worker tests should install one global tracing subscriber");
            });
            Self {
                bytes,
                capture_id: NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed),
            }
        }
    }

    impl CapturedLogs {
        /// Run an operation inside a uniquely identifiable capture span.
        ///
        /// # Arguments
        ///
        /// * `operation` - Operation whose structured events should be captured.
        ///
        /// # Returns
        ///
        /// Operation result after synchronous event capture.
        pub(crate) fn capture<T>(&self, operation: impl FnOnce() -> T) -> T {
            let _capture_guard = CAPTURE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let capture_span = tracing::info_span!(
                "test.capture",
                component = "test",
                capture_id = self.capture_id,
            );
            capture_span.in_scope(operation)
        }

        /// Return all captured bytes as UTF-8 text.
        ///
        /// # Returns
        ///
        /// Captured JSON Lines text.
        pub(crate) fn text(&self) -> String {
            self.events()
                .into_iter()
                .map(|event| serde_json::to_string(&event).expect("event should serialize"))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// Parse captured JSON Lines into event values.
        ///
        /// # Returns
        ///
        /// Parsed event objects in emission order.
        pub(crate) fn events(&self) -> Vec<Value> {
            let text = String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured logs should be UTF-8");
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_str(line).expect("captured log should be JSON"))
                .filter(|event: &Value| {
                    event["spans"].as_array().is_some_and(|spans| {
                        spans
                            .iter()
                            .any(|span| span["capture_id"].as_u64() == Some(self.capture_id))
                    })
                })
                .collect()
        }
    }

    #[derive(Clone)]
    struct CapturedSink {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    struct CapturedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedSink {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use super::test_support::CapturedLogs;
    use super::*;

    #[derive(Debug, Default)]
    struct FixtureAiTransport {
        responses: Vec<Result<AiHttpResponse, AiClientError>>,
        requests: Vec<AiHttpRequest>,
    }

    impl FixtureAiTransport {
        fn new(responses: Vec<Result<AiHttpResponse, AiClientError>>) -> Self {
            Self {
                responses: responses.into_iter().rev().collect(),
                requests: Vec::new(),
            }
        }
    }

    impl AiTransport for FixtureAiTransport {
        fn post_json(&mut self, request: AiHttpRequest) -> Result<AiHttpResponse, AiClientError> {
            self.requests.push(request);
            self.responses
                .pop()
                .unwrap_or_else(|| Err(AiClientError::Transport("missing fixture response".into())))
        }
    }

    #[test]
    fn ai_oversized_retry_counts_are_bounded() {
        let client = AiCompletionClient::new(FixtureAiTransport::default(), usize::MAX, 0.2);

        assert_eq!(
            client.retry_attempts,
            litradar_domain::DELIVERY_RETRY_ATTEMPTS_MAX
        );
    }

    #[test]
    fn request_debug_redacts_authorization_header() {
        let request = AiHttpRequest {
            url: "https://ai.example.com/chat/completions".to_string(),
            headers: vec![AiHttpHeader {
                name: "Authorization".to_string(),
                value: "Bearer ai-request-secret".to_string(),
            }],
            body: json!({"model": "fixture-model"}),
            timeout: None,
        };

        let debug = format!("{request:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("ai-request-secret"));
        assert!(!debug.contains("fixture-model"));
    }

    #[test]
    fn response_debug_redacts_model_output() {
        let response = AiHttpResponse {
            status_code: 200,
            request_id: Some("request-123".to_string()),
            retry_after_seconds: None,
            body: json!({"choices": [{"message": {"content": "response-sentinel"}}]}),
        };

        let debug = format!("{response:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("response-sentinel"));
    }

    #[test]
    fn live_transport_reloads_endpoint_approval_before_each_attempt() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        let secret_codec = litradar_storage::SecretCodec::from_key([53_u8; 32]);
        litradar_storage::initialize_auth_database(&auth_db_path)
            .expect("auth database should initialize");
        let persist_catalog = |value: &str| {
            litradar_storage::upsert_runtime_settings(
                &auth_db_path,
                &secret_codec,
                &HashMap::from([("ai_allowed_base_urls".to_string(), Some(value.to_string()))]),
                &HashMap::new(),
            )
            .expect("AI endpoint catalog should persist");
        };
        persist_catalog("https://127.0.0.1/v1/");
        let mut transport =
            ReqwestAiTransport::new(1, auth_db_path.clone()).expect("transport should initialize");
        let request = AiHttpRequest {
            url: "https://127.0.0.1/v1/chat/completions".to_string(),
            headers: Vec::new(),
            body: json!({}),
            timeout: None,
        };

        assert_eq!(
            transport.post_json(request.clone()).unwrap_err(),
            AiClientError::Transport(
                "Outbound endpoint resolved to a disallowed address".to_string()
            )
        );
        persist_catalog("");
        assert_eq!(
            transport.post_json(request).unwrap_err(),
            AiClientError::Transport("AI endpoint is not approved".to_string())
        );

        let debug = format!("{transport:?}");
        assert!(!debug.contains("auth.sqlite"));
    }

    #[test]
    fn select_articles_sends_openai_compatible_request() {
        let response = ok_response(json!({
            "choices": [{
                "message": {
                    "content": "{\"summary\":\"matches rust\",\"selected\":[{\"article_id\":101,\"score\":91.5}]}"
                }
            }]
        }));
        let mut client = AiCompletionClient::new(FixtureAiTransport::new(vec![response]), 0, 0.2)
            .with_sleep(|_| {});
        let result = client
            .select_articles(&ai_config(), &subscriber(), &defaults(), &[candidate(101)])
            .expect("selection should succeed");

        assert_eq!(result.summary, "matches rust");
        assert_eq!(result.selections[0].article_id, 101);
        assert_eq!(result.selections[0].score, 91.5);
        let request = &client.transport().requests[0];
        assert_eq!(request.url, "https://api.test/v1/chat/completions");
        assert!(request
            .headers
            .iter()
            .any(|header| { header.name == "Authorization" && header.value == "Bearer secret" }));
        assert_eq!(request.body["model"], "model");
        assert_eq!(
            request.body["response_format"]["json_schema"]["name"],
            "paper_selection"
        );
    }

    #[test]
    fn completion_falls_back_response_format_variants() {
        let responses = vec![
            ok_response(json!({"choices": []})),
            ok_response(json!({
                "choices": [{
                    "message": {
                        "content": "{\"summary\":\"fallback\",\"selected\":[{\"article_id\":102,\"score\":70}]}"
                    }
                }]
            })),
        ];
        let mut client =
            AiCompletionClient::new(FixtureAiTransport::new(responses), 0, 0.2).with_sleep(|_| {});
        let result = client
            .select_articles(&ai_config(), &subscriber(), &defaults(), &[candidate(102)])
            .expect("json_object fallback should succeed");

        assert_eq!(result.summary, "fallback");
        assert_eq!(client.transport().requests.len(), 2);
        assert_eq!(
            client.transport().requests[0].body["response_format"]["type"],
            "json_schema"
        );
        assert_eq!(
            client.transport().requests[1].body["response_format"]["type"],
            "json_object"
        );
    }

    #[test]
    fn completion_retries_each_response_format() {
        let responses = vec![
            Err(AiClientError::ConnectFailed),
            ok_response(json!({
                "choices": [{
                    "message": {
                        "content": "{\"summary\":\"retried\",\"selected\":[{\"article_id\":103,\"score\":60}]}"
                    }
                }]
            })),
        ];
        let mut client =
            AiCompletionClient::new(FixtureAiTransport::new(responses), 1, 0.2).with_sleep(|_| {});
        let result = client
            .select_articles(&ai_config(), &subscriber(), &defaults(), &[candidate(103)])
            .expect("retry should succeed");

        assert_eq!(result.summary, "retried");
        assert_eq!(client.transport().requests.len(), 2);
        assert_eq!(
            client.transport().requests[0].body["response_format"]["type"],
            "json_schema"
        );
        assert_eq!(
            client.transport().requests[1].body["response_format"]["type"],
            "json_schema"
        );
    }

    #[test]
    fn ai_attempt_events_are_correlated_and_omit_request_and_response_material() {
        let sentinel = "ai-key-prompt-response-sentinel";
        let responses = (0..3)
            .map(|_| {
                Err(AiClientError::HttpStatus {
                    status_code: 503,
                    request_id: Some("request-123".to_string()),
                    retry_after_seconds: Some(7),
                })
            })
            .collect::<Vec<_>>();
        let mut config = ai_config();
        config.base_url = format!("https://{sentinel}.example/v1");
        config.api_key = sentinel.to_string();
        config.system_prompt = sentinel.to_string();
        let mut subscriber = subscriber();
        subscriber.name = sentinel.to_string();
        subscriber.keywords = vec![sentinel.to_string()];
        let mut article = candidate(104);
        article.title = sentinel.to_string();
        article.abstract_text = sentinel.to_string();
        let logs = CapturedLogs::default();
        let mut client =
            AiCompletionClient::new(FixtureAiTransport::new(responses), 2, 0.2).with_sleep(|_| {});

        let error = logs
            .capture(|| client.select_articles(&config, &subscriber, &defaults(), &[article]))
            .expect_err("all transient attempts should fail");

        assert!(!error.to_string().contains(sentinel));
        let events = logs.events();
        let attempts = events
            .iter()
            .filter(|event| event["event"] == "ai.request.failed")
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0]["response_format"], "json_schema");
        assert_eq!(attempts[0]["attempt"], 1);
        assert_eq!(attempts[0]["http_status"], 503);
        assert_eq!(attempts[0]["span"]["operation"], "selection");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "ai.response_format.fallback")
                .count(),
            0
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "ai.completion.failed")
                .count(),
            1
        );
        assert!(!logs.text().contains(sentinel));
    }

    #[test]
    fn retry_backoff_is_capped_for_later_attempts() {
        let mut responses = (0..10)
            .map(|_| Err(AiClientError::ConnectFailed))
            .collect::<Vec<_>>();
        responses.push(ok_response(json!({
            "choices": [{
                "message": {
                    "content": "{\"summary\":\"retried\",\"selected\":[{\"article_id\":103,\"score\":60}]}"
                }
            }]
        })));
        let delays = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_delays = delays.clone();
        let mut client = AiCompletionClient::new(FixtureAiTransport::new(responses), 10, 0.2)
            .with_sleep(move |delay| {
                captured_delays
                    .lock()
                    .expect("retry delay lock should not be poisoned")
                    .push(delay);
            });

        client
            .select_articles(&ai_config(), &subscriber(), &defaults(), &[candidate(103)])
            .expect("retry sequence should eventually succeed");

        let delays = delays
            .lock()
            .expect("retry delay lock should not be poisoned");
        assert_eq!(delays.len(), 10);
        for (delay, cap) in delays.iter().zip([1_u64, 2, 4, 8, 8, 8, 8, 8, 8, 8]) {
            assert!(*delay <= Duration::from_secs(cap));
        }
    }

    #[test]
    fn completion_does_not_retry_or_fallback_nonretryable_http_statuses() {
        for status_code in [400_u16, 401, 403] {
            let responses = vec![Err(AiClientError::HttpStatus {
                status_code,
                request_id: None,
                retry_after_seconds: None,
            })];
            let mut client = AiCompletionClient::new(FixtureAiTransport::new(responses), 10, 0.2)
                .with_sleep(|_| {});

            let error = client
                .select_articles(&ai_config(), &subscriber(), &defaults(), &[candidate(103)])
                .expect_err("nonretryable status should fail immediately");

            assert!(matches!(
                error,
                AiClientError::HttpStatus {
                    status_code: actual,
                    ..
                } if actual == status_code
            ));
            assert_eq!(client.transport().requests.len(), 1);
        }
    }

    #[test]
    fn completion_enforces_one_shared_eight_request_budget() {
        let responses = (0..9)
            .map(|_| {
                Err(AiClientError::HttpStatus {
                    status_code: 429,
                    request_id: None,
                    retry_after_seconds: Some(0),
                })
            })
            .collect::<Vec<_>>();
        let control = DeliveryExecutionControl::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should follow Unix epoch")
                .as_secs_f64()
                + 60.0,
            8,
            || Ok(false),
        );
        let mut client = AiCompletionClient::new_with_control(
            FixtureAiTransport::new(responses),
            10,
            0.2,
            Duration::from_secs(60),
            Some(control),
        )
        .with_sleep(|_| {});

        let error = client
            .select_articles(&ai_config(), &subscriber(), &defaults(), &[candidate(103)])
            .expect_err("ninth AI request should be denied");

        assert_eq!(
            error,
            AiClientError::Control(DeliveryExecutionControlError::AiRequestBudgetExhausted)
        );
        assert_eq!(client.transport().requests.len(), 8);
    }

    #[test]
    fn summarize_selected_articles_returns_summary_text() {
        let response = ok_response(json!({
            "choices": [{
                "message": {
                    "content": "{\"summary\":\"Two selected papers focus on Rust systems.\"}"
                }
            }]
        }));
        let mut client = AiCompletionClient::new(FixtureAiTransport::new(vec![response]), 0, 0.2)
            .with_sleep(|_| {});
        let summary = client
            .summarize_selected_articles(&ai_config(), &subscriber(), &[candidate(101)])
            .expect("summary should succeed");

        assert_eq!(summary, "Two selected papers focus on Rust systems.");
        assert_eq!(
            client.transport().requests[0].body["response_format"]["json_schema"]["name"],
            "selected_paper_summary"
        );
    }

    fn ok_response(body: Value) -> Result<AiHttpResponse, AiClientError> {
        Ok(AiHttpResponse {
            status_code: 200,
            request_id: None,
            retry_after_seconds: None,
            body,
        })
    }

    fn ai_config() -> AiRuntimeConfig {
        AiRuntimeConfig {
            base_url: "https://api.test/v1/".to_string(),
            api_key: "secret".to_string(),
            model: "model".to_string(),
            system_prompt: String::new(),
        }
    }

    fn defaults() -> NotificationDefaults {
        NotificationDefaults {
            max_candidates: 120,
            ai_model: "model".to_string(),
            temperature: 0.2,
        }
    }

    fn subscriber() -> NotificationSubscriberInfo {
        NotificationSubscriberInfo {
            subscriber_id: "1".to_string(),
            user_id: 1,
            name: "Alice".to_string(),
            pushplus_token: "push-token".to_string(),
            channel: Some("wechat".to_string()),
            keywords: vec!["rust".to_string()],
            directions: vec!["systems".to_string()],
            selected_databases: Vec::new(),
            topic: None,
            template: Some("markdown".to_string()),
            delivery_method: "pushplus".to_string(),
            tracking_folder_id: Some(1),
            sync_to_tracking_folder: true,
            ai_base_url: Some("https://api.test/v1".to_string()),
            ai_api_key: Some("secret".to_string()),
            ai_model: Some("model".to_string()),
            ai_system_prompt: None,
            ai_backup_base_url: None,
            ai_backup_api_key: None,
            ai_backup_model: None,
            ai_backup_system_prompt: None,
            ai_retry_attempts: 1,
        }
    }

    fn candidate(article_id: i64) -> ArticleCandidateInfo {
        ArticleCandidateInfo {
            article_id,
            journal_id: 1,
            issue_id: Some(1),
            title: format!("Rust systems {article_id}"),
            abstract_text: "rust systems".to_string(),
            date: Some("2026-07-01".to_string()),
            journal_title: "Journal".to_string(),
            doi: Some(format!("10.0000/{article_id}")),
            open_access: true,
            in_press: false,
        }
    }
}
