//! Observe original AI and PushPlus request sequences without network traffic.
use litradar_domain::{ArticleCandidateInfo, NotificationSubscriberInfo};
use litradar_recommend::{AiRuntimeConfig, NotificationDefaults};
use litradar_worker::ai::*;
use litradar_worker::pushplus::*;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{self, BufRead};

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn optional(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_owned)
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn subscriber(value: &Value) -> NotificationSubscriberInfo {
    NotificationSubscriberInfo {
        subscriber_id: text(value, "subscriber_id"),
        user_id: value["user_id"].as_i64().unwrap_or(1),
        name: text(value, "name"),
        pushplus_token: text(value, "pushplus_token"),
        channel: optional(value, "channel"),
        keywords: strings(value, "keywords"),
        directions: strings(value, "directions"),
        selected_databases: strings(value, "selected_databases"),
        topic: optional(value, "topic"),
        template: optional(value, "template"),
        delivery_method: text(value, "delivery_method"),
        tracking_folder_id: value["tracking_folder_id"].as_i64(),
        sync_to_tracking_folder: value["sync_to_tracking_folder"].as_bool().unwrap_or(false),
        ai_base_url: optional(value, "ai_base_url"),
        ai_api_key: optional(value, "ai_api_key"),
        ai_model: optional(value, "ai_model"),
        ai_system_prompt: optional(value, "ai_system_prompt"),
        ai_backup_base_url: optional(value, "ai_backup_base_url"),
        ai_backup_api_key: optional(value, "ai_backup_api_key"),
        ai_backup_model: optional(value, "ai_backup_model"),
        ai_backup_system_prompt: optional(value, "ai_backup_system_prompt"),
        ai_retry_attempts: value["ai_retry_attempts"].as_i64().unwrap_or(1),
    }
}

struct Fixture {
    responses: VecDeque<Value>,
    requests: Vec<Value>,
}

impl AiTransport for Fixture {
    fn post_json(&mut self, request: AiHttpRequest) -> Result<AiHttpResponse, AiClientError> {
        self.requests.push(json!({"url":request.url,"headers":request.headers.iter().map(|item|(item.name.to_ascii_lowercase(),item.value.clone())).collect::<std::collections::BTreeMap<_,_>>(),"body":request.body}));
        let response = self
            .responses
            .pop_front()
            .expect("fixture response exhausted");
        match response["error"].as_str() {
            Some("connect_failed") => return Err(AiClientError::ConnectFailed),
            Some("timeout") => return Err(AiClientError::TimedOut),
            Some("transport") => {
                return Err(AiClientError::Transport("Outbound request failed".into()));
            }
            _ => {}
        }
        Ok(AiHttpResponse {
            status_code: response["status"].as_u64().unwrap_or(200) as u16,
            request_id: optional(&response, "request_id"),
            retry_after_seconds: response["retry_after"].as_u64(),
            body: response["body"].clone(),
        })
    }
}

impl PushPlusTransport for Fixture {
    fn post_json(
        &mut self,
        request: PushPlusHttpRequest,
    ) -> Result<PushPlusHttpResponse, PushPlusError> {
        self.requests
            .push(json!({"url":request.url,"body":request.body}));
        let response = self
            .responses
            .pop_front()
            .expect("fixture response exhausted");
        match response["error"].as_str() {
            Some("connect_failed") => return Err(PushPlusError::ConnectFailed),
            Some("timeout") => return Err(PushPlusError::TimedOut),
            Some("transport") => {
                return Err(PushPlusError::Transport("Outbound request failed".into()));
            }
            _ => {}
        }
        Ok(PushPlusHttpResponse {
            status_code: response["status"].as_u64().unwrap_or(200) as u16,
            request_id: optional(&response, "request_id"),
            retry_after_seconds: response["retry_after"].as_u64(),
            body: response["body"].clone(),
        })
    }
}

fn observe(input: &Value) -> Value {
    let fixture = Fixture {
        responses: input["responses"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .collect(),
        requests: vec![],
    };
    let retries = input["retries"].as_u64().unwrap_or(0) as usize;
    if input["op"] == "pushplus" {
        let value = &input["message"];
        let message = PushPlusMessage {
            token: text(value, "token"),
            title: text(value, "title"),
            content: text(value, "content"),
            channel: text(value, "channel"),
            template: text(value, "template"),
            topic: optional(value, "topic"),
            option: optional(value, "option"),
            to: optional(value, "to"),
        };
        let mut client = PushPlusClient::new(fixture, retries).with_sleep(|_| {});
        let result = match client.send(&message) {
            Ok(value) => json!({"value":value}),
            Err(error) => json!({"error":error.to_string()}),
        };
        return json!({"result":result,"requests":client.transport().requests});
    }
    let value = &input["config"];
    let config = AiRuntimeConfig {
        base_url: text(value, "base_url"),
        api_key: text(value, "api_key"),
        model: text(value, "model"),
        system_prompt: text(value, "system_prompt"),
    };
    let defaults = NotificationDefaults {
        max_candidates: 120,
        ai_model: config.model.clone(),
        temperature: 0.2,
    };
    let candidates: Vec<ArticleCandidateInfo> =
        serde_json::from_value(input["candidates"].clone()).unwrap();
    let mut client = AiCompletionClient::new(fixture, retries, 0.2).with_sleep(|_| {});
    let result = if input["op"] == "summary" {
        match client.summarize_selected_articles(
            &config,
            &subscriber(&input["subscriber"]),
            &candidates,
        ) {
            Ok(value) => json!({"value":value}),
            Err(error) => json!({"error":error.to_string()}),
        }
    } else {
        match client.select_articles(
            &config,
            &subscriber(&input["subscriber"]),
            &defaults,
            &candidates,
        ) {
            Ok(value) => json!({"value":value}),
            Err(error) => json!({"error":error.to_string()}),
        }
    };
    json!({"result":result,"requests":client.transport().requests})
}

fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", observe(&input));
    }
}
