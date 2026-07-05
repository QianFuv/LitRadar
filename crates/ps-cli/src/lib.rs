//! Shared Rust backend command dispatch.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ps_index::{
    run_cnki_fixture_index, run_live_index, run_live_index_worker_from_environment,
    run_scholarly_fixture_index, CnkiIndexConfig, LiveIndexConfig, ScholarlyIndexConfig,
};
use ps_worker::delivery::{
    run_recommendation_delivery, DeliveryMode, DeliveryWorkflow, RecommendationRunConfig,
};
use ps_worker::scheduler::{
    load_scheduler_jobs, run_due_scheduler_once, run_task_now, ScheduledRunSlot, SchedulerMode,
};
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
    if run_internal_live_index_worker_if_requested()? {
        return Ok(());
    }
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
    if run_internal_live_index_worker_if_requested()? {
        return Ok(());
    }
    let mut args = args;
    if has_help(&args) {
        println!("{}", legacy_index_usage());
        return Ok(());
    }
    let options = parse_legacy_index_options(&mut args)?;
    if !args.is_empty() {
        return Err(format!("unexpected index arguments: {}", args.join(" ")).into());
    }
    let project_root = project_root();
    apply_runtime_settings(&project_root.join("data").join("auth.sqlite"));
    let outcome = run_live_index(&LiveIndexConfig {
        project_root,
        file: options.file,
        worker_count: options.worker_count,
        process_count: options.process_count,
        issue_batch_size: options.issue_batch_size,
        timeout_seconds: options.timeout_seconds,
        resume: options.resume,
        update: options.update,
        notify: options.notify,
        notify_dry_run: options.notify_dry_run,
    })?;
    println!("{}", serde_json::to_string(&outcome)?);
    Ok(())
}

