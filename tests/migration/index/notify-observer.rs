//! Invoke byte-for-byte extracted original notification classifier and bounded drain functions.
use super::batch::NotifyHandoffStatus;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Cursor, Read};
include!("../../../output/migration/execution/index-oracle/notify-original.rs");

pub fn observe(input: &Value) -> Value {
    let bytes: Vec<u8> = serde_json::from_value(input["bytes"].clone()).unwrap();
    let bounded = read_bounded_notify_output(&mut Cursor::new(bytes)).unwrap();
    let observation = classify_notify_handoff_output(
        &bounded.bytes,
        bounded.exceeded_limit,
        input["attempt"].as_str().unwrap(),
        input["database"].as_str().unwrap(),
        input["dry"].as_bool().unwrap(),
        input["exit"].as_i64().map(|value| value as i32),
    );
    json!({"status":observation.status.as_str(),"exit":observation.exit_code,"retained":bounded.bytes.len(),"exceeded":bounded.exceeded_limit})
}
