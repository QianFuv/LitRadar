//! Observe original batch admission, recovery, publication intent and notification fencing.
use super::batch::*;
use litradar_domain::{IndexSyncMode, JournalCatalogEntry};
use serde_json::{Value, json};

fn request(input: &Value) -> Result<IndexBatchRequest, BatchDatabaseError> {
    let mut catalogs = Vec::new();
    for item in input["catalogs"].as_array().unwrap() {
        let entries: Vec<JournalCatalogEntry> =
            serde_json::from_value(item["entries"].clone()).unwrap();
        catalogs.push(CatalogInput {
            path: item["path"].as_str().unwrap_or("runtime.csv").into(),
            file_name: item["filename"].as_str().unwrap().into(),
            catalog_name: item["name"].as_str().unwrap().into(),
            csv_sha256: item["digest"].as_str().unwrap().into(),
            provider_name: item["provider"].as_str().unwrap().into(),
            entries,
        });
    }
    let mode: IndexSyncMode = serde_json::from_value(input["mode"].clone()).unwrap();
    IndexBatchRequest::new(
        catalogs,
        if input["selection"] == "all" {
            CatalogSelection::All
        } else {
            CatalogSelection::ExplicitFile
        },
        mode,
        input["size"].as_u64().unwrap() as usize,
        input["notify"].as_bool().unwrap(),
        input["dry"].as_bool().unwrap(),
    )
}
fn phase(value: &str) -> BatchCatalogPhase {
    match value {
        "pending" => BatchCatalogPhase::Pending,
        "indexing" => BatchCatalogPhase::Indexing,
        "manifest_prepared" => BatchCatalogPhase::ManifestPrepared,
        "manifest_published" => BatchCatalogPhase::ManifestPublished,
        "notifying" => BatchCatalogPhase::Notifying,
        "completed" => BatchCatalogPhase::Completed,
        _ => panic!("phase"),
    }
}
fn phase_text(value: BatchCatalogPhase) -> &'static str {
    match value {
        BatchCatalogPhase::Pending => "pending",
        BatchCatalogPhase::Indexing => "indexing",
        BatchCatalogPhase::ManifestPrepared => "manifest_prepared",
        BatchCatalogPhase::ManifestPublished => "manifest_published",
        BatchCatalogPhase::Notifying => "notifying",
        BatchCatalogPhase::Completed => "completed",
    }
}
fn handoff(value: NotifyHandoffState) -> Value {
    json!({"attempt":value.attempt_id,"status":value.status.as_str(),"exit":value.exit_code,"ack_attempt":value.unknown_acknowledged_attempt_id,"ack_time":value.unknown_acknowledged_at})
}
fn catalogs(values: Vec<IndexBatchCatalog>) -> Value {
    json!(values.into_iter().map(|value|json!({"ordinal":value.ordinal,"filename":value.file_name,"name":value.catalog_name,"provider":value.provider_name,"journals":value.journal_count,"phase":phase_text(value.phase),"outcome":value.outcome.map(|outcome|json!({"run":outcome.run_id,"journals":outcome.journal_count,"written":outcome.written_article_count,"attempts":outcome.source_attempt_count,"path":outcome.manifest_path})),"intent":value.manifest_intent.map(|intent|json!({"payload":intent.payload,"digest":intent.sha256,"through":intent.through_event_id,"path":intent.path,"run":intent.run_id,"generated":intent.generated_at})),"handoff":value.notify_handoff.map(handoff)})).collect::<Vec<_>>())
}
fn outcome(value: &Value) -> BatchCatalogOutcome {
    BatchCatalogOutcome {
        run_id: value["run"].as_str().unwrap().into(),
        journal_count: value["journals"].as_u64().unwrap() as usize,
        written_article_count: value["written"].as_i64().unwrap(),
        source_attempt_count: value["attempts"].as_u64().unwrap() as usize,
        manifest_path: value["path"].as_str().map(str::to_string),
    }
}
fn normalize_batch(value: IndexBatch, ids: &mut Vec<String>) -> Value {
    if !ids.contains(&value.batch_id) {
        ids.push(value.batch_id.clone())
    }
    json!({"id":format!("batch-{}",ids.iter().position(|id|*id==value.batch_id).unwrap()+1),"owner":value.owner_id,"started":value.started_at,"resumed":value.did_resume,"catalogs":catalogs(value.catalogs)})
}
fn normalize_ids(value: &mut Value, ids: &[String]) {
    match value {
        Value::String(text) => {
            if let Some(index) = ids.iter().position(|id| id == text) {
                *text = format!("batch-{}", index + 1)
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_ids(value, ids)
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                normalize_ids(value, ids)
            }
        }
        _ => {}
    }
}