fn run_internal_live_index_worker_if_requested() -> Result<bool, Box<dyn Error>> {
    let Some(response) = run_live_index_worker_from_environment()? else {
        return Ok(false);
    };
    println!("{response}");
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyIndexOptions {
    file: Option<String>,
    worker_count: usize,
    process_count: usize,
    issue_batch_size: usize,
    timeout_seconds: u64,
    resume: bool,
    update: bool,
    notify: bool,
    notify_dry_run: bool,
}

fn parse_legacy_index_options(
    args: &mut Vec<String>,
) -> Result<LegacyIndexOptions, Box<dyn Error>> {
    let file = extract_string_option_any(args, &["--file", "-f"])?;
    let worker_count = positive_usize(
        "--workers",
        extract_usize_option_any(args, &["--workers", "-w"])?,
    )?
    .unwrap_or(32);
    let issue_batch_size = positive_usize(
        "--issue-batch",
        extract_usize_option(args, "--issue-batch")?,
    )?
    .unwrap_or(worker_count);
    let timeout_seconds = extract_u64_option(args, "--timeout")?.unwrap_or(20);
    let process_count =
        positive_usize("--processes", extract_usize_option(args, "--processes")?)?.unwrap_or(2);
    let resume = extract_bool_pair(args, "--resume", "--no-resume", true);
    let update = extract_bool_pair(args, "--update", "--no-update", false);
    let notify = extract_bool_pair(args, "--notify", "--no-notify", false);
    let notify_dry_run = extract_bool_pair(args, "--notify-dry-run", "--no-notify-dry-run", false);
    if notify && !update {
        return Err("--notify requires --update".into());
    }
    Ok(LegacyIndexOptions {
        file,
        worker_count,
        process_count,
        issue_batch_size,
        timeout_seconds,
        resume,
        update,
        notify,
        notify_dry_run,
    })
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
    if matches!(args.first().map(String::as_str), Some("index"))
        && !matches!(args.get(1).map(String::as_str), Some("fixture"))
    {
        args.remove(0);
        return run_legacy_index(args);
    }
    if matches!(args.first().map(String::as_str), Some("notify" | "push")) {
        let workflow = if args.first().map(String::as_str) == Some("notify") {
            DeliveryWorkflow::Notify
        } else {
            DeliveryWorkflow::Push
        };
        args.remove(0);
        return run_legacy_delivery(workflow, args);
    }
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
    let worker_max_iterations = extract_usize_option(&mut args, "--max-iterations")?;
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
        [group, command] if group == "worker" && command == "execute" => run_worker_execute(
            &auth_db_path,
            worker_interval_seconds,
            worker_max_iterations,
        ),
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
                    worker_count: issue_batch_size,
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
                timeout_seconds: 60,
                retry_attempts: 3,
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
    let mut mode = DeliveryMode::Execute;
    if matches!(args.first().map(String::as_str), Some("dry-run" | "shadow")) {
        mode = if args.remove(0) == "shadow" {
            DeliveryMode::Shadow
        } else {
            DeliveryMode::DryRun
        };
    }
    if remove_flag(&mut args, "--dry-run") {
        mode = DeliveryMode::DryRun;
    }
    if remove_flag(&mut args, "--no-dry-run") {
        mode = DeliveryMode::Execute;
    }
    let auth_db_path = extract_auth_db_path(&mut args)?;
    let index_db_path = extract_path_option(&mut args, "--index-db")?;
    let db_name = extract_string_option(&mut args, "--db")?;
    let state_dir = extract_path_option(&mut args, "--state-dir")?;
    let changes_file = extract_path_option(&mut args, "--changes-file")?;
    let ai_model = extract_string_option(&mut args, "--ai-model")?;
    let max_candidates = extract_usize_option(&mut args, "--max-candidates")?;
    let timeout_seconds = extract_u64_option(&mut args, "--timeout")?.unwrap_or(60);
    let retry_attempts = extract_usize_option(&mut args, "--retries")?.unwrap_or(3);
    let dedupe_retention_days =
        extract_i64_option(&mut args, "--dedupe-retention-days")?.unwrap_or(60);
    if !args.is_empty() {
        return Err(format!("unexpected {command_name} arguments: {}", args.join(" ")).into());
    }

    let project_root = project_root();
    apply_runtime_settings(&auth_db_path);
    let changes_file = changes_file
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| resolve_project_path(&project_root, path));
    let state_dir = state_dir
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| resolve_project_path(&project_root, path))
        .unwrap_or_else(|| default_delivery_state_dir(&project_root, workflow));
    let targets = resolve_delivery_targets(
        &project_root,
        index_db_path,
        db_name,
        changes_file.as_deref(),
    )?;
    let mut outcomes = Vec::new();
    for target in targets {
        let outcome = run_recommendation_delivery(&RecommendationRunConfig {
            auth_db_path: auth_db_path.clone(),
            index_db_path: target.index_db_path,
            db_name: target.db_name,
            state_dir: state_dir.clone(),
            changes_file: changes_file.clone(),
            ai_model: ai_model.clone(),
            max_candidates,
            timeout_seconds,
            retry_attempts,
            dedupe_retention_days,
            mode,
            workflow,
        })?;
        outcomes.push(outcome);
    }
    let payload = json!({
        "workflow": workflow,
        "mode": mode,
        "status": if outcomes.iter().all(|outcome| outcome.status == "idle") { "idle" } else { "completed" },
        "databases": outcomes,
    });
    println!("{}", serde_json::to_string(&payload)?);
    Ok(())
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

