//! Observe unchanged recommendation functions through their public Rust interfaces.

use litradar_domain::{ArticleCandidateInfo, NotificationSubscriberInfo, SelectionResultInfo};
use litradar_recommend::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
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

fn observe(input: &Value) -> Value {
    match input["op"].as_str().unwrap() {
        "manifest" => {
            let directory =
                std::path::Path::new("output/migration/execution/delivery-oracle/manifest-inputs");
            std::fs::create_dir_all(directory).unwrap();
            let path = directory.join(format!("{}.json", std::process::id()));
            std::fs::write(&path, input["raw"].as_str().unwrap()).unwrap();
            let result = match load_change_manifest(&path, input["database"].as_str().unwrap()) {
                Ok(value) => {
                    json!({"value":{"pending_issue_keys":value.pending_issue_keys,"pending_inpress_keys":value.pending_inpress_keys,"pending_article_ids":value.pending_article_ids,"run_id":value.run_id}})
                }
                Err(RecommendationError::Json(_)) => json!({"error_kind":"json"}),
                Err(error) => json!({"error":error.to_string()}),
            };
            std::fs::remove_file(&path).unwrap();
            result
        }
        "checkpoint" => {
            match serde_json::from_str::<RecommendationSnapshot>(input["raw"].as_str().unwrap()) {
                Ok(value) => json!({"value":value}),
                Err(_) => json!({"error":"Stored delivery checkpoint is invalid"}),
            }
        }
        "payload" => {
            let kind = if input["kind"] == "summary" {
                AiPayloadKind::Summary
            } else {
                AiPayloadKind::Selection
            };
            match extract_response_payload(&input["response"], kind) {
                Ok(value) => json!({"value":value}),
                Err(error) => json!({"error":error.to_string()}),
            }
        }
        "selection" | "message" => {
            let candidates: Vec<ArticleCandidateInfo> =
                serde_json::from_value(input["candidates"].clone()).unwrap();
            let subscriber = subscriber(&input["subscriber"]);
            let by_id = candidates
                .iter()
                .map(|item| (item.article_id, item.clone()))
                .collect::<BTreeMap<_, _>>();
            let selection: SelectionResultInfo =
                serde_json::from_value(input["selection"].clone()).unwrap();
            if input["op"] == "message" {
                json!({"title":build_message_title(input["database"].as_str().unwrap(),input["run"].as_str().unwrap()),"content":build_markdown_content(input["database"].as_str().unwrap(),input["run"].as_str().unwrap(),&subscriber,&selection.summary,&selection.selections,&by_id)})
            } else {
                let dedupe: BTreeMap<String, String> =
                    serde_json::from_value(input["dedupe"].clone()).unwrap();
                json!({"accepted":apply_selection_rules(&selection,&subscriber,&by_id,&dedupe),"matches":candidates.iter().map(|item|candidate_match_score(item,&subscriber)).collect::<Vec<_>>(),"has_preferences":has_selection_preferences(&subscriber),"deduped":deduplicate_candidates(candidates)})
            }
        }
        "config" => {
            let value = &input["global"];
            let global = NotificationGlobalConfig {
                ai_base_url: text(value, "ai_base_url"),
                ai_allowed_base_urls: strings(value, "ai_allowed_base_urls"),
                ai_api_key: text(value, "ai_api_key"),
                pushplus_channel: text(value, "pushplus_channel"),
                pushplus_template: text(value, "pushplus_template"),
                pushplus_topic: optional(value, "pushplus_topic"),
                pushplus_option: optional(value, "pushplus_option"),
                ai_system_prompt: optional(value, "ai_system_prompt"),
            };
            let defaults = NotificationDefaults {
                max_candidates: 120,
                ai_model: text(&input["defaults"], "ai_model"),
                temperature: 0.2,
            };
            let configs = resolve_ai_runtime_configs(
                &subscriber(&input["subscriber"]),
                &global,
                &defaults,
                input["override"].as_str(),
            );
            json!(configs.iter().map(|item|json!({"base_url":item.base_url,"api_key":item.api_key,"model":item.model,"system_prompt":item.system_prompt})).collect::<Vec<_>>())
        }
        "snapshot" => {
            let previous: BTreeMap<String, i64> =
                serde_json::from_value(input["previous"].clone()).unwrap();
            let current: BTreeMap<String, i64> =
                serde_json::from_value(input["current"].clone()).unwrap();
            json!({"issues":compute_changed_issue_keys(&previous,&current),"inpress":compute_changed_inpress_keys(&previous,&current)})
        }
        "database" => json!(is_database_selected(
            &strings(input, "selected"),
            input["database"].as_str().unwrap()
        )),
        _ => panic!("unsupported recommendation observation"),
    }
}

fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", observe(&input));
    }
}
