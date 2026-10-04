//! Observe original control migrations, leases and frozen traversal transitions.

use litradar_domain::IndexSyncMode;
use litradar_index::control::*;
use serde_json::{Value, json};

fn checkpoint(value: ProviderRunCheckpoint) -> Value {
    json!({"batch_id":value.batch_id,"run_id":value.run_id,"mode":value.mode,"base_anchor":value.base_anchor,"traversal_checkpoint":value.traversal_checkpoint,"started_at":value.started_at,"updated_at":value.updated_at})
}

pub fn observe(input: &Value) -> Value {
    let connection = match input["path"].as_str() {
        Some(path) => rusqlite::Connection::open(path).unwrap(),
        None => rusqlite::Connection::open_in_memory().unwrap(),
    };
    if let Some(sql) = input["setup"].as_str() {
        connection.execute_batch(sql).unwrap();
    }
    let init = init_control_db(&connection);
    if let Err(error) = init {
        return json!({"error":error.to_string()});
    }
    let mut operations = Vec::new();
    for item in input["operations"].as_array().unwrap() {
        let catalog = item["catalog"].as_str().unwrap_or("catalog");
        let provider = item["provider"].as_str().unwrap_or("scholarly");
        let journal = item["journal"].as_str().unwrap_or("catalog:example");
        let batch = item["batch"].as_str().unwrap_or("batch-1");
        let run = item["run"].as_str().unwrap_or("run-1");
        let timestamp = item["timestamp"].as_str().unwrap_or("2026-10-04T00:00:00Z");
        let mode: IndexSyncMode =
            serde_json::from_value(item.get("mode").cloned().unwrap_or(json!("incremental")))
                .unwrap();
        let base = item["base"].as_str();
        let result:Result<Value,String>=match item["op"].as_str().unwrap(){
            "sql"=>connection.execute_batch(item["sql"].as_str().unwrap()).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "prepare"=>prepare_journal_sync(&connection,catalog,provider,journal,batch,run,mode,item["resume"].as_bool().unwrap_or(true),timestamp).map(|value|match value{JournalSyncPreparation::Skip=>json!({"skip":true,"checkpoint":null}),JournalSyncPreparation::Run(value)=>json!({"skip":false,"checkpoint":checkpoint(value)})}).map_err(|error|error.to_string()),
            "advance"=>advance_run_checkpoint(&connection,catalog,provider,journal,batch,run,mode,base,item["checkpoint"].as_str().unwrap(),timestamp).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "complete"=>complete_sync_run(&connection,catalog,provider,journal,batch,run,mode,base,item["anchor"].as_str(),timestamp).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "anchor"=>read_sync_anchor(&connection,catalog,provider,journal).map(|value|value.map(|value|json!({"committed_anchor":value.committed_anchor,"completed_at":value.completed_at,"completed_batch_id":value.completed_batch_id})).unwrap_or(Value::Null)).map_err(|error|error.to_string()),
            "checkpoint"=>read_run_checkpoint(&connection,catalog,provider,journal).map(|value|value.map(checkpoint).unwrap_or(Value::Null)).map_err(|error|error.to_string()),
            "acquire"=>acquire_lease(&connection,catalog,provider,run,item["now"].as_i64().unwrap()).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "heartbeat"=>heartbeat_lease(&connection,catalog,provider,run,item["now"].as_i64().unwrap()).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "release"=>release_lease(&connection,catalog,provider,run).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "counts"=>read_batch_journal_state(&connection,catalog,provider,batch).map(|value|json!({"completed":value.completed,"in_flight":value.in_flight})).map_err(|error|error.to_string()),
            "adopt"=>adopt_legacy_batch_state(&connection,catalog,provider,batch,mode,item["allow"].as_bool().unwrap_or(false)).map(|value|json!({"started_at":value.started_at,"checkpoints_adopted":value.checkpoints_adopted,"anchors_adopted":value.anchors_adopted})).map_err(|error|error.to_string()),
            "abandon"=>abandon_batch_checkpoints(&connection,batch).map(|value|json!(value)).map_err(|error|error.to_string()),
            "aliases"=>has_catalog_alias_sync_state(&connection,catalog,&serde_json::from_value::<Vec<String>>(item["aliases"].clone()).unwrap()).map(|value|json!(value)).map_err(|error|error.to_string()),
            _=>panic!("unknown control operation"),
        };
        operations.push(match result {
            Ok(value) => value,
            Err(error) => json!({"error":error}),
        });
    }
    json!({"operations":operations,"tables":super::snapshot_selected(&connection,&["provider_leases","provider_sync_anchors","provider_run_checkpoints"]),"version":connection.query_row("PRAGMA user_version",[],|row|row.get::<_,i64>(0)).unwrap()})
}
