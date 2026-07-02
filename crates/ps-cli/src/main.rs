//! Rust backend command entrypoints.

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ps_index::{run_scholarly_fixture_index, ScholarlyIndexConfig};
use ps_worker::delivery::{
    run_recommendation_delivery, DeliveryMode, DeliveryWorkflow, RecommendationRunConfig,
};
use ps_worker::scheduler::{load_scheduler_jobs, run_task_now, SchedulerMode};
use serde_json::json;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    let auth_db_path = extract_auth_db_path(&mut args)?;
    let index_db_path = extract_path_option(&mut args, "--index-db")?;
    let db_name = extract_string_option(&mut args, "--db")?;
    let state_dir = extract_path_option(&mut args, "--state-dir")?;
    let changes_file = extract_path_option(&mut args, "--changes-file")?;
    let csv_path = extract_path_option(&mut args, "--csv")?;
    let fixture_path = extract_path_option(&mut args, "--fixture")?;
    let output_db_path = extract_path_option(&mut args, "--output-db")?;
    let manifest_path = extract_path_option(&mut args, "--manifest")?;
    let run_id = extract_string_option(&mut args, "--run-id")?;
    let timestamp = extract_string_option(&mut args, "--timestamp")?;
    let has_semantic_scholar_key = extract_flag(&mut args, "--semantic-scholar-key")
        || env::var("SEMANTIC_SCHOLAR_API_KEY_POOL")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let ai_model = extract_string_option(&mut args, "--ai-model")?;
    let max_candidates = extract_usize_option(&mut args, "--max-candidates")?;
    let dedupe_retention_days =
        extract_i64_option(&mut args, "--dedupe-retention-days")?.unwrap_or(30);
    match args.as_slice() {
        [command] if command == "scheduler-dry-run" => print_scheduler_load(&auth_db_path),
        [command] if command == "scheduler-shadow" => print_scheduler_load(&auth_db_path),
        [group, command] if group == "scheduler" && command == "dry-run" => {
            print_scheduler_load(&auth_db_path)
        }
        [group, command] if group == "scheduler" && command == "shadow" => {
            print_scheduler_load(&auth_db_path)
        }
        [group, command] if group == "index" && command == "fixture" => {
            let csv_path = csv_path.ok_or("--csv is required for index fixture")?;
            let fixture_path = fixture_path.ok_or("--fixture is required for index fixture")?;
            let output_db_path =
                output_db_path.ok_or("--output-db is required for index fixture")?;
            let default_timestamp = default_timestamp();
            let outcome = run_scholarly_fixture_index(&ScholarlyIndexConfig {
                csv_path,
                fixture_path,
                output_db_path,
                manifest_path,
                run_id: run_id.unwrap_or_else(|| format!("run-{default_timestamp}")),
                timestamp: timestamp.unwrap_or(default_timestamp),
                has_semantic_scholar_key,
            })?;
            println!("{}", serde_json::to_string(&outcome)?);
            Ok(())
        }
        [group, command, task_id] if group == "scheduler" && command == "run-once" => {
            let task_id = task_id.parse::<i64>()?;
            let outcome = run_task_now(&auth_db_path, task_id, SchedulerMode::Execute)?;
            println!("{}", serde_json::to_string(&outcome)?);
            Ok(())
        }
        [group, command, task_id] if group == "scheduler" && command == "dry-run-once" => {
            let task_id = task_id.parse::<i64>()?;
            let outcome = run_task_now(&auth_db_path, task_id, SchedulerMode::DryRun)?;
            println!("{}", serde_json::to_string(&outcome)?);
            Ok(())
        }
        [group, command]
            if (group == "notify" || group == "push")
                && (command == "dry-run" || command == "shadow") =>
        {
            let workflow = if group == "notify" {
                DeliveryWorkflow::Notify
            } else {
                DeliveryWorkflow::Push
            };
            let mode = if command == "shadow" {
                DeliveryMode::Shadow
            } else {
                DeliveryMode::DryRun
            };
            let index_db_path =
                index_db_path.ok_or("--index-db is required for notification and push delivery")?;
            let db_name = db_name.ok_or("--db is required for notification and push delivery")?;
            let project_root = project_root();
            let state_dir =
                state_dir.unwrap_or_else(|| project_root.join("data").join("push_state"));
            let outcome = run_recommendation_delivery(&RecommendationRunConfig {
                auth_db_path,
                index_db_path,
                db_name,
                state_dir,
                changes_file,
                ai_model,
                max_candidates,
                dedupe_retention_days,
                mode,
                workflow,
            })?;
            println!("{}", serde_json::to_string(&outcome)?);
            Ok(())
        }
        _ => Err(usage().into()),
    }
}

fn print_scheduler_load(auth_db_path: &Path) -> Result<(), Box<dyn Error>> {
    let result = load_scheduler_jobs(auth_db_path)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn extract_auth_db_path(args: &mut Vec<String>) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(index) = args.iter().position(|argument| argument == "--auth-db") {
        if index + 1 >= args.len() {
            return Err("--auth-db requires a path".into());
        }
        let path = PathBuf::from(args.remove(index + 1));
        args.remove(index);
        return Ok(path);
    }
    let project_root = env::var("PAPER_SCANNER_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("current directory should be available"));
    Ok(project_root.join("data").join("auth.sqlite"))
}

fn extract_path_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<PathBuf>, Box<dyn Error>> {
    Ok(extract_string_option(args, name)?.map(PathBuf::from))
}

fn extract_string_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    if let Some(index) = args.iter().position(|argument| argument == name) {
        if index + 1 >= args.len() {
            return Err(format!("{name} requires a value").into());
        }
        let value = args.remove(index + 1);
        args.remove(index);
        return Ok(Some(value));
    }
    Ok(None)
}

fn extract_usize_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<usize>, Box<dyn Error>> {
    extract_string_option(args, name)?
        .map(|value| value.parse::<usize>().map_err(Into::into))
        .transpose()
}

fn extract_i64_option(args: &mut Vec<String>, name: &str) -> Result<Option<i64>, Box<dyn Error>> {
    extract_string_option(args, name)?
        .map(|value| value.parse::<i64>().map_err(Into::into))
        .transpose()
}

fn extract_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|argument| argument == name) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn project_root() -> PathBuf {
    env::var("PAPER_SCANNER_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("current directory should be available"))
}

fn default_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn usage() -> String {
    let payload = json!({
        "usage": [
            "ps-cli index fixture --csv PATH --fixture PATH --output-db PATH [--manifest PATH] [--semantic-scholar-key]",
            "ps-cli scheduler dry-run [--auth-db PATH]",
            "ps-cli scheduler shadow [--auth-db PATH]",
            "ps-cli scheduler run-once TASK_ID [--auth-db PATH]",
            "ps-cli scheduler dry-run-once TASK_ID [--auth-db PATH]",
            "ps-cli notify dry-run --auth-db PATH --index-db PATH --db NAME [--state-dir PATH]",
            "ps-cli notify shadow --auth-db PATH --index-db PATH --db NAME [--state-dir PATH]",
            "ps-cli push dry-run --auth-db PATH --index-db PATH --db NAME [--state-dir PATH]",
            "ps-cli push shadow --auth-db PATH --index-db PATH --db NAME [--state-dir PATH]"
        ]
    });
    payload.to_string()
}
