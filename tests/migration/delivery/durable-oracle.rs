//! Observe unchanged Rust delivery transitions and every persisted SQLite storage class.

use litradar_storage::*;
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};
use std::io::{self, BufRead};
use std::path::Path;

fn text<'a>(step: &'a Value, name: &str, fallback: &'a str) -> &'a str {
    step[name].as_str().unwrap_or(fallback)
}
fn number(step: &Value, name: &str, fallback: i64) -> i64 {
    step[name].as_i64().unwrap_or(fallback)
}
fn real(step: &Value, name: &str, fallback: f64) -> f64 {
    step[name].as_f64().unwrap_or(fallback)
}
fn workflow(step: &Value) -> DeliveryWorkflow {
    match text(step, "workflow", "notify") {
        "notify" => DeliveryWorkflow::Notify,
        "push" => DeliveryWorkflow::Push,
        _ => panic!("bad fixture workflow"),
    }
}
fn run_status(step: &Value) -> DeliveryRunStatus {
    match text(step, "status", "completed") {
        "queued" => DeliveryRunStatus::Queued,
        "claimed" => DeliveryRunStatus::Claimed,
        "running" => DeliveryRunStatus::Running,
        "cancelling" => DeliveryRunStatus::Cancelling,
        "completed" => DeliveryRunStatus::Completed,
        "failed" => DeliveryRunStatus::Failed,
        "cancelled" => DeliveryRunStatus::Cancelled,
        "timed_out" => DeliveryRunStatus::TimedOut,
        "skipped" => DeliveryRunStatus::Skipped,
        "unknown" => DeliveryRunStatus::Unknown,
        _ => panic!("bad run status"),
    }
}
fn item_status(step: &Value) -> DeliveryItemStatus {
    match text(step, "status", "succeeded") {
        "pending" => DeliveryItemStatus::Pending,
        "claimed" => DeliveryItemStatus::Claimed,
        "sending" => DeliveryItemStatus::Sending,
        "succeeded" => DeliveryItemStatus::Succeeded,
        "failed" => DeliveryItemStatus::Failed,
        "cancelled" => DeliveryItemStatus::Cancelled,
        "skipped" => DeliveryItemStatus::Skipped,
        "unknown" => DeliveryItemStatus::Unknown,
        _ => panic!("bad item status"),
    }
}
fn dedupe_status(step: &Value) -> DeliveryDedupeStatus {
    match text(step, "dedupe_status", "confirmed") {
        "reserved" => DeliveryDedupeStatus::Reserved,
        "confirmed" => DeliveryDedupeStatus::Confirmed,
        "unknown" => DeliveryDedupeStatus::Unknown,
        _ => panic!("bad dedupe status"),
    }
}
fn update(step: &Value) -> DeliveryCheckpointUpdate {
    DeliveryCheckpointUpdate {
        status: match text(step, "checkpoint_status", "completed") {
            "idle" => DeliveryCheckpointStatus::Idle,
            "running" => DeliveryCheckpointStatus::Running,
            "completed" => DeliveryCheckpointStatus::Completed,
            "failed" => DeliveryCheckpointStatus::Failed,
            "skipped" => DeliveryCheckpointStatus::Skipped,
            "unknown" => DeliveryCheckpointStatus::Unknown,
            _ => panic!("bad checkpoint status"),
        },
        snapshot_json: text(step, "snapshot", "{}").into(),
        last_completed_run_at: step["completed_at"].as_str().map(str::to_string),
        updated_at: real(step, "now", 100.25),
    }
}
fn resolutions(step: &Value) -> Vec<DeliveryDedupeResolution> {
    step["reservations"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| DeliveryDedupeResolution {
                    id: number(value, "id", 1),
                    expected_revision: number(value, "revision", 0),
                })
                .collect()
        })
        .unwrap_or_default()
}
fn run_create(step: &Value) -> DeliveryRunCreate {
    DeliveryRunCreate {
        external_id: text(step, "external_id", "run").into(),
        workflow: workflow(step),
        scope_key: text(step, "scope_key", "fixture.sqlite").into(),
        db_name: if step.get("db_name").is_some() {
            step["db_name"].as_str().map(str::to_string)
        } else {
            Some("fixture.sqlite".into())
        },
        trigger_kind: match text(step, "trigger", "scheduled") {
            "scheduled" => DeliveryTriggerKind::Scheduled,
            "manual" => DeliveryTriggerKind::Manual,
            "legacy" => DeliveryTriggerKind::Legacy,
            _ => panic!("bad trigger"),
        },
        mode: if text(step, "mode", "execute") == "execute" {
            DeliveryRunMode::Execute
        } else {
            DeliveryRunMode::DryRun
        },
        user_id: step["user_id"].as_i64(),
        deadline_at: step["deadline"].as_f64(),
        created_at: real(step, "now", 100.25),
    }
}

