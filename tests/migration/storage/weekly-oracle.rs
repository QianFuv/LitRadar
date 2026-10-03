//! Observe unchanged Rust weekly JSON and timestamp parsing, including ignored publications.
pub use litradar_storage::{IndexRepositoryError, StorageConfig, open_sqlite_connection};
#[allow(dead_code)]
#[path = "../../../crates/litradar-storage/src/weekly_manifest.rs"]
mod weekly_manifest;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn timestamp(value: DateTime<Utc>) -> Value {
    json!({"seconds":value.timestamp(),"nanoseconds":value.timestamp_subsec_nanos(),
        "whole":value.to_rfc3339_opts(SecondsFormat::Secs,true),"precise":value.to_rfc3339_opts(SecondsFormat::AutoSi,true),
        "window_start":weekly_manifest::weekly_window_start(value).to_rfc3339_opts(SecondsFormat::AutoSi,true)})
}

/// Preserve the three-way distinction between invalid JSON, ignored input and a valid publication.
fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let input = request["input"].as_str().unwrap();
        let result = if request["kind"] == "date" {
            json!({"timestamp":weekly_manifest::parse_iso_datetime(input).map(timestamp)})
        } else {
            match serde_json::from_str::<weekly_manifest::WeeklyManifestPayload>(input) {
                Err(_) => json!({"valid":false,"manifest":null}),
                Ok(payload) => {
                    json!({"valid":true,"manifest":weekly_manifest::parse_weekly_manifest(payload).map(|value|json!({"db_name":value.db_name,"run_id":value.run_id,"timestamp":timestamp(value.generated_at),"ids":value.article_ids.iter().map(i64::to_string).collect::<Vec<_>>()}))})
                }
            }
        };
        request["output"] = result;
        writeln!(output, "{request}")?;
    }
    output.flush()
}
