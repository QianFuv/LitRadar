//! OpenAI-compatible AI selection client for delivery workers.

use std::error::Error;
use std::fmt;
use std::thread;
use std::time::Duration;

use ps_domain::{
    ArticleCandidateInfo, NotificationSubscriberInfo, RankedSelectionInfo, SelectionResultInfo,
};
use ps_recommend::{
    extract_response_payload, AiPayloadKind, AiRuntimeConfig, NotificationDefaults,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};

const CHAT_COMPLETIONS_PATH: &str = "chat/completions";
const HTTP_REFERER: &str = "https://github.com/openai/codex";
const X_TITLE: &str = "Paper Scanner";
const SUMMARY_PROMPT_SUFFIX: &str = "Only summarize the supplied selected papers. Focus on major research themes, methods, and findings.";
const SELECTION_OUTPUT_CONTRACT: &str = "Return exactly one JSON object with keys \"summary\" and \"selected\". \"selected\" must be an array of objects that each contain \"article_id\" and \"score\". Do not wrap JSON in markdown fences.";
const SUMMARY_OUTPUT_CONTRACT: &str =
    "Return exactly one JSON object with the key \"summary\". Do not wrap JSON in markdown fences.";

/// Default system prompt used by the Python notification selector.
pub const DEFAULT_SELECTION_SYSTEM_PROMPT: &str = "You are a precise academic recommender. Use two-stage selection: directions-first filtering, then keyword-based ranking in the filtered set. Return relevant candidates ranked by score. Order selected items from highest to lowest. Judge by article content quality and topic relevance only. Ignore journal quality, prestige, and ranking completely. Do not invent article ids.";

/// Error returned by AI delivery clients.
#[derive(Debug, Clone, PartialEq)]
pub enum AiClientError {
    /// HTTP transport failed before a response payload was available.
    Transport(String),
    /// Upstream returned a non-success HTTP status.
    HttpStatus {
        /// HTTP status code.
        status_code: u16,
        /// Parsed response payload or raw body wrapper.
        body: Value,
    },
    /// Upstream response could not be parsed into the expected payload.
    InvalidResponse(String),
}

impl fmt::Display for AiClientError {
    /// Format the AI client error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) => formatter.write_str(message),
            Self::HttpStatus { status_code, body } => {
                write!(
                    formatter,
                    "AI request failed with HTTP {status_code}: {body}"
                )
            }
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
#[derive(Debug, Clone, PartialEq)]
pub struct AiHttpRequest {
    /// Request URL.
    pub url: String,
    /// Request headers.
    pub headers: Vec<AiHttpHeader>,
    /// JSON request body.
    pub body: Value,
}

/// HTTP response returned by an AI transport.
#[derive(Debug, Clone, PartialEq)]
pub struct AiHttpResponse {
    /// HTTP status code.
    pub status_code: u16,
    /// JSON response body.
    pub body: Value,
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
#[derive(Debug, Clone)]
pub struct ReqwestAiTransport {
    client: Client,
}

impl ReqwestAiTransport {
    /// Build a reqwest-backed AI transport.
    ///
    /// # Arguments
    ///
    /// * `timeout_seconds` - Request timeout in seconds.
    ///
    /// # Returns
    ///
    /// Reqwest AI transport.
    pub fn new(timeout_seconds: u64) -> Result<Self, AiClientError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(1)))
            .build()
            .map_err(|error| AiClientError::Transport(error.to_string()))?;
        Ok(Self { client })
    }
}

impl AiTransport for ReqwestAiTransport {
    /// Send one JSON POST request through reqwest.
    fn post_json(&mut self, request: AiHttpRequest) -> Result<AiHttpResponse, AiClientError> {
        let mut builder = self.client.post(&request.url);
        for header in &request.headers {
            builder = builder.header(header.name.as_str(), header.value.as_str());
        }
        let response = builder
            .json(&request.body)
            .send()
            .map_err(|error| AiClientError::Transport(error.to_string()))?;
        let status_code = response.status().as_u16();
        let text = response
            .text()
            .map_err(|error| AiClientError::Transport(error.to_string()))?;
        let body =
            serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "error": text }));
        Ok(AiHttpResponse { status_code, body })
    }
}

