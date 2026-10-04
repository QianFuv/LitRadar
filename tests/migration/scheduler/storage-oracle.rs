//! Observe unchanged Rust scheduler operations and every SQLite value after each transition.
use litradar_domain::{ScheduledJobSpec, SchedulerRunState};
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
fn flag(step: &Value, name: &str, fallback: bool) -> bool {
    step[name].as_bool().unwrap_or(fallback)
}

fn perform(path: &Path, step: &Value) -> Result<Value, BusinessRepositoryError> {
    let id = number(step, "task", 1);
    let run = number(step, "run", 1);
    let now = real(step, "now", 100.25);
    let lease = real(step, "lease", 10.0);
    let worker = text(step, "worker", "worker");
    match text(step,"op",""){
 "create"=>{let job:ScheduledJobSpec=serde_json::from_value(step.get("job").cloned().unwrap_or(json!({"kind":"index"})))?;let task=create_scheduled_task(path,ScheduledTaskCreateParams{name:text(step,"name","fixture"),job:&job,cron:text(step,"cron","* * * * *"),timezone:text(step,"timezone","UTC"),timeout_seconds:number(step,"timeout",60) as u64,coalesce:flag(step,"coalesce",true),enabled:flag(step,"enabled",true)})?;Connection::open(path)?.execute("UPDATE scheduled_tasks SET created_at=?1,updated_at=?1 WHERE id=?2",(now,task.id))?;Ok(json!(task.id))},
 "update"=>{let job:Option<ScheduledJobSpec>=step.get("job").map(|value|serde_json::from_value(value.clone())).transpose()?;let task=update_scheduled_task(path,ScheduledTaskUpdateParams{task_id:id,name:step["name"].as_str(),job:job.as_ref(),cron:step["cron"].as_str(),timezone:step["timezone"].as_str(),timeout_seconds:step["timeout"].as_u64(),coalesce:step["coalesce"].as_bool(),enabled:step["enabled"].as_bool()})?;if task.is_some(){Connection::open(path)?.execute("UPDATE scheduled_tasks SET updated_at=?1 WHERE id=?2",(now,id))?;};Ok(json!(task.map(|task|task.id)))},
 "get"=>Ok(json!(get_scheduled_task(path,id)?)),
 "list"=>Ok(json!(list_scheduled_tasks(path)?)),
 "delete"=>Ok(json!(delete_scheduled_task(path,id)?)),
 "enqueue"=>{let task=get_scheduled_task(path,id)?.unwrap();let slots:Vec<i64>=serde_json::from_value(step["slots"].clone())?;Ok(json!(enqueue_scheduled_runs(path,&task,&slots)?))},
 "manual"=>Ok(match claim_manual_scheduled_run(path,id,worker,now,lease)?{ManualScheduledRunAdmission::NotFound=>json!({"status":"not_found"}),ManualScheduledRunAdmission::Busy=>json!({"status":"busy"}),ManualScheduledRunAdmission::Claimed(claim)=>json!({"status":"claimed","run_id":claim.run_id,"scheduled_for":claim.scheduled_for,"worker_id":claim.worker_id,"task":claim.task})}),
 "claim"=>Ok(json!(claim_ready_scheduled_runs_with_limit(path,worker,now,lease,step["capacity"].as_u64().unwrap_or(u64::MAX) as usize)?.iter().map(|claim|json!({"run_id":claim.run_id,"scheduled_for":claim.scheduled_for,"worker_id":claim.worker_id,"task":claim.task})).collect::<Vec<_>>())),
 "start"=>Ok(json!(start_scheduled_run(path,run,worker,now,lease)?)),
 "renew"=>Ok(json!(heartbeat_scheduled_run(path,run,worker,now,lease)?)),
 "finish"=>{let task=get_scheduled_task(path,id)?.unwrap_or(serde_json::from_value(json!({"id":id,"name":"deleted","job":null,"legacy_command":null,"cron":"","timezone":"UTC","timeout_seconds":60,"coalesce":false,"enabled":false,"last_run_at":null,"last_status":"idle","created_at":0.0,"updated_at":0.0}))?);let state:SchedulerRunState=serde_json::from_value(json!(text(step,"status","success")))?;Ok(json!(finish_scheduled_run(path,&ScheduledRunClaim{run_id:run,scheduled_for:0,worker_id:worker.into(),task},state,text(step,"summary",""),now)?))},
 "heartbeat"=>record_scheduler_heartbeat(path,worker,now).map(|_|json!("ok")),
 "check"=>record_scheduler_check(path,now).map(|_|json!("ok")),
 "cursor"=>Ok(json!(get_scheduler_last_checked_at(path)?)),
 "status"=>Ok(json!(get_scheduler_status(path,now,real(step,"window",90.0),number(step,"limit",10) as usize)?)),
 "noop"=>Ok(json!("ok")),
 _=>panic!("unsupported observer operation")
 }
}

fn snapshot(path: &Path) -> Value {
    let connection = Connection::open(path).unwrap();
    let mut tables = serde_json::Map::new();
    for (table, order) in [
        ("scheduled_tasks", "id"),
        ("scheduled_task_runs", "id"),
        ("scheduler_state", "id"),
        ("scheduler_workers", "worker_id"),
        ("service_heartbeats", "service,instance_id"),
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
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
            migrate_auth_database(path).unwrap()
        };
        let mut observations = Vec::new();
        for step in input["steps"].as_array().unwrap() {
            if let Some(sql) = step["sql"].as_str() {
                Connection::open(path)
                    .unwrap()
                    .execute_batch(&format!("PRAGMA ignore_check_constraints=ON;{sql}"))
                    .unwrap()
            };
            let outcome = match perform(path, step) {
                Ok(value) => value,
                Err(error) => {
                    json!({"error":match error {BusinessRepositoryError::InvalidScheduledJob(ref value)|BusinessRepositoryError::InvalidScheduledTask(ref value)=>value.clone(),BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled=>error.to_string(),_=>"storage".into()}})
                }
            };
            observations.push(json!({"outcome":outcome,"tables":snapshot(path)}))
        }
        println!("{}", json!(observations))
    }
}
