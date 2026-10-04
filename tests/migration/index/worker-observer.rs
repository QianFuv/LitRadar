//! Observe the original strict private protocol including serde sequence and stream forms.
use super::worker_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io::Cursor;

fn decode<Message: Serialize + DeserializeOwned>(input: &Value) -> Value {
    let payload = input["payload"].as_str().unwrap();
    if input["stream"].as_bool().unwrap_or(false) {
        let mut reader = Cursor::new(payload.as_bytes());
        let mut results = Vec::new();
        for _ in 0..input["reads"].as_u64().unwrap_or(3) {
            results.push(match read_message::<Message>(&mut reader) {
                Ok(message) => json!({"value":serde_json::to_value(message).unwrap()}),
                Err(error) => json!({"error":error.to_string()}),
            })
        }
        json!(results)
    } else {
        match serde_json::from_str::<Message>(payload) {
            Ok(message) => json!({"value":serde_json::to_value(message).unwrap()}),
            Err(_) => json!({"error":"invalid"}),
        }
    }
}
pub fn observe(input: &Value) -> Value {
    match input["kind"].as_str().unwrap() {
        "request" => decode::<WorkerRequest>(input),
        "assignment" => decode::<WorkerJournalAssignment>(input),
        "bootstrap" => decode::<WorkerBootstrap>(input),
        "worker" => decode::<WorkerMessage>(input),
        "parent" => decode::<ParentMessage>(input),
        "failure" => decode::<WorkerFailure>(input),
        _ => panic!("kind"),
    }
}