/// OpenAI-compatible completion client.
pub struct AiCompletionClient<T: AiTransport> {
    transport: T,
    retry_attempts: usize,
    temperature: f64,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
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
        Self {
            transport,
            retry_attempts,
            temperature,
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
        let mut last_error = AiClientError::InvalidResponse("AI request was not attempted".into());
        for response_format in response_format_variants(schema_name, &schema) {
            for attempt in 0..=self.retry_attempts {
                let body =
                    completion_body(config, self.temperature, &messages, response_format.clone());
                let request = AiHttpRequest {
                    url: chat_completions_url(&config.base_url),
                    headers: ai_headers(&config.api_key),
                    body,
                };
                match self.send_completion(request, payload_kind) {
                    Ok(payload) => return Ok(payload),
                    Err(error) => {
                        last_error = error;
                        if attempt < self.retry_attempts {
                            (self.sleep)(Duration::from_secs(2_u64.pow(attempt as u32)));
                        }
                    }
                }
            }
        }
        Err(AiClientError::InvalidResponse(format!(
            "AI request failed: {last_error}"
        )))
    }

    fn send_completion(
        &mut self,
        request: AiHttpRequest,
        payload_kind: AiPayloadKind,
    ) -> Result<Value, AiClientError> {
        let response = self.transport.post_json(request)?;
        if !(200..300).contains(&response.status_code) {
            return Err(AiClientError::HttpStatus {
                status_code: response.status_code,
                body: response.body,
            });
        }
        extract_response_payload(&response.body, payload_kind)
            .map_err(|error| AiClientError::InvalidResponse(error.to_string()))
    }
}

/// Build a live OpenAI-compatible completion client.
///
/// # Arguments
///
/// * `timeout_seconds` - Request timeout in seconds.
/// * `retry_attempts` - Retry attempts per response format variant.
/// * `temperature` - Model temperature.
///
/// # Returns
///
/// Live completion client.
pub fn live_ai_client(
    timeout_seconds: u64,
    retry_attempts: usize,
    temperature: f64,
) -> Result<AiCompletionClient<ReqwestAiTransport>, AiClientError> {
    Ok(AiCompletionClient::new(
        ReqwestAiTransport::new(timeout_seconds)?,
        retry_attempts,
        temperature,
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

fn response_format_variants(schema_name: &str, schema: &Value) -> Vec<Option<Value>> {
    vec![
        Some(json!({
            "type": "json_schema",
            "json_schema": {
                "name": schema_name,
                "strict": true,
                "schema": schema
            }
        })),
        Some(json!({ "type": "json_object" })),
        None,
    ]
}

fn chat_completions_url(base_url: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        CHAT_COMPLETIONS_PATH
    )
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
        "in_press": candidate.in_press,
        "within_library_holdings": candidate.within_library_holdings
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
mod tests {
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
    fn request_debug_redacts_authorization_header() {
        let request = AiHttpRequest {
            url: "https://ai.example.com/chat/completions".to_string(),
            headers: vec![AiHttpHeader {
                name: "Authorization".to_string(),
                value: "Bearer ai-request-secret".to_string(),
            }],
            body: json!({"model": "fixture-model"}),
        };

        let debug = format!("{request:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("ai-request-secret"));
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
            Err(AiClientError::HttpStatus {
                status_code: 400,
                body: json!({"error": "schema unsupported"}),
            }),
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
            Err(AiClientError::Transport("temporary".into())),
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
            full_text_file: None,
            permalink: None,
            open_access: true,
            in_press: false,
            within_library_holdings: true,
        }
    }
}