fn perform(path: &Path, step: &Value) -> Result<Value, DeliveryRepositoryError> {
    let run = number(step, "run", 1);
    let revision = number(step, "revision", 0);
    let now = real(step, "now", 100.25);
    let owner = text(step, "owner", "owner");
    let database = text(step, "db", "fixture.sqlite");
    let seconds = real(step, "seconds", 10.0);
    let item = number(step, "item", 1);
    let ok = |_| json!("ok");
    match text(step,"op","") {
        "admit"|"admit_manual"=> {
            let create=run_create(step);
            let result = if step["op"] == "admit_manual" {
                match admit_manual_delivery_run(path,&create)? {
                    ManualDeliveryRunAdmissionOutcome::Admitted(result) => result,
                    ManualDeliveryRunAdmissionOutcome::BlockedUnknown(record) => return Ok(json!({"kind":"blocked_unknown","id":record.id})),
                }
            } else { admit_delivery_run(path,&create)? };
            Ok(match result {DeliveryRunAdmissionOutcome::Enqueued(record)=>json!({"kind":"enqueued","id":record.id}),DeliveryRunAdmissionOutcome::Existing(record)=>json!({"kind":"existing","id":record.id}),DeliveryRunAdmissionOutcome::Busy(record)=>json!({"kind":"busy","id":record.id})})
        }
        "claim"=>Ok(match claim_delivery_run(path,run,owner,revision,now,seconds)?{DeliveryRunClaimOutcome::Claimed(record)=>json!({"kind":"claimed","id":record.id}),DeliveryRunClaimOutcome::Busy(record)=>json!({"kind":"busy","id":record.id}),DeliveryRunClaimOutcome::Unavailable(record)=>json!({"kind":"unavailable","id":record.id})}),
        "start"=>start_delivery_run(path,run,owner,revision,now).map(|_|json!("ok")),
        "renew"=>renew_delivery_run(path,run,owner,revision,now,seconds).map(|_|json!("ok")),
        "cancel"=>request_delivery_run_cancellation(path,run,revision,now).map(|_|json!("ok")),
        "finalize"=>finalize_delivery_run(path,run,owner,revision,run_status(step),step["result"].as_str(),step["error_code"].as_str(),now).map(|_|json!("ok")),
        "finalize_queued"=>finalize_queued_delivery_run(path,run,revision,run_status(step),step["result"].as_str(),step["error_code"].as_str(),now).map(|_|json!("ok")),
        "checkpoint"=>compare_and_swap_delivery_checkpoint(path,workflow(step),database,step["checkpoint_revision"].as_i64(),&update(step)).map(|_|json!("ok")),
        "finalize_checkpoint"=>finalize_delivery_run_with_checkpoint(path,run,owner,revision,run_status(step),step["result"].as_str(),step["error_code"].as_str(),workflow(step),database,step["checkpoint_revision"].as_i64(),&update(step),number(step,"lease_revision",0)).map(|_|json!("ok")),
        "items"|"insert_items"=>{
            let items=step["items"].as_array().unwrap().iter().map(|item|DeliveryRunItemCreate{item_kind:match text(item,"kind","article"){"issue"=>DeliveryItemKind::Issue,"inpress"=>DeliveryItemKind::InPress,"article"=>DeliveryItemKind::Article,"subscriber"=>DeliveryItemKind::Subscriber,_=>panic!("bad item kind")},item_key:text(item,"key","1").into(),user_id:item["user_id"].as_i64(),article_id:item["article_id"].as_i64()}).collect::<Vec<_>>();
            if step["op"]=="items"{ensure_delivery_run_items(path,run,&items,now).map(|_|json!("ok"))}else{insert_delivery_run_items(path,run,&items,now).map(|_|json!("ok"))}
        }
        "claim_item"=>claim_delivery_run_item(path,run,owner,revision,item,text(step,"item_owner",owner),now,seconds).map(|record|json!({"id":record.id})),
        "claim_next"=>claim_next_delivery_run_item(path,run,owner,revision,text(step,"item_owner",owner),now,seconds).map(|record|record.map(|record|json!({"id":record.id})).unwrap_or(Value::Null)),
        "sending"=>mark_delivery_run_item_sending(path,item,owner,revision,now).map(|_|json!("ok")),
        "finalize_item"=>finalize_delivery_run_item(path,item,owner,revision,item_status(step),step["result"].as_str(),step["error_code"].as_str(),now).map(|_|json!("ok")),
        "reserve"=>Ok(match reserve_delivery_dedupe(path,workflow(step),database,number(step,"user_id",1),number(step,"article_id",7),run,owner,now)?{DeliveryDedupeReserveOutcome::Reserved(record)=>json!({"kind":"reserved","id":record.id}),DeliveryDedupeReserveOutcome::Existing(record)=>json!({"kind":"existing","id":record.id})}),
        "resolve"=>resolve_delivery_dedupe(path,number(step,"dedupe",1),run,owner,revision,dedupe_status(step),step["message"].as_str(),now).map(|_|json!("ok")),
        "release_reservations"=>release_delivery_dedupe_reservations(path,run,owner,&resolutions(step)).map(|count|json!(count)),
        "finalize_attempt"=>finalize_delivery_attempt(path,item,owner,revision,item_status(step),step["result"].as_str(),step["error_code"].as_str(),run,&resolutions(step),dedupe_status(step),step["message"].as_str(),now).map(|_|json!("ok")),
        "cleanup"=>cleanup_confirmed_delivery_dedupe(path,workflow(step),database,now).map(|count|json!(count)),
        "acquire"=>Ok(match acquire_delivery_lease(path,workflow(step),database,run,owner,now,seconds)?{DeliveryLeaseAcquireOutcome::Acquired(record)=>json!({"kind":"acquired","id":record.id}),DeliveryLeaseAcquireOutcome::Busy(record)=>json!({"kind":"busy","id":record.id})}),
        "renew_lease"=>renew_delivery_lease(path,workflow(step),database,run,owner,revision,now,seconds).map(|_|json!("ok")),
        "release_lease"=>release_delivery_lease(path,workflow(step),database,run,owner,revision,now).map(|_|json!("ok")),
        "reconcile"=>reconcile_delivery_run_after_takeover(path,run,owner,revision,now).map(|result|json!({"reset_item_count":result.reset_item_count,"unknown_item_count":result.unknown_item_count,"released_dedupe_count":result.released_dedupe_count,"unknown_dedupe_count":result.unknown_dedupe_count})),
        "load"=>load_delivery_run(path,run).map(|record|record.map(|record|json!({"id":record.id})).unwrap_or(Value::Null)),
        "list_items"=>list_delivery_run_items(path,run).map(|items|json!(items.len())),
        "load_checkpoint"=>load_delivery_checkpoint(path,workflow(step),database).map(|record|record.map(|record|json!({"id":record.id})).unwrap_or(Value::Null)),
        "load_lease"=>load_delivery_lease(path,workflow(step),database).map(|record|record.map(|record|json!({"id":record.id})).unwrap_or(Value::Null)),
        "noop"=>Ok(ok(())),
        _=>panic!("unknown fixture operation")
    }
}

