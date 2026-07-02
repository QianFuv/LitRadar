//! Rust backend command entrypoints.

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

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
    match args.as_slice() {
        [command] if command == "scheduler-dry-run" => print_scheduler_load(&auth_db_path),
        [command] if command == "scheduler-shadow" => print_scheduler_load(&auth_db_path),
        [group, command] if group == "scheduler" && command == "dry-run" => {
            print_scheduler_load(&auth_db_path)
        }
        [group, command] if group == "scheduler" && command == "shadow" => {
            print_scheduler_load(&auth_db_path)
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

fn usage() -> String {
    let payload = json!({
        "usage": [
            "ps-cli scheduler dry-run [--auth-db PATH]",
            "ps-cli scheduler shadow [--auth-db PATH]",
            "ps-cli scheduler run-once TASK_ID [--auth-db PATH]",
            "ps-cli scheduler dry-run-once TASK_ID [--auth-db PATH]"
        ]
    });
    payload.to_string()
}