pub fn observe(input: &Value) -> Value {
    let connection = match input["path"].as_str() {
        Some(path) => rusqlite::Connection::open(path).unwrap(),
        None => rusqlite::Connection::open_in_memory().unwrap(),
    };
    if let Some(setup) = input["setup"].as_str() {
        connection.execute_batch(setup).unwrap()
    }
    if let Err(error) = init_batch_db(&connection) {
        return json!({"error":error.to_string()});
    }
    let mut ids: Vec<String> = Vec::new();
    let mut current = String::from("missing");
    let mut operations = Vec::new();
    for item in input["operations"].as_array().unwrap() {
        let owner = item["owner"].as_str().unwrap_or("owner-1");
        let now = item["now"].as_i64().unwrap_or(100);
        let ordinal = item["ordinal"].as_u64().unwrap_or(0) as usize;
        let result: Result<Value, BatchDatabaseError> = (|| {
            Ok(match item["op"].as_str().unwrap() {
                "admit" => {
                    let requested = request(&item["request"])?;
                    let (is_abandoning, value) = match admit_batch(
                        &connection,
                        &requested,
                        item["resume"].as_bool().unwrap_or(true),
                        owner,
                        now,
                    )? {
                        BatchAdmission::Ready(value) => (false, value),
                        BatchAdmission::Abandoning(value) => (true, value),
                    };
                    current = value.batch_id.clone();
                    json!({"abandoning":is_abandoning,"batch":normalize_batch(value,&mut ids)})
                }
                "replace" => {
                    let requested = request(&item["request"])?;
                    let value =
                        replace_abandoning_batch(&connection, &current, &requested, owner, now)?;
                    current = value.batch_id.clone();
                    normalize_batch(value, &mut ids)
                }
                "heartbeat" => {
                    heartbeat_batch_lease(&connection, &current, owner, now)?;
                    json!({"ok":true})
                }
                "release" => {
                    release_batch_lease(&connection, &current, owner)?;
                    json!({"ok":true})
                }
                "read" => catalogs(read_batch_catalogs(&connection, &current)?),
                "phase" => {
                    transition_catalog_phase(
                        &connection,
                        &current,
                        owner,
                        ordinal,
                        phase(item["phase"].as_str().unwrap()),
                        now,
                    )?;
                    json!({"ok":true})
                }
                "outcome" => {
                    store_catalog_outcome(
                        &connection,
                        &current,
                        owner,
                        ordinal,
                        &outcome(&item["value"]),
                        now,
                    )?;
                    json!({"ok":true})
                }
                "intent" => {
                    let value = &item["value"];
                    let mut intent = ManifestIntent::new(
                        serde_json::from_value(value["payload"].clone()).unwrap(),
                        value["through"].as_i64(),
                        value["path"].as_str().unwrap(),
                        value["run"].as_str().unwrap(),
                        value["generated"].as_str().unwrap(),
                    )?;
                    if let Some(digest) = value["digest"].as_str() {
                        intent.sha256 = digest.into()
                    };
                    store_manifest_intent(&connection, &current, owner, ordinal, &intent, now)?;
                    json!({"ok":true})
                }
                "notify_prepare" => {
                    let value = prepare_notify_attempt(
                        &connection,
                        &current,
                        owner,
                        ordinal,
                        item["attempt"].as_str().unwrap(),
                        item["ack"].as_bool().unwrap_or(false),
                        now,
                    )?;
                    match value {
                        NotifyAttemptPreparation::Run(state) => {
                            json!({"decision":"run","state":handoff(state)})
                        }
                        NotifyAttemptPreparation::Succeeded(state) => {
                            json!({"decision":"succeeded","state":handoff(state)})
                        }
                        NotifyAttemptPreparation::BlockedUnknown(state) => {
                            json!({"decision":"blocked_unknown","state":handoff(state)})
                        }
                    }
                }
                "notify_record" => handoff(record_notify_attempt_result(
                    &connection,
                    &current,
                    owner,
                    ordinal,
                    item["attempt"].as_str().unwrap(),
                    NotifyHandoffStatus::parse(item["status"].as_str().unwrap())?,
                    item["exit"].as_i64().map(|value| value as i32),
                    now,
                )?),
                "complete_catalog" => {
                    complete_catalog(
                        &connection,
                        &current,
                        owner,
                        ordinal,
                        &outcome(&item["value"]),
                        now,
                    )?;
                    json!({"ok":true})
                }
                "complete_batch" => {
                    complete_batch(&connection, &current, owner, now)?;
                    json!({"ok":true})
                }
                "sql" => {
                    connection.execute_batch(item["sql"].as_str().unwrap())?;
                    json!({"ok":true})
                }
                _ => panic!("operation"),
            })
        })();
        operations.push(match result {
            Ok(value) => value,
            Err(error) => json!({"error":error.to_string()}),
        });
    }
    let mut result = json!({"operations":operations,"tables":super::snapshot_selected(&connection,&["index_batches","index_batch_catalogs","index_batch_lease"]),"version":connection.query_row("PRAGMA user_version",[],|row|row.get::<_,i64>(0)).unwrap()});
    normalize_ids(&mut result, &ids);
    result
}