fn snapshot(path: &Path) -> Value {
    let connection = Connection::open(path).unwrap();
    let mut tables = serde_json::Map::new();
    for table in [
        "delivery_checkpoints",
        "delivery_runs",
        "delivery_run_items",
        "delivery_dedupe",
        "delivery_leases",
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY id"))
            .unwrap();
        let count = statement.column_count();
        let rows=statement.query_map([],|row|Ok((0..count).map(|index|match row.get_ref(index).unwrap(){ValueRef::Null=>Value::Null,ValueRef::Integer(value)=>json!({"integer":value.to_string()}),ValueRef::Real(value)=>json!({"real":value}),ValueRef::Text(value)=>json!({"text":std::str::from_utf8(value).unwrap()}),ValueRef::Blob(value)=>json!({"blob":value.iter().map(|byte|format!("{byte:02x}")).collect::<String>()})}).collect::<Vec<_>>())).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        tables.insert(table.into(), json!(rows));
    }
    Value::Object(tables)
}

fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let path = Path::new(input["path"].as_str().unwrap());
        if !input["resume"].as_bool().unwrap_or(false) {
            migrate_auth_database(path).unwrap();
            Connection::open(path).unwrap().execute_batch("INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'fixture','hash','salt',1,1),(2,'second','hash','salt',1,1)").unwrap();
        }
        let mut observations = Vec::new();
        for step in input["steps"].as_array().unwrap() {
            if let Some(sql) = step["sql"].as_str() {
                Connection::open(path).unwrap().execute_batch(sql).unwrap();
            }
            let outcome = match perform(path, step) {
                Ok(value) => value,
                Err(error) => json!({"error":error.to_string()}),
            };
            observations.push(json!({"outcome":outcome,"tables":snapshot(path)}));
        }
        println!("{}", json!(observations));
    }
}