fn run_worker_execute(
    auth_db_path: &Path,
    interval_seconds: u64,
    max_iterations: Option<usize>,
) -> Result<(), Box<dyn Error>> {
    let interval_seconds = interval_seconds.max(1);
    let mut executed_slots = BTreeSet::<ScheduledRunSlot>::new();
    let mut iterations = 0_usize;
    loop {
        let result = run_due_scheduler_once(auth_db_path, &mut executed_slots)?;
        let payload = json!({
            "interval_seconds": interval_seconds,
            "mode": "execute",
            "status": result.status,
            "minute_epoch": result.minute_epoch,
            "jobs": result.jobs,
            "skipped": result.skipped.len(),
            "due": result.due,
            "already_executed": result.already_executed,
            "executed": result.executed.len(),
            "executions": result.executed,
        });
        println!("{}", serde_json::to_string(&payload)?);
        iterations += 1;
        if max_iterations.is_some_and(|limit| iterations >= limit) {
            return Ok(());
        }
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

fn extract_string_option_any(
    args: &mut Vec<String>,
    names: &[&str],
) -> Result<Option<String>, Box<dyn Error>> {
    for name in names {
        if let Some(value) = extract_string_option(args, name)? {
            return Ok(Some(value));
        }
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

fn extract_usize_option_any(
    args: &mut Vec<String>,
    names: &[&str],
) -> Result<Option<usize>, Box<dyn Error>> {
    extract_string_option_any(args, names)?
        .map(|value| value.parse::<usize>().map_err(Into::into))
        .transpose()
}

fn positive_usize(name: &str, value: Option<usize>) -> Result<Option<usize>, Box<dyn Error>> {
    match value {
        Some(0) => Err(format!("{name} must be at least 1").into()),
        other => Ok(other),
    }
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

fn extract_bool_pair(args: &mut Vec<String>, yes_name: &str, no_name: &str, default: bool) -> bool {
    let mut value = default;
    if remove_flag(args, yes_name) {
        value = true;
    }
    if remove_flag(args, no_name) {
        value = false;
    }
    value
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

fn apply_runtime_settings(auth_db_path: &Path) {
    let Ok(settings) = ps_storage::list_runtime_settings(auth_db_path) else {
        return;
    };
    for setting in settings {
        if setting.source != "database" {
            continue;
        }
        if setting.value.trim().is_empty() {
            env::remove_var(setting.key);
        } else {
            env::set_var(setting.key, setting.value);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveryTarget {
    index_db_path: PathBuf,
    db_name: String,
}

fn resolve_delivery_targets(
    project_root: &Path,
    index_db_path: Option<PathBuf>,
    db_name: Option<String>,
    changes_file: Option<&Path>,
) -> Result<Vec<DeliveryTarget>, Box<dyn Error>> {
    if let Some(index_db_path) = index_db_path {
        let index_db_path = resolve_project_path(project_root, index_db_path);
        let db_name = db_name
            .map(|value| normalize_db_name(&value))
            .unwrap_or_else(|| {
                index_db_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| "index.sqlite".to_string())
            });
        return Ok(vec![DeliveryTarget {
            index_db_path,
            db_name,
        }]);
    }
    if let Some(db_name) = db_name {
        let db_name = normalize_db_name(&db_name);
        let index_db_path = project_root.join("data").join("index").join(&db_name);
        if !index_db_path.exists() {
            return Err("Database not found".into());
        }
        return Ok(vec![DeliveryTarget {
            index_db_path,
            db_name,
        }]);
    }
    if let Some(changes_file) = changes_file {
        let db_name = db_name_from_manifest(changes_file)?;
        let index_db_path = project_root.join("data").join("index").join(&db_name);
        if !index_db_path.exists() {
            return Err("Database not found".into());
        }
        return Ok(vec![DeliveryTarget {
            index_db_path,
            db_name,
        }]);
    }
    let index_dir = project_root.join("data").join("index");
    fs::create_dir_all(&index_dir)?;
    let mut targets = Vec::new();
    for entry in fs::read_dir(&index_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("sqlite") {
            continue;
        }
        let Some(db_name) = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        targets.push(DeliveryTarget {
            index_db_path: path,
            db_name,
        });
    }
    targets.sort_by(|left, right| left.db_name.cmp(&right.db_name));
    if targets.is_empty() {
        return Err("No SQLite databases found".into());
    }
    Ok(targets)
}

fn db_name_from_manifest(path: &Path) -> Result<String, Box<dyn Error>> {
    let payload: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let db_name = payload
        .get("db_name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Change manifest missing db_name; specify --db explicitly")?;
    Ok(normalize_db_name(db_name))
}

fn normalize_db_name(value: &str) -> String {
    let mut db_name = Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(value)
        .trim()
        .to_string();
    if !db_name.ends_with(".sqlite") {
        db_name.push_str(".sqlite");
    }
    db_name
}

fn resolve_project_path(project_root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

fn default_delivery_state_dir(project_root: &Path, workflow: DeliveryWorkflow) -> PathBuf {
    match workflow {
        DeliveryWorkflow::Notify => project_root.join("data").join("push_state"),
        DeliveryWorkflow::Push => project_root.join("data").join("folder_push_state"),
    }
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
            "ps-cli worker execute [--auth-db PATH] [--interval-seconds N] [--max-iterations N]",
            "index [--file FILE] [--workers N] [--processes N] [--issue-batch N] [--timeout N] [--resume|--no-resume] [--update|--no-update] [--notify] [--notify-dry-run]",
            "notify [--db NAME] [--state-dir PATH] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--timeout N] [--retries N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]",
            "push [--db NAME] [--state-dir PATH] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--timeout N] [--retries N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]"
        ]
    });
    payload.to_string()
}

fn legacy_index_usage() -> String {
    let payload = json!({
        "usage": "index [--file FILE] [--workers N] [--processes N] [--issue-batch N] [--timeout N] [--resume|--no-resume] [--update|--no-update] [--notify] [--notify-dry-run]"
    });
    payload.to_string()
}

fn legacy_delivery_usage(workflow: DeliveryWorkflow) -> String {
    let command_name = match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    };
    let payload = json!({
        "usage": format!("{command_name} [--db NAME] [--state-dir PATH] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--timeout N] [--retries N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]")
    });
    payload.to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::{Builder, TempDir};

    use super::{
        default_delivery_state_dir, extract_auth_db_path, extract_bool_pair, extract_string_option,
        extract_usize_option, grouped_usage, legacy_delivery_usage, legacy_index_usage,
        normalize_db_name, parse_legacy_index_options, resolve_delivery_targets,
        resolve_project_path, run_legacy_index, run_legacy_notify, run_ps_cli,
    };
    use ps_worker::delivery::DeliveryWorkflow;

    #[test]
    fn grouped_usage_hides_fixture_migration_command() {
        let usage = grouped_usage();

        assert!(!usage.contains("index fixture"));
        assert!(!usage.contains("PAPER_SCANNER_LIVE_INDEX_WORKER_REQUEST"));
        assert!(usage.contains("index [--file FILE]"));
        assert!(usage.contains("notify [--db NAME]"));
        assert!(usage.contains("push [--db NAME]"));
        assert!(usage.contains("ps-cli worker execute"));
    }

    #[test]
    fn legacy_index_usage_exposes_old_flags() {
        let usage = legacy_index_usage();

        assert!(usage.contains("--file FILE"));
        assert!(usage.contains("--workers N"));
        assert!(usage.contains("--processes N"));
        assert!(usage.contains("--notify-dry-run"));
        assert!(!usage.contains("--fixture"));
        assert!(!usage.contains("PAPER_SCANNER_LIVE_INDEX_WORKER_REQUEST"));
    }

    #[test]
    fn legacy_index_options_preserve_concurrency_flags() {
        let mut args = vec![
            "--file".to_string(),
            "selected.csv".to_string(),
            "--workers".to_string(),
            "4".to_string(),
            "--processes".to_string(),
            "3".to_string(),
            "--issue-batch".to_string(),
            "2".to_string(),
            "--timeout".to_string(),
            "7".to_string(),
            "--no-resume".to_string(),
            "--update".to_string(),
            "--notify".to_string(),
            "--notify-dry-run".to_string(),
        ];

        let options = parse_legacy_index_options(&mut args).expect("index options should parse");

        assert!(args.is_empty());
        assert_eq!(options.file.as_deref(), Some("selected.csv"));
        assert_eq!(options.worker_count, 4);
        assert_eq!(options.process_count, 3);
        assert_eq!(options.issue_batch_size, 2);
        assert_eq!(options.timeout_seconds, 7);
        assert!(!options.resume);
        assert!(options.update);
        assert!(options.notify);
        assert!(options.notify_dry_run);
    }

    #[test]
    fn legacy_index_options_default_issue_batch_to_workers() {
        let mut args = vec!["--workers".to_string(), "5".to_string()];

        let options = parse_legacy_index_options(&mut args).expect("index options should parse");

        assert_eq!(options.worker_count, 5);
        assert_eq!(options.process_count, 2);
        assert_eq!(options.issue_batch_size, 5);
    }

    #[test]
    fn legacy_index_options_reject_zero_parallelism_values() {
        for arguments in [
            vec!["--workers".to_string(), "0".to_string()],
            vec!["--processes".to_string(), "0".to_string()],
            vec!["--issue-batch".to_string(), "0".to_string()],
        ] {
            let mut arguments = arguments;
            let error =
                parse_legacy_index_options(&mut arguments).expect_err("zero value should fail");

            assert!(error.to_string().contains("must be at least 1"));
        }
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

    #[test]
    fn delivery_targets_resolve_manifest_database() {
        let root = temp_root("ps-cli-targets");
        let index_dir = root.path().join("data").join("index");
        fs::create_dir_all(&index_dir).expect("index dir should be created");
        fs::write(index_dir.join("alpha.sqlite"), "").expect("db file should be created");
        let manifest = root.path().join("manifest.json");
        fs::write(&manifest, r#"{"db_name":"alpha"}"#).expect("manifest should be created");

        let targets = resolve_delivery_targets(root.path(), None, None, Some(&manifest))
            .expect("manifest target should resolve");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].db_name, "alpha.sqlite");
    }

    #[test]
    fn delivery_targets_scan_all_databases_in_name_order() {
        let root = temp_root("ps-cli-all-dbs");
        let index_dir = root.path().join("data").join("index");
        fs::create_dir_all(&index_dir).expect("index dir should be created");
        fs::write(index_dir.join("zeta.sqlite"), "").expect("db file should be created");
        fs::write(index_dir.join("alpha.sqlite"), "").expect("db file should be created");

        let targets = resolve_delivery_targets(root.path(), None, None, None)
            .expect("targets should resolve");

        assert_eq!(
            targets
                .iter()
                .map(|target| target.db_name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha.sqlite", "zeta.sqlite"]
        );
    }

    #[test]
    fn delivery_defaults_match_legacy_commands() {
        let root = std::path::Path::new("/tmp/project");

        assert_eq!(normalize_db_name("utd24"), "utd24.sqlite");
        assert_eq!(normalize_db_name("data/index/utd24.sqlite"), "utd24.sqlite");
        assert_eq!(
            default_delivery_state_dir(root, DeliveryWorkflow::Notify),
            root.join("data").join("push_state")
        );
        assert_eq!(
            default_delivery_state_dir(root, DeliveryWorkflow::Push),
            root.join("data").join("folder_push_state")
        );
    }

    #[test]
    fn option_extractors_remove_values_and_report_parse_errors() {
        let mut args = vec![
            "--limit".to_string(),
            "25".to_string(),
            "--name".to_string(),
            "daily".to_string(),
            "tail".to_string(),
        ];

        let limit = extract_usize_option(&mut args, "--limit")
            .expect("limit should parse")
            .expect("limit should be present");
        let name = extract_string_option(&mut args, "--name")
            .expect("name should parse")
            .expect("name should be present");

        assert_eq!(limit, 25);
        assert_eq!(name, "daily");
        assert_eq!(args, ["tail"]);

        let mut missing_value = vec!["--limit".to_string()];
        let missing_error = extract_usize_option(&mut missing_value, "--limit")
            .expect_err("missing option value should fail");
        assert_eq!(missing_error.to_string(), "--limit requires a value");

        let mut invalid_value = vec!["--limit".to_string(), "NaN".to_string()];
        let invalid_error = extract_usize_option(&mut invalid_value, "--limit")
            .expect_err("invalid usize should fail");
        assert!(invalid_error.to_string().contains("invalid digit"));
    }

    #[test]
    fn bool_pair_uses_no_flag_as_final_override() {
        let mut args = vec![
            "--dry-run".to_string(),
            "--no-dry-run".to_string(),
            "tail".to_string(),
        ];

        let value = extract_bool_pair(&mut args, "--dry-run", "--no-dry-run", false);

        assert!(!value);
        assert_eq!(args, ["tail"]);
    }

    #[test]
    fn auth_db_extractor_prefers_explicit_path_and_reports_missing_values() {
        let mut args = vec![
            "--auth-db".to_string(),
            "data/auth.sqlite".to_string(),
            "tail".to_string(),
        ];

        let path = extract_auth_db_path(&mut args).expect("auth db path should parse");

        assert_eq!(path, std::path::PathBuf::from("data/auth.sqlite"));
        assert_eq!(args, ["tail"]);

        let mut missing = vec!["--auth-db".to_string()];
        let error =
            extract_auth_db_path(&mut missing).expect_err("missing auth db value should fail");
        assert_eq!(error.to_string(), "--auth-db requires a path");
    }

    #[test]
    fn manifest_target_requires_database_name() {
        let root = temp_root("ps-cli-missing-manifest-db");
        let manifest = root.path().join("manifest.json");
        fs::write(&manifest, r#"{"generated_at":"2026-07-05T00:00:00Z"}"#)
            .expect("manifest should be created");

        let error = resolve_delivery_targets(root.path(), None, None, Some(&manifest))
            .expect_err("manifest without db_name should fail");

        assert_eq!(
            error.to_string(),
            "Change manifest missing db_name; specify --db explicitly"
        );
    }

    #[test]
    fn delivery_target_resolution_reports_missing_databases() {
        let root = temp_root("ps-cli-missing-db-targets");
        let manifest = root.path().join("manifest.json");
        fs::write(&manifest, r#"{"db_name":"missing.sqlite"}"#)
            .expect("manifest should be created");

        let by_name =
            resolve_delivery_targets(root.path(), None, Some("missing".to_string()), None)
                .expect_err("missing db name target should fail");
        let by_manifest = resolve_delivery_targets(root.path(), None, None, Some(&manifest))
            .expect_err("missing manifest db target should fail");

        assert_eq!(by_name.to_string(), "Database not found");
        assert_eq!(by_manifest.to_string(), "Database not found");
    }

    #[test]
    fn project_path_resolution_keeps_absolute_paths_and_joins_relative_paths() {
        let root = temp_root("ps-cli-project-path");
        let absolute_path = root.path().join("absolute.sqlite");
        let relative_path = std::path::PathBuf::from("data/index/alpha.sqlite");

        assert_eq!(
            resolve_project_path(root.path(), absolute_path.clone()),
            absolute_path
        );
        assert_eq!(
            resolve_project_path(root.path(), relative_path.clone()),
            root.path().join(relative_path)
        );
    }

    #[test]
    fn legacy_index_notify_requires_update_before_live_execution() {
        let error = run_legacy_index(vec!["--notify".to_string()])
            .expect_err("notify handoff should require update mode");

        assert_eq!(error.to_string(), "--notify requires --update");
    }

    #[test]
    fn help_and_legacy_delivery_errors_return_before_execution() {
        run_ps_cli(vec!["--help".to_string()]).expect("grouped help should succeed");

        let error = run_legacy_notify(vec!["--unexpected".to_string()])
            .expect_err("unexpected legacy delivery args should fail");

        assert_eq!(
            error.to_string(),
            "unexpected notify arguments: --unexpected"
        );
    }

    #[test]
    fn scheduler_dispatch_requires_valid_task_id() {
        let root = temp_root("ps-cli-scheduler-dispatch");
        let auth_db_path = root.path().join("auth.sqlite");
        ps_storage::initialize_auth_database(&auth_db_path)
            .expect("auth database should initialize");

        let error = run_ps_cli(vec![
            "--auth-db".to_string(),
            auth_db_path.to_string_lossy().into_owned(),
            "scheduler".to_string(),
            "run-once".to_string(),
            "not-a-number".to_string(),
        ])
        .expect_err("invalid scheduler task id should fail before execution");

        assert!(error.to_string().contains("invalid digit"));
    }

    #[test]
    fn scheduler_worker_execute_dispatch_supports_finite_test_iterations() {
        let root = temp_root("ps-cli-worker-execute");
        let auth_db_path = root.path().join("auth.sqlite");
        ps_storage::initialize_auth_database(&auth_db_path)
            .expect("auth database should initialize");

        run_ps_cli(vec![
            "--auth-db".to_string(),
            auth_db_path.to_string_lossy().into_owned(),
            "--interval-seconds".to_string(),
            "1".to_string(),
            "--max-iterations".to_string(),
            "1".to_string(),
            "worker".to_string(),
            "execute".to_string(),
        ])
        .expect("finite worker execute should return");
    }

    #[test]
    fn delivery_dispatch_requires_index_database_and_db_name() {
        let root = temp_root("ps-cli-delivery-dispatch");
        let auth_db_path = root.path().join("auth.sqlite");
        ps_storage::initialize_auth_database(&auth_db_path)
            .expect("auth database should initialize");

        let missing_index = run_ps_cli(vec![
            "--auth-db".to_string(),
            auth_db_path.to_string_lossy().into_owned(),
            "notify".to_string(),
            "dry-run".to_string(),
            "--db".to_string(),
            "fixture".to_string(),
        ])
        .expect_err("notify dry-run should require an index database");

        assert!(missing_index.to_string().contains("--index-db is required"));

        let missing_db = run_ps_cli(vec![
            "--auth-db".to_string(),
            auth_db_path.to_string_lossy().into_owned(),
            "--index-db".to_string(),
            root.path()
                .join("fixture.sqlite")
                .to_string_lossy()
                .into_owned(),
            "push".to_string(),
            "shadow".to_string(),
        ])
        .expect_err("push shadow should require a database name");

        assert!(missing_db.to_string().contains("--db is required"));
    }

    #[test]
    fn grouped_dispatch_reports_unknown_worker_command() {
        let root = temp_root("ps-cli-worker-dispatch");
        let auth_db_path = root.path().join("auth.sqlite");
        ps_storage::initialize_auth_database(&auth_db_path)
            .expect("auth database should initialize");

        let error = run_ps_cli(vec![
            "--auth-db".to_string(),
            auth_db_path.to_string_lossy().into_owned(),
            "worker".to_string(),
            "dry-run".to_string(),
        ])
        .expect_err("unknown worker command should return usage");

        assert!(error.to_string().contains("ps-cli worker shadow"));
    }

    fn temp_root(prefix: &str) -> TempDir {
        Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("temp root should be created")
    }
}
