//! Shared Rust backend command dispatch.

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ps_index::{
    run_cnki_fixture_index, run_scholarly_fixture_index, CnkiIndexConfig, ScholarlyIndexConfig,
};
use ps_worker::delivery::{
    run_recommendation_delivery, DeliveryMode, DeliveryWorkflow, RecommendationRunConfig,
};
use ps_worker::scheduler::{load_scheduler_jobs, run_task_now, SchedulerMode};
use serde_json::json;

/// Run the grouped `ps-cli` command dispatcher.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_ps_cli(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_grouped_command(args)
}

/// Run the legacy `index` command dispatcher.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_legacy_index(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if has_help(&args) {
        println!("{}", legacy_index_usage());
        return Ok(());
    }
    Err("legacy index execution will be restored by the live Rust index task".into())
}

/// Run the legacy `notify` command dispatcher.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_legacy_notify(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_legacy_delivery(DeliveryWorkflow::Notify, args)
}

/// Run the legacy `push` command dispatcher.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_legacy_push(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_legacy_delivery(DeliveryWorkflow::Push, args)
}

fn run_grouped_command(mut args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if has_help(&args) {
        println!("{}", grouped_usage());
        return Ok(());
    }
    let auth_db_path = extract_auth_db_path(&mut args)?;
    let index_db_path = extract_path_option(&mut args, "--index-db")?;
    let db_name = extract_string_option(&mut args, "--db")?;
    let state_dir = extract_path_option(&mut args, "--state-dir")?;
    let changes_file = extract_path_option(&mut args, "--changes-file")?;
    let csv_path = extract_path_option(&mut args, "--csv")?;
    let fixture_path = extract_path_option(&mut args, "--fixture")?;
    let output_db_path = extract_path_option(&mut args, "--output-db")?;
    let manifest_path = extract_path_option(&mut args, "--manifest")?;
    let index_source = extract_string_option(&mut args, "--source")?;
    let run_id = extract_string_option(&mut args, "--run-id")?;
    let timestamp = extract_string_option(&mut args, "--timestamp")?;
    let resume_index = extract_flag(&mut args, "--resume");
    let update_index = extract_flag(&mut args, "--update");
    let issue_batch_size = extract_usize_option(&mut args, "--issue-batch-size")?.unwrap_or(10);
    let worker_interval_seconds =
        extract_u64_option(&mut args, "--interval-seconds")?.unwrap_or(300);
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
        [group, command] if group == "worker" && command == "shadow" => {
            run_worker_shadow(&auth_db_path, worker_interval_seconds)
        }
        [group, command] if group == "index" && command == "fixture" => {
            let csv_path = csv_path.ok_or("--csv is required for index fixture")?;
            let fixture_path = fixture_path.ok_or("--fixture is required for index fixture")?;
            let output_db_path =
                output_db_path.ok_or("--output-db is required for index fixture")?;
            let default_timestamp = default_timestamp();
            let run_id = run_id.unwrap_or_else(|| format!("run-{default_timestamp}"));
            let timestamp = timestamp.unwrap_or(default_timestamp);
            let source = index_source.as_deref().unwrap_or("scholarly");
            let payload = match source {
                "scholarly" => {
                    serde_json::to_value(run_scholarly_fixture_index(&ScholarlyIndexConfig {
                        csv_path,
                        fixture_path,
                        output_db_path,
                        manifest_path,
                        run_id,
                        timestamp,
                        has_semantic_scholar_key,
                    })?)?
                }
                "cnki" => serde_json::to_value(run_cnki_fixture_index(&CnkiIndexConfig {
                    csv_path,
                    fixture_path,
                    output_db_path,
                    manifest_path,
                    run_id,
                    timestamp,
                    resume: resume_index,
                    update: update_index,
                    issue_batch_size,
                })?)?,
                other => return Err(format!("unsupported index fixture source: {other}").into()),
            };
            println!("{}", serde_json::to_string(&payload)?);
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
        _ => Err(grouped_usage().into()),
    }
}

fn run_legacy_delivery(
    workflow: DeliveryWorkflow,
    mut args: Vec<String>,
) -> Result<(), Box<dyn Error>> {
    if has_help(&args) {
        println!("{}", legacy_delivery_usage(workflow));
        return Ok(());
    }
    let command_name = match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    };
    let mut mode = None;
    if matches!(args.first().map(String::as_str), Some("dry-run" | "shadow")) {
        mode = Some(args.remove(0));
    }
    if remove_flag(&mut args, "--dry-run") {
        mode = Some("dry-run".to_string());
    }
    let Some(mode) = mode else {
        return Err(format!(
            "legacy {command_name} execution will be restored by the delivery behavior task"
        )
        .into());
    };
    let mut grouped_args = vec![command_name.to_string(), mode];
    grouped_args.extend(args);
    run_grouped_command(grouped_args)
}

fn print_scheduler_load(auth_db_path: &Path) -> Result<(), Box<dyn Error>> {
    let result = load_scheduler_jobs(auth_db_path)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn run_worker_shadow(auth_db_path: &Path, interval_seconds: u64) -> Result<(), Box<dyn Error>> {
    let interval_seconds = interval_seconds.max(1);
    loop {
        let result = load_scheduler_jobs(auth_db_path)?;
        let payload = json!({
            "interval_seconds": interval_seconds,
            "jobs": result.jobs.len(),
            "mode": "shadow",
            "skipped": result.skipped.len(),
            "status": "running"
        });
        println!("{}", serde_json::to_string(&payload)?);
        thread::sleep(Duration::from_secs(interval_seconds));
    }
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

fn extract_u64_option(args: &mut Vec<String>, name: &str) -> Result<Option<u64>, Box<dyn Error>> {
    extract_string_option(args, name)?
        .map(|value| value.parse::<u64>().map_err(Into::into))
        .transpose()
}

fn extract_i64_option(args: &mut Vec<String>, name: &str) -> Result<Option<i64>, Box<dyn Error>> {
    extract_string_option(args, name)?
        .map(|value| value.parse::<i64>().map_err(Into::into))
        .transpose()
}

fn extract_flag(args: &mut Vec<String>, name: &str) -> bool {
    remove_flag(args, name)
}

fn remove_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|argument| argument == name) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn has_help(args: &[String]) -> bool {
    args.iter()
        .any(|argument| argument == "--help" || argument == "-h")
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

fn grouped_usage() -> String {
    let payload = json!({
        "usage": [
            "ps-cli scheduler dry-run [--auth-db PATH]",
            "ps-cli scheduler shadow [--auth-db PATH]",
            "ps-cli scheduler run-once TASK_ID [--auth-db PATH]",
            "ps-cli scheduler dry-run-once TASK_ID [--auth-db PATH]",
            "ps-cli worker shadow [--auth-db PATH] [--interval-seconds N]",
            "index [--file FILE] [--workers N] [--issue-batch N] [--timeout N] [--resume|--no-resume] [--update|--no-update] [--notify] [--notify-dry-run]",
            "notify [--db NAME] [--state-dir PATH] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]",
            "push [--db NAME] [--state-dir PATH] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]"
        ]
    });
    payload.to_string()
}

fn legacy_index_usage() -> String {
    let payload = json!({
        "usage": "index [--file FILE] [--workers N] [--issue-batch N] [--timeout N] [--resume|--no-resume] [--update|--no-update] [--notify] [--notify-dry-run]"
    });
    payload.to_string()
}

fn legacy_delivery_usage(workflow: DeliveryWorkflow) -> String {
    let command_name = match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    };
    let payload = json!({
        "usage": format!("{command_name} [--db NAME] [--state-dir PATH] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]")
    });
    payload.to_string()
}

#[cfg(test)]
mod tests {
    use super::{grouped_usage, legacy_delivery_usage, legacy_index_usage};
    use ps_worker::delivery::DeliveryWorkflow;

    #[test]
    fn grouped_usage_hides_fixture_migration_command() {
        let usage = grouped_usage();

        assert!(!usage.contains("index fixture"));
        assert!(usage.contains("index [--file FILE]"));
        assert!(usage.contains("notify [--db NAME]"));
        assert!(usage.contains("push [--db NAME]"));
    }

    #[test]
    fn legacy_index_usage_exposes_old_flags() {
        let usage = legacy_index_usage();

        assert!(usage.contains("--file FILE"));
        assert!(usage.contains("--workers N"));
        assert!(usage.contains("--notify-dry-run"));
        assert!(!usage.contains("--fixture"));
    }

    #[test]
    fn legacy_delivery_usage_exposes_old_flags() {
        let notify_usage = legacy_delivery_usage(DeliveryWorkflow::Notify);
        let push_usage = legacy_delivery_usage(DeliveryWorkflow::Push);

        assert!(notify_usage.contains("notify [--db NAME]"));
        assert!(push_usage.contains("push [--db NAME]"));
        assert!(notify_usage.contains("--dry-run|--no-dry-run"));
        assert!(push_usage.contains("--changes-file PATH"));
    }
}
