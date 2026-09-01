//! Shared Rust backend command entrypoints.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::Instant;

use litradar_auth::AuthService;
use litradar_domain::{DELIVERY_RETRY_ATTEMPTS_MAX, DELIVERY_RETRY_ATTEMPTS_MIN};
use litradar_index::live::ProviderProxySelection;
use litradar_index::{
    run_live_index, run_live_index_worker_from_file_path, LiveIndexConfig, LiveIndexOutcome,
    LiveScholarlyConfig,
};
use litradar_storage::{
    create_backup, migrate_auth_database, migrate_database_secrets, optimize_index_storage,
    preflight_existing_index_databases, preflight_index_database, preflight_storage,
    restore_backup, rotate_database_secrets, verify_backup, verify_database_secrets,
    BackupCreateOptions, BackupRestoreOptions, IndexStorageOptimizationError,
    IndexStorageOptimizationOptions, IndexStorageOptimizationOutcome, ManagedMetaAction,
    ManagedMetaPreparationReport, SecretCodec, StorageConfig,
};
use litradar_worker::delivery::{
    run_manual_delivery_job, run_recommendation_delivery, DeliveryMode, DeliveryOutcomeState,
    DeliveryWorkflow, RecommendationRunConfig,
};
use litradar_worker::scheduler::{load_scheduler_jobs, run_task_now, SchedulerMode};
use serde_json::json;

const DEFAULT_INDEX_WORKER_COUNT: usize = 6;
const DEFAULT_INDEX_PROCESS_COUNT: usize = 1;
const DEFAULT_INDEX_ISSUE_BATCH_SIZE: usize = 8;

#[derive(Debug)]
struct DeliveryCommandStatusError {
    command: &'static str,
    status: DeliveryOutcomeState,
}

impl std::fmt::Display for DeliveryCommandStatusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} delivery finished with {} status",
            self.command,
            self.status.as_str()
        )
    }
}

impl Error for DeliveryCommandStatusError {}

fn run_cli_command(
    command: &'static str,
    operation: impl FnOnce() -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let span = tracing::info_span!("cli.command", component = "cli", command);
    span.in_scope(|| {
        let started_at = Instant::now();
        tracing::info!(event = "cli.command.started", component = "cli", command,);
        let result = operation();
        let duration_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        match &result {
            Ok(()) => tracing::info!(
                event = "cli.command.completed",
                component = "cli",
                command,
                outcome = "success",
                duration_ms,
            ),
            Err(_) => tracing::error!(
                event = "cli.command.failed",
                component = "cli",
                command,
                outcome = "failure",
                error_kind = "command_failed",
                duration_ms,
            ),
        }
        result
    })
}

fn print_help(help: &str) {
    println!("{help}");
}

fn print_result(result: &str) {
    println!("{result}");
}

/// Run the unified application's local `admin` maintenance command.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_admin_command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_cli_command("admin", move || run_admin_command_inner(args))
}

fn run_admin_command_inner(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if has_help(&args) {
        print_help(&admin_usage());
        return Ok(());
    }
    let stdin = std::io::stdin();
    match run_admin_command_with_reader(args, stdin.lock()) {
        Ok(payload) => {
            print_result(&serde_json::to_string(&payload)?);
            Ok(())
        }
        Err(error) => {
            if let Some(optimization_error) = error.downcast_ref::<IndexStorageOptimizationError>()
            {
                print_result(&serde_json::to_string(
                    &index_optimization_failure_payload(optimization_error),
                )?);
            }
            Err(error)
        }
    }
}

fn index_optimization_failure_payload(error: &IndexStorageOptimizationError) -> serde_json::Value {
    json!({
        "status": "failed",
        "error": {
            "code": error.code(),
            "message": error.to_string(),
            "recovery_paths": error.recovery_paths(),
        }
    })
}

fn run_admin_command_with_reader(
    mut args: Vec<String>,
    mut password_reader: impl BufRead,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let project_root = extract_project_root(&mut args)?;
    let has_explicit_auth_db = args.iter().any(|argument| argument == "--auth-db");
    let auth_db_path = extract_auth_db_path_with_project_root(&mut args, &project_root)?;
    let username = extract_string_option(&mut args, "--username")?;
    let should_read_password = remove_flag(&mut args, "--password-stdin");
    let secret_key_file = extract_path_option(&mut args, "--secret-key-file")?;
    let old_key_file = extract_path_option(&mut args, "--old-key-file")?;
    let new_key_file = extract_path_option(&mut args, "--new-key-file")?;
    let output_dir = extract_path_option(&mut args, "--output")?;
    let backup_dir = extract_path_option(&mut args, "--backup")?;
    let include_index_databases = remove_flag(&mut args, "--include-indexes");
    let include_push_state = remove_flag(&mut args, "--include-push-state");
    let is_restore_confirmed = remove_flag(&mut args, "--confirm-restore");
    let is_index_maintenance_confirmed = remove_flag(&mut args, "--confirm-index-maintenance");
    let has_backup_options = output_dir.is_some()
        || backup_dir.is_some()
        || include_index_databases
        || include_push_state
        || is_restore_confirmed
        || is_index_maintenance_confirmed;
    match args.as_slice() {
        [command]
            if command == "bootstrap"
                && username.is_some()
                && should_read_password
                && !has_backup_options =>
        {
            migrate_auth_database(&auth_db_path)?;
            let mut password = String::new();
            if password_reader.read_line(&mut password)? == 0 {
                return Err("password stdin was empty".into());
            }
            while password.ends_with(['\r', '\n']) {
                password.pop();
            }
            let user = AuthService::new(&auth_db_path)
                .bootstrap_admin(username.as_deref().unwrap_or_default().trim(), &password)?;
            Ok(json!({"status": "created", "user": user}))
        }
        [group, command]
            if group == "secrets"
                && command == "migrate"
                && secret_key_file.is_some()
                && username.is_none()
                && !should_read_password
                && !has_backup_options =>
        {
            migrate_auth_database(&auth_db_path)?;
            let codec = SecretCodec::load(secret_key_file.as_ref().expect("checked key path"))?;
            let report = migrate_database_secrets(&auth_db_path, &codec)?;
            Ok(json!({
                "status": "migrated",
                "migrated": report.migrated,
                "verified": report.verified,
                "empty": report.empty,
            }))
        }
        [group, command]
            if group == "secrets"
                && command == "verify"
                && secret_key_file.is_some()
                && username.is_none()
                && !should_read_password
                && !has_backup_options =>
        {
            migrate_auth_database(&auth_db_path)?;
            let codec = SecretCodec::load(secret_key_file.as_ref().expect("checked key path"))?;
            let report = verify_database_secrets(&auth_db_path, &codec)?;
            Ok(json!({
                "status": "verified",
                "verified": report.verified,
                "empty": report.empty,
            }))
        }
        [group, command]
            if group == "secrets"
                && command == "rotate"
                && old_key_file.is_some()
                && new_key_file.is_some()
                && username.is_none()
                && !should_read_password
                && !has_backup_options =>
        {
            migrate_auth_database(&auth_db_path)?;
            let old_codec = SecretCodec::load(old_key_file.as_ref().expect("checked old path"))?;
            let new_codec = SecretCodec::load(new_key_file.as_ref().expect("checked new path"))?;
            let rotated = rotate_database_secrets(&auth_db_path, &old_codec, &new_codec)?;
            Ok(json!({"status": "rotated", "rotated": rotated}))
        }
        [group, command]
            if group == "backup"
                && command == "create"
                && output_dir.is_some()
                && backup_dir.is_none()
                && !is_restore_confirmed
                && !is_index_maintenance_confirmed
                && username.is_none()
                && !should_read_password
                && secret_key_file.is_none()
                && old_key_file.is_none()
                && new_key_file.is_none() =>
        {
            let output_dir = resolve_project_path(
                &project_root,
                output_dir.as_ref().expect("checked output path").clone(),
            );
            let manifest = create_backup(&BackupCreateOptions {
                storage_config: StorageConfig::from_project_root(&project_root),
                auth_db_path,
                output_dir: output_dir.clone(),
                include_index_databases,
                include_push_state,
            })?;
            Ok(json!({
                "status": "created",
                "backup": output_dir,
                "manifest": manifest,
            }))
        }
        [group, command]
            if group == "backup"
                && command == "verify"
                && backup_dir.is_some()
                && output_dir.is_none()
                && !include_index_databases
                && !include_push_state
                && !is_restore_confirmed
                && !is_index_maintenance_confirmed
                && username.is_none()
                && !should_read_password
                && secret_key_file.is_none()
                && old_key_file.is_none()
                && new_key_file.is_none() =>
        {
            let backup_dir = resolve_project_path(
                &project_root,
                backup_dir.as_ref().expect("checked backup path").clone(),
            );
            let manifest = verify_backup(&backup_dir)?;
            Ok(json!({
                "status": "verified",
                "backup": backup_dir,
                "manifest": manifest,
            }))
        }
        [group, command]
            if group == "backup"
                && command == "restore"
                && backup_dir.is_some()
                && output_dir.is_none()
                && !include_index_databases
                && !include_push_state
                && is_restore_confirmed
                && !is_index_maintenance_confirmed
                && username.is_none()
                && !should_read_password
                && secret_key_file.is_none()
                && old_key_file.is_none()
                && new_key_file.is_none() =>
        {
            let backup_dir = resolve_project_path(
                &project_root,
                backup_dir.as_ref().expect("checked backup path").clone(),
            );
            let report = restore_backup(&BackupRestoreOptions {
                storage_config: StorageConfig::from_project_root(&project_root),
                auth_db_path,
                backup_dir: backup_dir.clone(),
            })?;
            Ok(json!({
                "status": "restored",
                "backup": backup_dir,
                "report": report,
            }))
        }
        [group, command]
            if group == "index"
                && command == "optimize-storage"
                && !has_explicit_auth_db
                && username.is_none()
                && !should_read_password
                && secret_key_file.is_none()
                && old_key_file.is_none()
                && new_key_file.is_none()
                && output_dir.is_none()
                && backup_dir.is_none()
                && !include_index_databases
                && !include_push_state
                && !is_restore_confirmed =>
        {
            let report = optimize_index_storage(&IndexStorageOptimizationOptions {
                storage_config: StorageConfig::from_project_root(&project_root),
                confirmed: is_index_maintenance_confirmed,
            })?;
            let status = match report.outcome {
                IndexStorageOptimizationOutcome::Noop => "noop",
                IndexStorageOptimizationOutcome::Optimized => "optimized",
            };
            Ok(json!({
                "status": status,
                "report": report,
            }))
        }
        _ => Err(admin_usage().into()),
    }
}

/// Run the unified application's `index` command.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
/// * `application_executable` - Canonical application executable used for child processes.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_index_command(
    args: Vec<String>,
    application_executable: impl AsRef<Path>,
) -> Result<(), Box<dyn Error>> {
    let application_executable = application_executable.as_ref().to_path_buf();
    run_cli_command("index", move || {
        run_index_command_with_bundled_meta_dir(
            args,
            &application_executable,
            litradar_storage::discover_packaged_meta_dir()?,
        )
    })
}

fn run_index_command_with_bundled_meta_dir(
    args: Vec<String>,
    application_executable: &Path,
    bundled_meta_dir: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let mut args = args;
    if has_help(&args) {
        print_help(&index_usage());
        return Ok(());
    }
    if let Some(request_path) = extract_path_option(&mut args, "--live-worker-request")? {
        if !args.is_empty() {
            return Err(format!("unexpected index worker arguments: {}", args.join(" ")).into());
        }
        run_live_index_worker_from_file_path(request_path)?;
        return Ok(());
    }
    let project_root = extract_project_root(&mut args)?;
    let auth_db_path = extract_auth_db_path_with_project_root(&mut args, &project_root)?;
    let secret_key_file = extract_path_option(&mut args, "--secret-key-file")?;
    let options = parse_index_options(&mut args)?;
    if !args.is_empty() {
        return Err(format!("unexpected index arguments: {}", args.join(" ")).into());
    }
    emit_legacy_issue_batch_warning(&options);
    let secret_key_file = required_secret_key_file(secret_key_file)?;
    migrate_auth_database(&auth_db_path)?;
    preflight_index_command_databases(&project_root, options.file.as_deref())?;
    let storage_config =
        StorageConfig::from_project_root(&project_root).with_auth_db_path(auth_db_path.clone());
    prepare_index_managed_meta(&storage_config, bundled_meta_dir.as_deref())?;
    let secret_codec = SecretCodec::load(&secret_key_file)?;
    verify_database_secrets(&auth_db_path, &secret_codec)?;
    let (scholarly_config, index_provider_routes, cnki_captcha_token, provider_proxy_selection) =
        live_index_runtime_config(&auth_db_path, &secret_codec, options.timeout_seconds)?;
    let has_scholarly_route = index_provider_routes
        .values()
        .any(|provider| provider == "scholarly");
    let concurrency = litradar_domain::validate_index_concurrency(
        options.worker_count,
        options.process_count,
        has_scholarly_route,
    )?;
    let outcome = run_live_index(&LiveIndexConfig {
        application_executable: application_executable.to_path_buf(),
        project_root: project_root.clone(),
        secret_key_file: secret_key_file.clone(),
        file: options.file.clone(),
        worker_count: options.worker_count,
        process_count: options.process_count,
        issue_batch_size: options.issue_batch_size,
        timeout_seconds: options.timeout_seconds,
        resume: options.resume,
        update: options.update,
        full_rescan: options.full_rescan,
        notify: options.notify,
        notify_dry_run: options.notify_dry_run,
        acknowledge_unknown_notify: options.acknowledge_unknown_notify,
        scholarly_config,
        cnki_captcha_token,
        provider_proxy_selection,
        index_provider_routes: index_provider_routes.clone(),
    })?;
    let effective_concurrency =
        index_concurrency_payload(&options, concurrency, &index_provider_routes, &outcome);
    print_result(&serialize_index_outcome(&outcome, effective_concurrency)?);
    Ok(())
}

fn prepare_index_managed_meta(
    storage_config: &StorageConfig,
    bundled_meta_dir: Option<&Path>,
) -> Result<Option<ManagedMetaPreparationReport>, Box<dyn Error>> {
    let Some(bundled_meta_dir) = bundled_meta_dir else {
        return Ok(None);
    };
    let report = litradar_storage::prepare_managed_meta(storage_config, bundled_meta_dir)?;
    let created = report
        .catalogs
        .iter()
        .filter(|catalog| catalog.action == ManagedMetaAction::Created)
        .count();
    let adopted = report
        .catalogs
        .iter()
        .filter(|catalog| catalog.action == ManagedMetaAction::Adopted)
        .count();
    let updated = report
        .catalogs
        .iter()
        .filter(|catalog| catalog.action == ManagedMetaAction::Updated)
        .count();
    let customized = report
        .catalogs
        .iter()
        .filter(|catalog| catalog.action == ManagedMetaAction::Customized)
        .count();
    let unchanged = report
        .catalogs
        .iter()
        .filter(|catalog| catalog.action == ManagedMetaAction::Unchanged)
        .count();
    tracing::info!(
        event = "storage.managed_meta.prepared",
        component = "storage",
        context = "index_startup",
        bundle_version = report.bundle_version,
        catalog_count = report.catalogs.len(),
        created,
        adopted,
        updated,
        customized,
        unchanged,
    );
    Ok(Some(report))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexOptions {
    file: Option<String>,
    worker_count: usize,
    process_count: usize,
    issue_batch_size: usize,
    is_issue_batch_explicit: bool,
    timeout_seconds: u64,
    resume: bool,
    update: bool,
    full_rescan: bool,
    notify: bool,
    notify_dry_run: bool,
    acknowledge_unknown_notify: bool,
}

fn parse_index_options(args: &mut Vec<String>) -> Result<IndexOptions, Box<dyn Error>> {
    let file = extract_string_option_any(args, &["--file", "-f"])?;
    let worker_count = positive_usize(
        "--workers",
        extract_usize_option_any(args, &["--workers", "-w"])?,
    )?
    .unwrap_or(DEFAULT_INDEX_WORKER_COUNT);
    let issue_batch_size = extract_usize_option(args, "--issue-batch")?;
    let is_issue_batch_explicit = issue_batch_size.is_some();
    let issue_batch_size = positive_usize("--issue-batch", issue_batch_size)?
        .unwrap_or(DEFAULT_INDEX_ISSUE_BATCH_SIZE);
    let timeout_seconds = extract_u64_option(args, "--timeout")?.unwrap_or(20);
    let process_count = positive_usize("--processes", extract_usize_option(args, "--processes")?)?
        .unwrap_or(DEFAULT_INDEX_PROCESS_COUNT);
    litradar_domain::validate_index_concurrency(worker_count, process_count, false)?;
    let resume = extract_bool_pair(args, "--resume", "--no-resume", true);
    let update = extract_bool_pair(args, "--update", "--no-update", false);
    let full_rescan = extract_bool_pair(args, "--full-rescan", "--no-full-rescan", false);
    let notify = extract_bool_pair(args, "--notify", "--no-notify", false);
    let notify_dry_run = extract_bool_pair(args, "--notify-dry-run", "--no-notify-dry-run", false);
    let acknowledge_unknown_notify = remove_flag(args, "--acknowledge-unknown-notify");
    if update && full_rescan {
        return Err("--update cannot be combined with --full-rescan".into());
    }
    if notify && !update {
        return Err("--notify requires --update".into());
    }
    if acknowledge_unknown_notify && !notify {
        return Err("--acknowledge-unknown-notify requires --notify".into());
    }
    if acknowledge_unknown_notify && !resume {
        return Err("--acknowledge-unknown-notify requires --resume".into());
    }
    Ok(IndexOptions {
        file,
        worker_count,
        process_count,
        issue_batch_size,
        is_issue_batch_explicit,
        timeout_seconds,
        resume,
        update,
        full_rescan,
        notify,
        notify_dry_run,
        acknowledge_unknown_notify,
    })
}

fn emit_legacy_issue_batch_warning(options: &IndexOptions) {
    if options.is_issue_batch_explicit {
        tracing::warn!(
            event = "cli.index.legacy_issue_batch",
            component = "cli",
            option = "issue_batch",
            behavior = "resume_compatibility_only",
            "explicit --issue-batch is legacy resume metadata and does not control current Provider concurrency or memory"
        );
    }
}

fn index_concurrency_payload(
    options: &IndexOptions,
    concurrency: litradar_domain::IndexConcurrency,
    index_provider_routes: &BTreeMap<String, String>,
    outcome: &LiveIndexOutcome,
) -> serde_json::Value {
    let (effective_worker_count, effective_process_count, effective_aggregate_capacity) = outcome
        .csvs
        .iter()
        .map(|csv| {
            let catalog = Path::new(&csv.csv_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let worker_count = match index_provider_routes.get(catalog).map(String::as_str) {
                Some("scholarly" | "cnki") => options.worker_count,
                _ => usize::from(csv.journal_count > 0),
            };
            let process_count = options.process_count.min(csv.journal_count);
            (
                worker_count,
                process_count,
                worker_count.saturating_mul(process_count),
            )
        })
        .max_by_key(|(_, _, aggregate_capacity)| *aggregate_capacity)
        .unwrap_or((0, 0, 0));
    json!({
        "workers": options.worker_count,
        "processes": options.process_count,
        "issue_batch": options.issue_batch_size,
        "configured_workers": concurrency.worker_count,
        "configured_processes": concurrency.process_count,
        "configured_aggregate_capacity": concurrency.aggregate_capacity,
        "effective_workers": effective_worker_count,
        "effective_processes": effective_process_count,
        "effective_aggregate_capacity": effective_aggregate_capacity,
        "aggregate_limit": litradar_domain::INDEX_AGGREGATE_CONCURRENCY_MAX,
    })
}

fn serialize_index_outcome(
    outcome: &LiveIndexOutcome,
    effective_concurrency: serde_json::Value,
) -> Result<String, serde_json::Error> {
    let mut payload = serde_json::to_value(outcome)?;
    payload
        .as_object_mut()
        .expect("live index outcomes should serialize as objects")
        .insert("effective_concurrency".to_string(), effective_concurrency);
    serde_json::to_string(&payload)
}

/// Run the unified application's `notify` command.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_notify_command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_delivery_command(DeliveryWorkflow::Notify, args)
}

/// Run the unified application's `push` command.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_push_command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_delivery_command(DeliveryWorkflow::Push, args)
}

/// Run the unified application's `scheduler` command.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
/// * `application_executable` - Canonical application executable used for task subprocesses.
///
/// # Returns
///
/// Result indicating whether the command completed successfully.
pub fn run_scheduler_command(
    args: Vec<String>,
    application_executable: impl AsRef<Path>,
) -> Result<(), Box<dyn Error>> {
    let application_executable = application_executable.as_ref().to_path_buf();
    run_cli_command("scheduler", move || {
        run_scheduler_command_inner(args, &application_executable)
    })
}

/// Run the private durable manual-delivery child command.
///
/// The application composition root only dispatches this entrypoint when a validated internal
/// parent-run marker is present.
///
/// # Arguments
///
/// * `args` - Private child arguments without the executable or subcommand name.
///
/// # Returns
///
/// Result indicating whether the durable job reached a persisted terminal state.
pub fn run_delivery_run_command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_cli_command("delivery-run", move || run_delivery_run_command_inner(args))
}

fn run_delivery_run_command_inner(mut args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let project_root = extract_project_root(&mut args)?;
    let auth_db_path = extract_auth_db_path_with_project_root(&mut args, &project_root)?;
    let secret_key_file =
        required_secret_key_file(extract_path_option(&mut args, "--secret-key-file")?)?;
    let run_id = extract_string_option(&mut args, "--run-id")?
        .ok_or("--run-id is required")?
        .parse::<i64>()?;
    let owner_id =
        extract_string_option(&mut args, "--owner-id")?.ok_or("--owner-id is required")?;
    if run_id <= 0 {
        return Err("--run-id must be a positive integer".into());
    }
    if !args.is_empty() {
        return Err("unexpected delivery-run arguments".into());
    }

    migrate_auth_database(&auth_db_path)?;
    let secret_codec = SecretCodec::load(&secret_key_file)?;
    verify_database_secrets(&auth_db_path, &secret_codec)?;
    let storage_config =
        StorageConfig::from_project_root(project_root).with_auth_db_path(auth_db_path);
    let terminal = run_manual_delivery_job(&storage_config, secret_codec, run_id, &owner_id)?;
    if !terminal.status.is_terminal() {
        return Err("durable manual delivery job did not reach a terminal state".into());
    }
    print_result(&serde_json::to_string(&json!({
        "run_id": terminal.id,
        "status": terminal.status.as_str(),
    }))?);
    Ok(())
}

fn run_scheduler_command_inner(
    mut args: Vec<String>,
    application_executable: &Path,
) -> Result<(), Box<dyn Error>> {
    if has_help(&args) {
        print_help(&scheduler_usage());
        return Ok(());
    }
    let project_root = extract_project_root(&mut args)?;
    let auth_db_path = extract_auth_db_path_with_project_root(&mut args, &project_root)?;
    let secret_key_file = extract_path_option(&mut args, "--secret-key-file")?;
    let action = match args.as_slice() {
        [command] if command == "validate" => SchedulerAction::Validate,
        [command, task_id] if command == "run-once" => {
            let task_id = task_id.parse::<i64>()?;
            SchedulerAction::RunOnce(task_id, SchedulerMode::Execute)
        }
        [command, task_id] if command == "dry-run-once" => {
            let task_id = task_id.parse::<i64>()?;
            SchedulerAction::RunOnce(task_id, SchedulerMode::DryRun)
        }
        _ => return Err(scheduler_usage().into()),
    };
    let secret_key_file = required_secret_key_file(secret_key_file)?;
    preflight_command_storage(&project_root, &auth_db_path)?;
    let secret_codec = SecretCodec::load(&secret_key_file)?;
    verify_database_secrets(&auth_db_path, &secret_codec)?;
    match action {
        SchedulerAction::Validate => print_scheduler_load(&auth_db_path),
        SchedulerAction::RunOnce(task_id, mode) => {
            let outcome = run_task_now(
                &auth_db_path,
                application_executable,
                &secret_key_file,
                task_id,
                mode,
            )?;
            print_result(&serde_json::to_string(&outcome)?);
            Ok(())
        }
    }
}

enum SchedulerAction {
    Validate,
    RunOnce(i64, SchedulerMode),
}

fn run_delivery_command(
    workflow: DeliveryWorkflow,
    args: Vec<String>,
) -> Result<(), Box<dyn Error>> {
    let command = match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    };
    run_cli_command(command, move || run_delivery_command_inner(workflow, args))
}

fn run_delivery_command_inner(
    workflow: DeliveryWorkflow,
    mut args: Vec<String>,
) -> Result<(), Box<dyn Error>> {
    if has_help(&args) {
        print_help(&delivery_usage(workflow));
        return Ok(());
    }
    let command_name = match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    };
    let mut mode = DeliveryMode::Execute;
    if remove_flag(&mut args, "--dry-run") {
        mode = DeliveryMode::DryRun;
    }
    if remove_flag(&mut args, "--no-dry-run") {
        mode = DeliveryMode::Execute;
    }
    let is_internal_handoff = remove_flag(&mut args, "--internal-handoff-json");
    let project_root = extract_project_root(&mut args)?;
    let auth_db_path = extract_auth_db_path_with_project_root(&mut args, &project_root)?;
    let secret_key_file = extract_path_option(&mut args, "--secret-key-file")?;
    let index_db_path = extract_path_option(&mut args, "--index-db")?;
    let db_name = extract_string_option(&mut args, "--db")?;
    let changes_file = extract_path_option(&mut args, "--changes-file")?;
    let attempt_id = extract_string_option(&mut args, "--attempt-id")?;
    match (is_internal_handoff, attempt_id.as_deref()) {
        (true, Some(attempt_id)) if is_valid_delivery_attempt_id(attempt_id) => {}
        (true, Some(_)) => {
            return Err("--attempt-id must be exactly 32 hexadecimal characters".into())
        }
        (true, None) => return Err("--internal-handoff-json requires --attempt-id".into()),
        (false, Some(_)) => return Err("--attempt-id requires --internal-handoff-json".into()),
        (false, None) => {}
    }
    let ai_model = extract_string_option(&mut args, "--ai-model")?;
    let max_candidates = extract_usize_option(&mut args, "--max-candidates")?;
    let timeout_seconds = extract_u64_option(&mut args, "--timeout")?.unwrap_or(60);
    let retry_attempts = extract_usize_option(&mut args, "--retries")?.unwrap_or(3);
    if !(DELIVERY_RETRY_ATTEMPTS_MIN..=DELIVERY_RETRY_ATTEMPTS_MAX).contains(&retry_attempts) {
        return Err(format!(
            "--retries must be between {DELIVERY_RETRY_ATTEMPTS_MIN} and {DELIVERY_RETRY_ATTEMPTS_MAX}"
        )
        .into());
    }
    let dedupe_retention_days =
        extract_i64_option(&mut args, "--dedupe-retention-days")?.unwrap_or(60);
    if !args.is_empty() {
        return Err(format!("unexpected {command_name} arguments: {}", args.join(" ")).into());
    }
    let secret_key_file = required_secret_key_file(secret_key_file)?;
    let changes_file = changes_file
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| resolve_project_path(&project_root, path));
    let targets = resolve_delivery_targets(
        &project_root,
        index_db_path,
        db_name,
        changes_file.as_deref(),
    )?;
    if is_internal_handoff && targets.len() != 1 {
        return Err("internal delivery handoff requires exactly one database".into());
    }
    preflight_command_storage(&project_root, &auth_db_path)?;
    let secret_codec = SecretCodec::load(&secret_key_file)?;
    verify_database_secrets(&auth_db_path, &secret_codec)?;
    let storage_config = StorageConfig::from_project_root(&project_root);
    let tokenizer_path = storage_config.simple_tokenizer_path();
    for target in &targets {
        preflight_index_database(&target.index_db_path, tokenizer_path.as_deref())?;
    }
    let mut outcomes = Vec::new();
    for target in targets {
        let outcome = run_recommendation_delivery(&RecommendationRunConfig {
            auth_db_path: auth_db_path.clone(),
            secret_codec: secret_codec.clone(),
            index_db_path: target.index_db_path,
            db_name: target.db_name,
            changes_file: changes_file.clone(),
            attempt_id: attempt_id.clone(),
            ai_model: ai_model.clone(),
            max_candidates,
            timeout_seconds,
            retry_attempts,
            dedupe_retention_days,
            mode,
            workflow,
            trigger: litradar_worker::delivery::DeliveryTrigger::Scheduled,
            execution_control: None,
        })?;
        outcomes.push(outcome);
    }
    let status = aggregate_delivery_status(&outcomes);
    let payload = if is_internal_handoff {
        json!({
            "protocol_version": 1,
            "attempt_id": attempt_id.expect("validated internal handoff should have an attempt"),
            "workflow": workflow,
            "mode": mode,
            "status": status,
            "db_name": outcomes
                .first()
                .expect("validated internal handoff should have one outcome")
                .db_name,
        })
    } else {
        json!({
            "workflow": workflow,
            "mode": mode,
            "status": status,
            "databases": outcomes,
        })
    };
    print_result(&serde_json::to_string(&payload)?);
    ensure_delivery_command_success(command_name, status)?;
    Ok(())
}

fn is_valid_delivery_attempt_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn aggregate_delivery_status(
    outcomes: &[litradar_worker::delivery::RecommendationRunOutcome],
) -> DeliveryOutcomeState {
    outcomes
        .iter()
        .map(|outcome| outcome.status)
        .max_by_key(|status| delivery_status_priority(*status))
        .unwrap_or(DeliveryOutcomeState::Idle)
}

fn delivery_status_priority(status: DeliveryOutcomeState) -> u8 {
    match status {
        DeliveryOutcomeState::Idle => 0,
        DeliveryOutcomeState::Skipped => 1,
        DeliveryOutcomeState::Completed => 2,
        DeliveryOutcomeState::Running => 3,
        DeliveryOutcomeState::Cancelled => 4,
        DeliveryOutcomeState::TimedOut => 5,
        DeliveryOutcomeState::Failed => 6,
        DeliveryOutcomeState::Unknown => 7,
    }
}

fn ensure_delivery_command_success(
    command: &'static str,
    status: DeliveryOutcomeState,
) -> Result<(), DeliveryCommandStatusError> {
    match status {
        DeliveryOutcomeState::Idle
        | DeliveryOutcomeState::Completed
        | DeliveryOutcomeState::Skipped => Ok(()),
        DeliveryOutcomeState::Running
        | DeliveryOutcomeState::Failed
        | DeliveryOutcomeState::Cancelled
        | DeliveryOutcomeState::TimedOut
        | DeliveryOutcomeState::Unknown => Err(DeliveryCommandStatusError { command, status }),
    }
}

fn print_scheduler_load(auth_db_path: &Path) -> Result<(), Box<dyn Error>> {
    let result = load_scheduler_jobs(auth_db_path)?;
    print_result(&serde_json::to_string(&result)?);
    Ok(())
}

fn preflight_command_storage(
    project_root: &Path,
    auth_db_path: &Path,
) -> Result<(), Box<dyn Error>> {
    preflight_storage(
        &StorageConfig::from_project_root(project_root).with_auth_db_path(auth_db_path),
    )?;
    Ok(())
}

fn preflight_index_command_databases(
    project_root: &Path,
    selected_file: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let storage_config = StorageConfig::from_project_root(project_root);
    let Some(selected_file) = selected_file else {
        preflight_existing_index_databases(&storage_config)?;
        return Ok(());
    };
    let file_path = Path::new(selected_file);
    if file_path.file_name().and_then(|value| value.to_str()) != Some(selected_file)
        || file_path.extension().and_then(|value| value.to_str()) != Some("csv")
    {
        return Err("--file must be one CSV filename without directory components".into());
    }
    let catalog_name = file_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("--file must have a UTF-8 stem")?;
    let index_path = storage_config
        .index_dir()
        .join(format!("{catalog_name}.sqlite"));
    if index_path.exists() {
        let tokenizer_path = storage_config.simple_tokenizer_path();
        preflight_index_database(&index_path, tokenizer_path.as_deref())?;
    }
    Ok(())
}

#[cfg(test)]
fn extract_auth_db_path(args: &mut Vec<String>) -> Result<PathBuf, Box<dyn Error>> {
    let project_root = std::env::current_dir().expect("current directory should be available");
    extract_auth_db_path_with_project_root(args, &project_root)
}

fn extract_auth_db_path_with_project_root(
    args: &mut Vec<String>,
    project_root: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(index) = args.iter().position(|argument| argument == "--auth-db") {
        if index + 1 >= args.len() {
            return Err("--auth-db requires a path".into());
        }
        let path = PathBuf::from(args.remove(index + 1));
        args.remove(index);
        return Ok(path);
    }
    Ok(project_root.join("data").join("auth.sqlite"))
}

fn extract_project_root(args: &mut Vec<String>) -> Result<PathBuf, Box<dyn Error>> {
    Ok(extract_path_option(args, "--project-root")?
        .unwrap_or_else(|| std::env::current_dir().expect("current directory should be available")))
}

fn extract_path_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<PathBuf>, Box<dyn Error>> {
    Ok(extract_string_option(args, name)?.map(PathBuf::from))
}

fn required_secret_key_file(path: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    path.ok_or_else(|| "--secret-key-file is required".into())
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

type LiveIndexRuntimeConfig = (
    LiveScholarlyConfig,
    BTreeMap<String, String>,
    Option<String>,
    ProviderProxySelection,
);

fn live_index_runtime_config(
    auth_db_path: &Path,
    secret_codec: &SecretCodec,
    timeout_seconds: u64,
) -> Result<LiveIndexRuntimeConfig, Box<dyn Error>> {
    let settings = litradar_storage::load_runtime_settings(auth_db_path, secret_codec)?;
    let setting_value = |field: &str| {
        settings
            .iter()
            .find(|setting| setting.field == field)
            .map(|setting| setting.value.as_str())
            .unwrap_or_default()
    };
    let scholarly_config = LiveScholarlyConfig::from_value_pools(
        timeout_seconds,
        setting_value("openalex_api_key_pool"),
        setting_value("semantic_scholar_api_key_pool"),
        setting_value("crossref_mailto_pool"),
    );
    let index_provider_routes =
        serde_json::from_str::<BTreeMap<String, String>>(setting_value("index_provider_routes"))?;
    let cnki_captcha_token = {
        let from_settings = setting_value("cnki_captcha_token").trim();
        if !from_settings.is_empty() {
            Some(from_settings.to_string())
        } else {
            std::env::var("LITRADAR_CNKI_CAPTCHA_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty())
        }
    };
    let provider_proxy_selection = ProviderProxySelection::from_runtime_settings(&settings)?;
    Ok((
        scholarly_config,
        index_provider_routes,
        cnki_captcha_token,
        provider_proxy_selection,
    ))
}

fn index_usage() -> String {
    let payload = json!({
        "usage": "litradar index --secret-key-file PATH [--project-root PATH] [--auth-db PATH] [--file FILE] [--workers N] [--processes N] [--issue-batch N] [--timeout N] [--resume|--no-resume] [--update|--no-update] [--full-rescan|--no-full-rescan] [--notify] [--notify-dry-run] [--acknowledge-unknown-notify]",
        "defaults": {
            "workers": DEFAULT_INDEX_WORKER_COUNT,
            "processes": DEFAULT_INDEX_PROCESS_COUNT,
            "issue_batch": DEFAULT_INDEX_ISSUE_BATCH_SIZE,
            "resume": true,
            "update": false,
            "full_rescan": false,
        },
        "modes": {
            "update": "incremental synchronization that publishes a change manifest",
            "full_rescan": "complete historical synchronization without a change manifest; mutually exclusive with --update",
            "resume": "continue only a compatible active project batch; completed batches always start a new independent update",
            "no_resume": "abandon the active batch and its owned traversal checkpoints, then start a new batch from committed anchors",
            "file": "select and freeze exactly one CSV; it cannot adopt an active all-CSV batch",
            "issue_batch": "legacy active-batch resume metadata; explicit use warns and does not control current Provider concurrency or memory",
            "acknowledge_unknown_notify": "after review, acknowledge an ambiguous notify attempt and resume with a new delivery attempt",
        },
        "limits": {
            "workers_min": litradar_domain::INDEX_WORKER_COUNT_MIN,
            "workers_max": litradar_domain::INDEX_WORKER_COUNT_MAX,
            "processes_min": litradar_domain::INDEX_PROCESS_COUNT_MIN,
            "processes_max": litradar_domain::INDEX_PROCESS_COUNT_MAX,
            "aggregate_max": litradar_domain::INDEX_AGGREGATE_CONCURRENCY_MAX,
            "scholarly_workers_max": litradar_domain::SCHOLARLY_WORKER_COUNT_MAX,
            "scholarly_processes_max": litradar_domain::SCHOLARLY_PROCESS_COUNT_MAX,
        }
    });
    payload.to_string()
}

fn scheduler_usage() -> String {
    let payload = json!({
        "usage": [
            "litradar scheduler validate --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
            "litradar scheduler run-once TASK_ID --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
            "litradar scheduler dry-run-once TASK_ID --secret-key-file PATH [--project-root PATH] [--auth-db PATH]"
        ]
    });
    payload.to_string()
}

fn admin_usage() -> String {
    json!({
        "usage": [
            "litradar admin bootstrap --username NAME --password-stdin [--project-root PATH] [--auth-db PATH]",
            "litradar admin secrets migrate --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
            "litradar admin secrets verify --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
            "litradar admin secrets rotate --old-key-file PATH --new-key-file PATH [--project-root PATH] [--auth-db PATH]",
            "litradar admin backup create --output PATH [--include-indexes] [--include-push-state] [--project-root PATH] [--auth-db PATH]",
            "litradar admin backup verify --backup PATH [--project-root PATH]",
            "litradar admin backup restore --backup PATH --confirm-restore [--project-root PATH] [--auth-db PATH]",
            "litradar admin index optimize-storage --confirm-index-maintenance [--project-root PATH]"
        ]
    })
    .to_string()
}

fn delivery_usage(workflow: DeliveryWorkflow) -> String {
    let command_name = match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    };
    let payload = json!({
        "usage": format!("litradar {command_name} --secret-key-file PATH [--project-root PATH] [--auth-db PATH] [--db NAME] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--timeout N] [--retries N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]")
    });
    payload.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use tempfile::{Builder, TempDir};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    use super::{
        admin_usage, aggregate_delivery_status, delivery_usage, ensure_delivery_command_success,
        extract_auth_db_path, extract_bool_pair, extract_string_option, extract_usize_option,
        index_concurrency_payload, index_optimization_failure_payload, index_usage,
        live_index_runtime_config, normalize_db_name, parse_index_options,
        preflight_command_storage, preflight_index_command_databases, prepare_index_managed_meta,
        resolve_delivery_targets, resolve_project_path, run_admin_command_with_reader,
        run_index_command, run_index_command_with_bundled_meta_dir, run_notify_command,
        run_push_command, run_scheduler_command, scheduler_usage, serialize_index_outcome,
        IndexOptions,
    };
    use litradar_index::{LiveCsvIndexOutcome, LiveIndexOutcome};
    use litradar_worker::delivery::{
        DeliveryMode, DeliveryOutcomeState, DeliveryWorkflow, RecommendationRunOutcome,
    };

    #[derive(Clone, Default)]
    struct CapturedEvents {
        events: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
    }

    impl CapturedEvents {
        fn snapshot(&self) -> Vec<BTreeMap<String, String>> {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[derive(Default)]
    struct CapturedEventFields {
        fields: BTreeMap<String, String>,
    }

    impl Visit for CapturedEventFields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    impl Subscriber for CapturedEvents {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut captured = CapturedEventFields::default();
            event.record(&mut captured);
            captured
                .fields
                .insert("level".to_string(), event.metadata().level().to_string());
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(captured.fields);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[test]
    fn unified_usage_lists_only_supported_commands() {
        let admin = admin_usage();
        let index = index_usage();
        let notify = delivery_usage(DeliveryWorkflow::Notify);
        let push = delivery_usage(DeliveryWorkflow::Push);
        let scheduler = scheduler_usage();

        assert!(index.contains("--file FILE"));
        assert!(index.contains("--workers N"));
        assert!(index.contains("--processes N"));
        assert!(index.contains("--full-rescan|--no-full-rescan"));
        assert!(index.contains("--notify-dry-run"));
        assert!(index.contains("--acknowledge-unknown-notify"));
        let index_payload: serde_json::Value =
            serde_json::from_str(&index).expect("index usage should be JSON");
        assert_eq!(index_payload["defaults"]["workers"], 6);
        assert_eq!(index_payload["defaults"]["processes"], 1);
        assert_eq!(index_payload["defaults"]["issue_batch"], 8);
        assert_eq!(index_payload["defaults"]["resume"], true);
        assert_eq!(index_payload["defaults"]["update"], false);
        assert_eq!(index_payload["defaults"]["full_rescan"], false);
        assert!(index_payload["modes"]["full_rescan"]
            .as_str()
            .is_some_and(|value| value.contains("mutually exclusive with --update")));
        assert!(index_payload["modes"]["resume"]
            .as_str()
            .is_some_and(|value| value.contains("compatible active project batch")));
        assert!(index_payload["modes"]["no_resume"]
            .as_str()
            .is_some_and(|value| value.contains("start a new batch from committed anchors")));
        assert!(index_payload["modes"]["file"]
            .as_str()
            .is_some_and(|value| value.contains("exactly one CSV")));
        assert!(index_payload["modes"]["issue_batch"]
            .as_str()
            .is_some_and(|value| value.contains("does not control current Provider concurrency")));
        assert!(index_payload["modes"]["acknowledge_unknown_notify"]
            .as_str()
            .is_some_and(|value| value.contains("ambiguous notify attempt")));
        assert_eq!(index_payload["limits"]["workers_max"], 32);
        assert_eq!(index_payload["limits"]["aggregate_max"], 32);
        assert_eq!(index_payload["limits"]["scholarly_workers_max"], 6);
        assert!(notify.contains("notify --secret-key-file PATH"));
        assert!(push.contains("push --secret-key-file PATH"));
        assert!(scheduler.contains("scheduler validate"));
        assert!(scheduler.contains("scheduler run-once TASK_ID"));
        assert!(scheduler.contains("scheduler dry-run-once TASK_ID"));
        assert!(admin.contains("admin bootstrap --username NAME --password-stdin"));
        assert!(admin.contains("admin backup create --output PATH"));
        assert!(admin.contains("admin backup restore --backup PATH --confirm-restore"));
        assert!(admin.contains("admin index optimize-storage --confirm-index-maintenance"));
        assert!(!admin.contains("--password "));
        for usage in [admin, index, notify, push, scheduler] {
            assert!(usage.contains("litradar "));
            assert!(!usage.contains("litradar-cli"));
            assert!(!usage.contains("shadow"));
            assert!(!usage.contains("index fixture"));
            assert!(!usage.contains("worker execute"));
            assert!(!usage.contains("LITRADAR_LIVE_INDEX_WORKER_REQUEST"));
        }
    }

    #[test]
    fn admin_bootstrap_reads_password_from_stdin_and_refuses_repeat() {
        let root = temp_root("litradar-cli-admin-bootstrap");
        let auth_db_path = root.path().join("auth.sqlite");
        let args = vec![
            "bootstrap".to_string(),
            "--username".to_string(),
            "fixture_admin".to_string(),
            "--password-stdin".to_string(),
            "--auth-db".to_string(),
            auth_db_path.to_string_lossy().into_owned(),
        ];

        let payload =
            run_admin_command_with_reader(args.clone(), std::io::Cursor::new("fixture-password\n"))
                .expect("first bootstrap should succeed");
        let repeat_error =
            run_admin_command_with_reader(args, std::io::Cursor::new("different-password\n"))
                .expect_err("repeat bootstrap should fail");
        let user =
            litradar_storage::find_user_credentials_by_username(&auth_db_path, "fixture_admin")
                .expect("bootstrapped user should load")
                .expect("bootstrapped user should exist");

        assert_eq!(payload["status"], "created");
        assert_eq!(payload["user"]["username"], "fixture_admin");
        assert_eq!(payload["user"]["is_admin"], true);
        assert!(!payload.to_string().contains("fixture-password"));
        assert!(user.is_admin);
        assert_eq!(
            repeat_error.to_string(),
            "Administrator bootstrap is already complete"
        );
        assert_eq!(
            litradar_storage::count_users(&auth_db_path).expect("user count should load"),
            1
        );
    }

    #[test]
    fn admin_bootstrap_rejects_command_line_passwords_without_reading_them() {
        let root = temp_root("litradar-cli-admin-password-argument");
        let auth_db_path = root.path().join("auth.sqlite");

        let error = run_admin_command_with_reader(
            vec![
                "bootstrap".to_string(),
                "--username".to_string(),
                "fixture_admin".to_string(),
                "--password".to_string(),
                "forbidden-password".to_string(),
                "--auth-db".to_string(),
                auth_db_path.to_string_lossy().into_owned(),
            ],
            std::io::Cursor::new(Vec::<u8>::new()),
        )
        .expect_err("command-line password should be rejected");

        assert!(error.to_string().contains("--password-stdin"));
        assert!(!auth_db_path.exists());
    }

    #[test]
    fn admin_secret_migration_and_verification_are_explicit() {
        let root = temp_root("litradar-cli-secret-migration");
        let auth_db_path = root.path().join("auth.sqlite");
        let secret_key_file = root.path().join("secret.key");
        fs::write(&secret_key_file, [15_u8; 32]).expect("secret key should write");
        litradar_storage::migrate_auth_database(&auth_db_path)
            .expect("auth database should migrate");
        litradar_storage::open_sqlite_connection(&auth_db_path)
            .expect("auth database should open")
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, 1)",
                ("openalex_api_key_pool", "legacy-plaintext-key"),
            )
            .expect("legacy plaintext fixture should insert");

        let migrated = run_admin_command_with_reader(
            vec![
                "--auth-db".to_string(),
                auth_db_path.to_string_lossy().into_owned(),
                "--secret-key-file".to_string(),
                secret_key_file.to_string_lossy().into_owned(),
                "secrets".to_string(),
                "migrate".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("explicit migration should succeed");
        assert_eq!(migrated["status"], "migrated");
        assert_eq!(migrated["migrated"], 1);

        let verified = run_admin_command_with_reader(
            vec![
                "--auth-db".to_string(),
                auth_db_path.to_string_lossy().into_owned(),
                "--secret-key-file".to_string(),
                secret_key_file.to_string_lossy().into_owned(),
                "secrets".to_string(),
                "verify".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("explicit verification should succeed");
        assert_eq!(verified["status"], "verified");
        assert_eq!(verified["verified"], 1);
        let raw: String = litradar_storage::open_sqlite_connection(&auth_db_path)
            .expect("auth database should reopen")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted value should load");
        assert!(raw.starts_with("litradarenc:v1:"));
        assert!(!raw.contains("legacy-plaintext-key"));
    }

    #[test]
    fn admin_secret_rotation_reencrypts_existing_values() {
        let root = temp_root("litradar-cli-secret-rotation");
        let auth_db_path = root.path().join("auth.sqlite");
        let old_key_file = root.path().join("old.key");
        let new_key_file = root.path().join("new.key");
        fs::write(&old_key_file, [16_u8; 32]).expect("old key should write");
        fs::write(&new_key_file, [17_u8; 32]).expect("new key should write");
        litradar_storage::migrate_auth_database(&auth_db_path)
            .expect("auth database should migrate");
        let old_codec =
            litradar_storage::SecretCodec::load(&old_key_file).expect("old codec should load");
        litradar_storage::upsert_runtime_settings(
            &auth_db_path,
            &old_codec,
            &HashMap::from([(
                "openalex_api_key_pool".to_string(),
                Some("rotation-fixture-secret".to_string()),
            )]),
            &HashMap::new(),
        )
        .expect("encrypted runtime setting should write");

        let payload = run_admin_command_with_reader(
            vec![
                "--auth-db".to_string(),
                auth_db_path.to_string_lossy().into_owned(),
                "--old-key-file".to_string(),
                old_key_file.to_string_lossy().into_owned(),
                "--new-key-file".to_string(),
                new_key_file.to_string_lossy().into_owned(),
                "secrets".to_string(),
                "rotate".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("secret rotation should succeed");
        let new_codec =
            litradar_storage::SecretCodec::load(&new_key_file).expect("new codec should load");
        let settings = litradar_storage::load_runtime_settings(&auth_db_path, &new_codec)
            .expect("new codec should decrypt settings");

        assert_eq!(payload["status"], "rotated");
        assert_eq!(payload["rotated"], 1);
        assert!(!payload.to_string().contains("rotation-fixture-secret"));
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("rotated setting should exist")
                .value,
            "rotation-fixture-secret"
        );
        assert!(litradar_storage::load_runtime_settings(&auth_db_path, &old_codec).is_err());
    }

    #[test]
    fn live_index_runtime_config_loads_provider_proxy_and_rejects_inconsistent_state() {
        let root = temp_root("litradar-cli-provider-proxy");
        let auth_db_path = root.path().join("auth.sqlite");
        let secret_key_file = root.path().join("secret.key");
        let password_sentinel = "cli-provider-proxy-password-sentinel";
        fs::write(&secret_key_file, [19_u8; 32]).expect("secret key should write");
        litradar_storage::migrate_auth_database(&auth_db_path)
            .expect("auth database should migrate");
        let codec =
            litradar_storage::SecretCodec::load(&secret_key_file).expect("codec should load");
        litradar_storage::upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([
                (
                    "provider_proxy_url".to_string(),
                    Some(format!(
                        "socks5h://proxy-user:{password_sentinel}@127.0.0.1:1081"
                    )),
                ),
                (
                    "provider_proxy_policy".to_string(),
                    Some(r#"{"cnki":true,"scholarly":false}"#.to_string()),
                ),
            ]),
            &HashMap::new(),
        )
        .expect("Provider proxy settings should write");

        let (_, _, _, selection) = live_index_runtime_config(&auth_db_path, &codec, 30)
            .expect("valid Provider proxy state should load for each index command");
        assert!(selection.for_provider("cnki").is_explicit());
        assert!(!selection.for_provider("scholarly").is_explicit());
        assert!(!format!("{selection:?}").contains(password_sentinel));

        let connection = litradar_storage::open_sqlite_connection(&auth_db_path)
            .expect("auth database should open");
        connection
            .execute(
                "UPDATE runtime_settings SET value = ?1 WHERE key = 'provider_proxy_policy'",
                [r#"{"unknown-provider":true}"#],
            )
            .expect("unknown Provider policy fixture should write");
        let invalid_error = live_index_runtime_config(&auth_db_path, &codec, 30)
            .expect_err("unknown Provider policy should fail at command startup");
        assert!(invalid_error.to_string().contains("Unknown Provider"));
        assert!(!invalid_error.to_string().contains(password_sentinel));

        connection
            .execute(
                "UPDATE runtime_settings SET value = ?1 WHERE key = 'provider_proxy_policy'",
                [r#"{"cnki":true}"#],
            )
            .expect("enabled Provider policy fixture should write");
        connection
            .execute(
                "UPDATE runtime_settings SET value = '' WHERE key = 'provider_proxy_url'",
                [],
            )
            .expect("missing proxy URL fixture should write");
        let inconsistent_error = live_index_runtime_config(&auth_db_path, &codec, 30)
            .expect_err("enabled Provider without a URL should fail at command startup");
        assert!(inconsistent_error
            .to_string()
            .contains("Provider proxy URL is required"));
        assert!(!inconsistent_error.to_string().contains(password_sentinel));
    }

    #[test]
    fn live_index_runtime_config_keeps_captcha_environment_fallback() {
        let output = Command::new(
            std::env::current_exe().expect("current CLI test executable should resolve"),
        )
        .arg("--exact")
        .arg("tests::live_index_captcha_environment_helper")
        .arg("--ignored")
        .arg("--nocapture")
        .env(
            "LITRADAR_CNKI_CAPTCHA_TOKEN",
            "index-captcha-environment-sentinel",
        )
        .output()
        .expect("index captcha environment helper should run");

        assert!(
            output.status.success(),
            "index captcha environment helper failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "private helper process for index captcha environment isolation"]
    fn live_index_captcha_environment_helper() {
        let root = temp_root("litradar-cli-captcha-environment");
        let auth_db_path = root.path().join("auth.sqlite");
        litradar_storage::migrate_auth_database(&auth_db_path)
            .expect("auth database should migrate");
        let codec = litradar_storage::SecretCodec::from_key([83_u8; 32]);

        let (_, _, captcha_token, _) = live_index_runtime_config(&auth_db_path, &codec, 30)
            .expect("index runtime configuration should load");
        assert_eq!(
            captcha_token.as_deref(),
            Some("index-captcha-environment-sentinel")
        );
    }

    #[test]
    fn admin_backup_create_verify_and_confirmed_restore_are_explicit() {
        let root = temp_root("litradar-cli-backup");
        let source_root = root.path().join("source");
        let source_config = litradar_storage::StorageConfig::from_project_root(&source_root);
        litradar_storage::migrate_auth_database(source_config.auth_db_path())
            .expect("source auth database should migrate");
        litradar_storage::open_sqlite_connection(source_config.auth_db_path())
            .expect("source auth database should open")
            .execute_batch(
                "CREATE TABLE backup_cli_probe (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO backup_cli_probe (id, value) VALUES (1, 'cli-row');",
            )
            .expect("source probe should write");

        let created = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                source_root.to_string_lossy().into_owned(),
                "--output".to_string(),
                "backups/fixture".to_string(),
                "backup".to_string(),
                "create".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("backup create should succeed");
        let backup_dir = source_root.join("backups").join("fixture");
        assert_eq!(created["status"], "created");
        assert!(backup_dir.join("manifest.json").is_file());

        let verified = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                source_root.to_string_lossy().into_owned(),
                "--backup".to_string(),
                backup_dir.to_string_lossy().into_owned(),
                "backup".to_string(),
                "verify".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("backup verify should succeed");
        assert_eq!(verified["status"], "verified");

        let restore_root = root.path().join("restored");
        let missing_confirmation = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                restore_root.to_string_lossy().into_owned(),
                "--backup".to_string(),
                backup_dir.to_string_lossy().into_owned(),
                "backup".to_string(),
                "restore".to_string(),
            ],
            "".as_bytes(),
        )
        .expect_err("restore without confirmation should fail");
        assert!(missing_confirmation
            .to_string()
            .contains("--confirm-restore"));
        let restore_config = litradar_storage::StorageConfig::from_project_root(&restore_root);
        assert!(!restore_config.auth_db_path().exists());

        let restored = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                restore_root.to_string_lossy().into_owned(),
                "--backup".to_string(),
                backup_dir.to_string_lossy().into_owned(),
                "--confirm-restore".to_string(),
                "backup".to_string(),
                "restore".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("confirmed restore should succeed");
        assert_eq!(restored["status"], "restored");
        let value: String = litradar_storage::open_sqlite_connection(restore_config.auth_db_path())
            .expect("restored auth database should open")
            .query_row(
                "SELECT value FROM backup_cli_probe WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("restored probe should load");
        assert_eq!(value, "cli-row");
    }

    #[test]
    fn admin_index_storage_optimization_is_confirmed_and_returns_structured_reports() {
        let root = temp_root("litradar-cli-index-storage-optimization");
        let project_root = root.path().join("project");
        let config = litradar_storage::StorageConfig::from_project_root(&project_root);
        fs::create_dir_all(config.index_dir()).expect("index directory should create");
        litradar_storage::migrate_index_database(config.index_dir().join("fixture.sqlite"), None)
            .expect("fixture index should initialize");

        let missing_confirmation = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                project_root.to_string_lossy().into_owned(),
                "index".to_string(),
                "optimize-storage".to_string(),
            ],
            "".as_bytes(),
        )
        .expect_err("missing maintenance confirmation should fail");
        let typed_error = missing_confirmation
            .downcast_ref::<litradar_storage::IndexStorageOptimizationError>()
            .expect("confirmation failure should retain its structured error type");
        assert_eq!(typed_error.code(), "confirmation_required");
        assert_eq!(
            index_optimization_failure_payload(typed_error)["error"]["code"],
            "confirmation_required"
        );

        let optimized = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                project_root.to_string_lossy().into_owned(),
                "--confirm-index-maintenance".to_string(),
                "index".to_string(),
                "optimize-storage".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("confirmed optimization should succeed");
        assert_eq!(optimized["status"], "optimized");
        assert_eq!(optimized["report"]["database_count"], 1);
        assert_eq!(
            optimized["report"]["databases"][0]["target_schema_version"],
            litradar_storage::INDEX_SCHEMA_VERSION
        );

        let empty_root = root.path().join("empty");
        let noop = run_admin_command_with_reader(
            vec![
                "--project-root".to_string(),
                empty_root.to_string_lossy().into_owned(),
                "--confirm-index-maintenance".to_string(),
                "index".to_string(),
                "optimize-storage".to_string(),
            ],
            "".as_bytes(),
        )
        .expect("empty index target should be a no-op");
        assert_eq!(noop["status"], "noop");
        assert_eq!(noop["report"]["database_count"], 0);
        assert!(!empty_root.join("data").exists());
    }

    #[test]
    fn admin_index_storage_failures_have_stable_json_codes_and_recovery_paths() {
        let rollback_error = litradar_storage::IndexStorageOptimizationError::RollbackFailed {
            detail: "injected rollback failure".to_string(),
            recovery_paths: Box::new(litradar_storage::IndexStorageRecoveryPaths {
                marker: PathBuf::from("data/.litradar-index-maintenance.json"),
                staging: PathBuf::from("data/.litradar-index-staging"),
                rollback: PathBuf::from("data/.litradar-index-rollback"),
            }),
        };

        let payload = index_optimization_failure_payload(&rollback_error);

        assert_eq!(payload["status"], "failed");
        assert_eq!(payload["error"]["code"], "rollback_failed");
        assert!(payload["error"]["recovery_paths"]["marker"]
            .as_str()
            .is_some_and(|path| path.contains(".litradar-index-maintenance.json")));
        assert!(!payload.to_string().contains("article"));
    }

    #[test]
    fn index_options_preserve_concurrency_flags() {
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

        let options = parse_index_options(&mut args).expect("index options should parse");

        assert!(args.is_empty());
        assert_eq!(options.file.as_deref(), Some("selected.csv"));
        assert_eq!(options.worker_count, 4);
        assert_eq!(options.process_count, 3);
        assert_eq!(options.issue_batch_size, 2);
        assert!(options.is_issue_batch_explicit);
        assert_eq!(options.timeout_seconds, 7);
        assert!(!options.resume);
        assert!(options.update);
        assert!(!options.full_rescan);
        assert!(options.notify);
        assert!(options.notify_dry_run);
        assert!(!options.acknowledge_unknown_notify);
    }

    #[test]
    fn index_issue_batch_warns_once_only_when_explicit() {
        let root = temp_root("litradar-cli-legacy-issue-batch-warning");
        let arguments = |issue_batch: Option<&str>| {
            let mut arguments = vec![
                "--project-root".to_string(),
                root.path().to_string_lossy().into_owned(),
            ];
            if let Some(issue_batch) = issue_batch {
                arguments.extend(["--issue-batch".to_string(), issue_batch.to_string()]);
            }
            arguments
        };
        let captured = CapturedEvents::default();

        tracing::subscriber::with_default(captured.clone(), || {
            let default_error = run_index_command_with_bundled_meta_dir(
                arguments(None),
                Path::new("litradar"),
                None,
            )
            .expect_err("missing deployment key should stop the default command");
            let explicit_error = run_index_command_with_bundled_meta_dir(
                arguments(Some("5")),
                Path::new("litradar"),
                None,
            )
            .expect_err("missing deployment key should stop the explicit command");
            assert_eq!(default_error.to_string(), "--secret-key-file is required");
            assert_eq!(explicit_error.to_string(), "--secret-key-file is required");
        });

        let events = captured.snapshot();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(
            event.get("event").map(String::as_str),
            Some("cli.index.legacy_issue_batch")
        );
        assert_eq!(event.get("component").map(String::as_str), Some("cli"));
        assert_eq!(event.get("option").map(String::as_str), Some("issue_batch"));
        assert_eq!(
            event.get("behavior").map(String::as_str),
            Some("resume_compatibility_only")
        );
        assert_eq!(event.get("level").map(String::as_str), Some("WARN"));
        assert!(event.get("message").is_some_and(|message| {
            message.contains("does not control current Provider concurrency or memory")
        }));
        let serialized = serde_json::to_string(&events).expect("captured warning should serialize");
        assert!(!serialized.contains(&root.path().to_string_lossy().into_owned()));
        for prohibited in ["secret", "credential", "cursor"] {
            assert!(!serialized.contains(prohibited));
        }
    }

    #[test]
    fn index_unknown_notify_acknowledgement_requires_resumable_notification_mode() {
        let mut valid_args = vec![
            "--update".to_string(),
            "--notify".to_string(),
            "--acknowledge-unknown-notify".to_string(),
        ];
        let valid = parse_index_options(&mut valid_args)
            .expect("unknown notification acknowledgement should parse");
        assert!(valid_args.is_empty());
        assert!(valid.acknowledge_unknown_notify);

        let mut missing_notify = vec!["--acknowledge-unknown-notify".to_string()];
        assert_eq!(
            parse_index_options(&mut missing_notify)
                .expect_err("acknowledgement without notification should fail")
                .to_string(),
            "--acknowledge-unknown-notify requires --notify"
        );

        let mut no_resume = vec![
            "--update".to_string(),
            "--notify".to_string(),
            "--no-resume".to_string(),
            "--acknowledge-unknown-notify".to_string(),
        ];
        assert_eq!(
            parse_index_options(&mut no_resume)
                .expect_err("acknowledgement without recovery should fail")
                .to_string(),
            "--acknowledge-unknown-notify requires --resume"
        );
    }

    #[test]
    fn index_options_expose_full_rescan_and_reject_update_conflicts() {
        let mut full_rescan_args = vec!["--full-rescan".to_string()];
        let full_rescan =
            parse_index_options(&mut full_rescan_args).expect("full rescan should parse");
        assert!(full_rescan_args.is_empty());
        assert!(full_rescan.full_rescan);
        assert!(!full_rescan.update);
        assert!(full_rescan.resume);

        let mut conflict_args = vec!["--update".to_string(), "--full-rescan".to_string()];
        assert_eq!(
            parse_index_options(&mut conflict_args)
                .expect_err("update and full rescan must conflict")
                .to_string(),
            "--update cannot be combined with --full-rescan"
        );
    }

    #[test]
    fn index_issue_batch_keeps_legacy_default_independent_from_concurrency() {
        let mut args = vec!["--workers".to_string(), "5".to_string()];

        let options = parse_index_options(&mut args).expect("index options should parse");

        assert_eq!(options.worker_count, 5);
        assert_eq!(options.process_count, 1);
        assert_eq!(options.issue_batch_size, 8);
        assert!(!options.is_issue_batch_explicit);

        let mut default_args = Vec::new();
        let defaults =
            parse_index_options(&mut default_args).expect("default index options should parse");
        assert_eq!(defaults.worker_count, 6);
        assert_eq!(defaults.process_count, 1);
        assert_eq!(defaults.issue_batch_size, 8);
        assert!(!defaults.is_issue_batch_explicit);
    }

    #[test]
    fn index_workers_reject_generic_and_aggregate_overcommitment() {
        for (mut args, detail) in [
            (
                vec!["--workers".to_string(), "33".to_string()],
                "worker_count must be between 1 and 32",
            ),
            (
                vec!["--processes".to_string(), "33".to_string()],
                "process_count must be between 1 and 32",
            ),
            (
                vec![
                    "--workers".to_string(),
                    "17".to_string(),
                    "--processes".to_string(),
                    "2".to_string(),
                ],
                "process_count * worker_count must be at most 32",
            ),
        ] {
            assert_eq!(
                parse_index_options(&mut args)
                    .expect_err("unsafe index concurrency should fail")
                    .to_string(),
                detail
            );
        }
        let mut boundary = vec!["--workers".to_string(), "32".to_string()];
        assert_eq!(
            parse_index_options(&mut boundary)
                .expect("the generic worker boundary should pass")
                .worker_count,
            32
        );
    }

    #[test]
    fn index_outcome_reports_effective_concurrency_without_changing_status_shape() {
        let outcome = LiveIndexOutcome {
            status: "succeeded".to_string(),
            message: None,
            csvs: Vec::new(),
        };
        let options = IndexOptions {
            file: None,
            worker_count: 4,
            process_count: 2,
            issue_batch_size: 3,
            is_issue_batch_explicit: true,
            timeout_seconds: 20,
            resume: true,
            update: false,
            full_rescan: false,
            notify: false,
            notify_dry_run: false,
            acknowledge_unknown_notify: false,
        };
        let concurrency = litradar_domain::validate_index_concurrency(4, 2, false)
            .expect("test concurrency should validate");

        let payload: serde_json::Value = serde_json::from_str(
            &serialize_index_outcome(
                &outcome,
                index_concurrency_payload(&options, concurrency, &BTreeMap::new(), &outcome),
            )
            .expect("index outcome should serialize"),
        )
        .expect("serialized index outcome should be JSON");

        assert_eq!(payload["status"], "succeeded");
        assert_eq!(payload["message"], serde_json::Value::Null);
        assert_eq!(payload["csvs"], serde_json::json!([]));
        assert_eq!(payload["effective_concurrency"]["workers"], 4);
        assert_eq!(payload["effective_concurrency"]["processes"], 2);
        assert_eq!(payload["effective_concurrency"]["issue_batch"], 3);
        assert_eq!(
            payload["effective_concurrency"]["configured_aggregate_capacity"],
            8
        );
        assert_eq!(payload["effective_concurrency"]["effective_workers"], 0);
        assert_eq!(payload["effective_concurrency"]["effective_processes"], 0);
        assert_eq!(payload["effective_concurrency"]["aggregate_limit"], 32);
        assert!(payload.get("secret_key_file").is_none());

        let active_outcome = LiveIndexOutcome {
            status: "succeeded".to_string(),
            message: None,
            csvs: vec![LiveCsvIndexOutcome {
                csv_path: "data/meta/catalog.csv".to_string(),
                db_path: "data/index/catalog.sqlite".to_string(),
                run_id: "run".to_string(),
                status: "succeeded".to_string(),
                journal_count: 2,
                written_article_count: 0,
                source_attempt_count: 0,
                manifest_path: None,
                notify_exit_code: None,
            }],
        };
        let routes = BTreeMap::from([("catalog".to_string(), "cnki".to_string())]);
        let active = index_concurrency_payload(&options, concurrency, &routes, &active_outcome);
        assert_eq!(active["effective_workers"], 4);
        assert_eq!(active["effective_processes"], 2);
        assert_eq!(active["effective_aggregate_capacity"], 8);
    }

    #[test]
    fn index_command_resumes_a_precompleted_local_catalog() {
        let root = temp_root("litradar-cli-offline-index");
        let project_root = root.path().join("project");
        let storage_config = litradar_storage::StorageConfig::from_project_root(&project_root);
        let secret_key_file = root.path().join("secret.key");
        fs::write(&secret_key_file, [18_u8; 32]).expect("secret key should write");
        fs::create_dir_all(storage_config.meta_dir())
            .expect("metadata directory should be created");
        fs::write(
            storage_config.meta_dir().join("offline.csv"),
            "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating\nissn-0001-3072,,Abacus,0001-3072,1467-6281,0001-3072;1467-6281,,Accounting & Auditing,,,7,3,7,B,,\n",
        )
        .expect("local catalog should write");
        litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
        let codec =
            litradar_storage::SecretCodec::load(&secret_key_file).expect("codec should load");
        litradar_storage::upsert_runtime_settings(
            storage_config.auth_db_path(),
            &codec,
            &HashMap::from([(
                "index_provider_routes".to_string(),
                Some(r#"{"offline":"offline_fixture"}"#.to_string()),
            )]),
            &HashMap::new(),
        )
        .expect("offline provider route should write");
        let arguments = || {
            vec![
                "--project-root".to_string(),
                project_root.to_string_lossy().into_owned(),
                "--secret-key-file".to_string(),
                secret_key_file.to_string_lossy().into_owned(),
                "--file".to_string(),
                "offline.csv".to_string(),
                "--workers".to_string(),
                "1".to_string(),
                "--processes".to_string(),
                "1".to_string(),
                "--issue-batch".to_string(),
                "1".to_string(),
                "--timeout".to_string(),
                "1".to_string(),
            ]
        };
        let error =
            run_index_command_with_bundled_meta_dir(arguments(), Path::new("litradar"), None)
                .expect_err("fixture provider should stop before network access");
        assert!(error
            .to_string()
            .contains("index provider offline_fixture is not registered"));

        let batch_connection = litradar_storage::open_sqlite_connection(
            storage_config
                .index_control_dir()
                .join("index-batches.sqlite"),
        )
        .expect("batch database should open");
        let batch_id = batch_connection
            .query_row(
                "SELECT batch_id FROM index_batches WHERE status = 'active'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("active batch should exist");
        assert_eq!(
            batch_connection
                .execute(
                    "UPDATE index_batch_catalogs
                     SET phase = 'completed', run_id = ?2, written_article_count = ?3,
                         source_attempt_count = ?4, updated_at = ?5, completed_at = ?5
                     WHERE batch_id = ?1 AND ordinal = 0 AND phase = 'indexing'",
                    (&batch_id, "offline-complete-run", 0_i64, 0_i64, 1_i64),
                )
                .expect("fixture catalog should complete"),
            1
        );
        drop(batch_connection);

        run_index_command_with_bundled_meta_dir(arguments(), Path::new("litradar"), None)
            .expect("precompleted batch catalog should resume without Provider access");

        assert!(storage_config.index_dir().join("offline.sqlite").is_file());
        assert!(storage_config
            .index_control_dir()
            .join("offline.sqlite")
            .is_file());
    }

    #[test]
    fn index_command_rejects_explicit_file_against_active_all_batch() {
        let root = temp_root("litradar-cli-index-selection");
        let project_root = root.path().join("project");
        let storage_config = litradar_storage::StorageConfig::from_project_root(&project_root);
        let secret_key_file = root.path().join("secret.key");
        fs::write(&secret_key_file, [19_u8; 32]).expect("secret key should write");
        fs::create_dir_all(storage_config.meta_dir())
            .expect("metadata directory should be created");
        fs::write(
            storage_config.meta_dir().join("offline.csv"),
            "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating\nissn-0001-3072,,Abacus,0001-3072,1467-6281,0001-3072;1467-6281,,Accounting & Auditing,,,7,3,7,B,,\n",
        )
        .expect("local catalog should write");
        litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
        let codec =
            litradar_storage::SecretCodec::load(&secret_key_file).expect("codec should load");
        litradar_storage::upsert_runtime_settings(
            storage_config.auth_db_path(),
            &codec,
            &HashMap::from([(
                "index_provider_routes".to_string(),
                Some(r#"{"offline":"offline_fixture"}"#.to_string()),
            )]),
            &HashMap::new(),
        )
        .expect("offline provider route should write");
        let common_arguments = || {
            vec![
                "--project-root".to_string(),
                project_root.to_string_lossy().into_owned(),
                "--secret-key-file".to_string(),
                secret_key_file.to_string_lossy().into_owned(),
                "--workers".to_string(),
                "1".to_string(),
                "--processes".to_string(),
                "1".to_string(),
                "--issue-batch".to_string(),
                "1".to_string(),
                "--timeout".to_string(),
                "1".to_string(),
            ]
        };
        run_index_command_with_bundled_meta_dir(common_arguments(), Path::new("litradar"), None)
            .expect_err("fixture provider should leave an active all-CSV batch");
        let batch_path = storage_config
            .index_control_dir()
            .join("index-batches.sqlite");
        let before = litradar_storage::open_sqlite_connection(&batch_path)
            .expect("batch database should open")
            .query_row(
                "SELECT phase, run_id, written_article_count, source_attempt_count
                 FROM index_batch_catalogs WHERE ordinal = 0",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .expect("batch catalog state should read");
        let mut explicit_arguments = common_arguments();
        explicit_arguments.extend(["--file".to_string(), "offline.csv".to_string()]);

        let error = run_index_command_with_bundled_meta_dir(
            explicit_arguments,
            Path::new("litradar"),
            None,
        )
        .expect_err("explicit selection must not adopt an active all-CSV batch");
        assert!(error.to_string().contains("catalog_selection"));
        let after = litradar_storage::open_sqlite_connection(batch_path)
            .expect("batch database should reopen")
            .query_row(
                "SELECT phase, run_id, written_article_count, source_attempt_count
                 FROM index_batch_catalogs WHERE ordinal = 0",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .expect("batch catalog state should remain readable");
        assert_eq!(before, after);
    }

    #[test]
    fn configured_index_preparation_uses_the_explicit_auth_database() {
        let root = temp_root("litradar-cli-managed-meta");
        let project_root = root.path().join("project");
        let auth_db_path = root.path().join("state/custom-auth.sqlite");
        let bundle_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/meta");
        preflight_command_storage(&project_root, &auth_db_path)
            .expect("command storage should be prepared");
        let storage_config = litradar_storage::StorageConfig::from_project_root(&project_root)
            .with_auth_db_path(&auth_db_path);

        let report = prepare_index_managed_meta(&storage_config, Some(&bundle_dir))
            .expect("configured bundle should prepare")
            .expect("configured preparation should return a report");

        assert_eq!(report.catalogs.len(), 3);
        assert!(storage_config
            .meta_dir()
            .join("ccf_computer_journals.csv")
            .is_file());
        let state_count: i64 = litradar_storage::open_sqlite_connection(&auth_db_path)
            .expect("explicit auth database should open")
            .query_row("SELECT COUNT(*) FROM managed_meta_catalogs", [], |row| {
                row.get(0)
            })
            .expect("managed state should load");
        assert_eq!(state_count, 3);
        assert!(!project_root.join("data/auth.sqlite").exists());
    }

    #[test]
    fn command_storage_preflight_imports_legacy_delivery_state() {
        let root = temp_root("litradar-cli-delivery-import");
        let project_root = root.path().join("project");
        let auth_db_path = root.path().join("state/custom-auth.sqlite");
        let state_dir = project_root.join("data/push_state");
        fs::create_dir_all(&state_dir).expect("legacy state directory should be created");
        let state_path = state_dir.join("fixture.json");
        fs::write(
            &state_path,
            r#"{"db_name":"fixture.sqlite","status":"completed","delivery_dedupe":{"7:41":"2026-07-27T00:00:00Z"}}"#,
        )
        .expect("legacy state should be written");

        preflight_command_storage(&project_root, &auth_db_path)
            .expect("command startup should import legacy state");

        let checkpoint = litradar_storage::load_delivery_checkpoint(
            &auth_db_path,
            litradar_storage::DeliveryWorkflow::Notify,
            "fixture.sqlite",
        )
        .expect("delivery checkpoint should load")
        .expect("delivery checkpoint should exist");
        assert_eq!(
            checkpoint.status,
            litradar_storage::DeliveryCheckpointStatus::Completed
        );
        assert_eq!(
            litradar_storage::list_delivery_dedupe_for_scope(
                &auth_db_path,
                litradar_storage::DeliveryWorkflow::Notify,
                "fixture.sqlite",
            )
            .expect("delivery dedupe should load")
            .len(),
            1
        );
        assert!(state_path.exists());
    }

    #[test]
    fn selected_index_migration_does_not_open_or_modify_a_sibling_database() {
        let root = temp_root("litradar-cli-selected-index-migration");
        let project_root = root.path().join("project");
        let index_dir = project_root.join("data/index");
        fs::create_dir_all(&index_dir).expect("index directory should be created");
        let english_path = index_dir.join("english_journals.sqlite");
        create_version_four_content_database(&english_path);
        let ccf_path = index_dir.join("ccf_computer_journals.sqlite");
        let ccf_connection = litradar_storage::open_sqlite_connection(&ccf_path)
            .expect("sibling access sentinel should open");
        ccf_connection
            .execute_batch(
                "CREATE TABLE access_sentinel (value TEXT NOT NULL);
                 INSERT INTO access_sentinel VALUES ('untouched');
                 PRAGMA user_version = 4;",
            )
            .expect("sibling access sentinel should initialize");
        drop(ccf_connection);
        let ccf_before = fs::read(&ccf_path).expect("sibling bytes should read");

        preflight_index_command_databases(&project_root, Some("english_journals.csv"))
            .expect("selected index preflight should succeed without inspecting its sibling");

        assert_eq!(
            content_database_version(&english_path),
            litradar_storage::INDEX_SCHEMA_VERSION
        );
        assert_eq!(
            fs::read(&ccf_path).expect("sibling bytes should remain readable"),
            ccf_before
        );
    }

    #[test]
    fn default_index_migration_keeps_all_database_scope() {
        let root = temp_root("litradar-cli-default-index-migration");
        let project_root = root.path().join("project");
        let index_dir = project_root.join("data/index");
        fs::create_dir_all(&index_dir).expect("index directory should be created");
        let english_path = index_dir.join("english_journals.sqlite");
        let ccf_path = index_dir.join("ccf_computer_journals.sqlite");
        create_version_four_content_database(&english_path);
        create_version_four_content_database(&ccf_path);

        preflight_index_command_databases(&project_root, None)
            .expect("default index preflight should include every database");

        assert_eq!(
            content_database_version(&english_path),
            litradar_storage::INDEX_SCHEMA_VERSION
        );
        assert_eq!(
            content_database_version(&ccf_path),
            litradar_storage::INDEX_SCHEMA_VERSION
        );
    }

    #[test]
    fn unset_index_bundle_keeps_local_metadata_behavior() {
        let root = temp_root("litradar-cli-unset-managed-meta");
        let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
        litradar_storage::migrate_auth_database(storage_config.auth_db_path())
            .expect("auth database should migrate");

        let report = prepare_index_managed_meta(&storage_config, None)
            .expect("unset bundle should be a no-op");

        assert!(report.is_none());
        assert!(!storage_config.meta_dir().exists());
    }

    #[test]
    fn internal_index_worker_short_circuits_before_bundle_preparation() {
        let root = temp_root("litradar-cli-internal-worker-meta");
        let error = run_index_command_with_bundled_meta_dir(
            vec![
                "--live-worker-request".to_string(),
                root.path()
                    .join("missing-request.json")
                    .to_string_lossy()
                    .into_owned(),
                "unexpected".to_string(),
            ],
            Path::new("litradar"),
            Some(root.path().join("missing-bundle")),
        )
        .expect_err("worker argument validation should run before preparation");

        assert_eq!(
            error.to_string(),
            "unexpected index worker arguments: unexpected"
        );
        assert!(!root.path().join("data").exists());
    }

    #[test]
    fn legacy_index_failure_names_exact_file_before_machine_result_output() {
        let root = temp_root("litradar-cli-legacy-index-rebuild");
        let project_root = root.path().join("project");
        let index_dir = project_root.join("data/index");
        fs::create_dir_all(&index_dir).expect("index directory should create");
        let legacy_path = index_dir.join("legacy.sqlite");
        let connection = litradar_storage::open_sqlite_connection(&legacy_path)
            .expect("legacy index database should open");
        connection
            .execute_batch(
                "CREATE TABLE journals (journal_id INTEGER PRIMARY KEY);
                 PRAGMA user_version = 3;",
            )
            .expect("legacy schema should initialize");
        drop(connection);
        let secret_key_file = root.path().join("secret.key");
        fs::write(&secret_key_file, [7_u8; 32]).expect("secret key should write");

        let error = run_index_command_with_bundled_meta_dir(
            vec![
                "--project-root".to_string(),
                project_root.to_string_lossy().into_owned(),
                "--secret-key-file".to_string(),
                secret_key_file.to_string_lossy().into_owned(),
            ],
            Path::new("litradar"),
            None,
        )
        .expect_err("legacy index should require a deliberate rebuild");
        let message = error.to_string();
        let expected_path = legacy_path.display().to_string();
        #[cfg(windows)]
        let expected_path = expected_path
            .strip_prefix(r"\\?\")
            .unwrap_or(&expected_path)
            .replace('/', "\\");
        assert!(
            message.contains(&expected_path),
            "expected path {expected_path:?}; unexpected diagnostic: {message}"
        );
        assert!(message.contains("legacy schema version 3"));
        assert!(message.contains("move or delete that exact file and rebuild"));
    }

    #[test]
    fn index_options_reject_zero_parallelism_values() {
        for arguments in [
            vec!["--workers".to_string(), "0".to_string()],
            vec!["--processes".to_string(), "0".to_string()],
            vec!["--issue-batch".to_string(), "0".to_string()],
        ] {
            let mut arguments = arguments;
            let error = parse_index_options(&mut arguments).expect_err("zero value should fail");

            assert!(error.to_string().contains("must be at least 1"));
        }
    }

    #[test]
    fn delivery_usage_exposes_supported_flags() {
        let notify_usage = delivery_usage(DeliveryWorkflow::Notify);
        let push_usage = delivery_usage(DeliveryWorkflow::Push);

        assert!(notify_usage.contains("notify --secret-key-file PATH"));
        assert!(push_usage.contains("push --secret-key-file PATH"));
        assert!(notify_usage.contains("--dry-run|--no-dry-run"));
        assert!(push_usage.contains("--changes-file PATH"));
        assert!(!notify_usage.contains("--state-dir"));
        assert!(!notify_usage.contains("--internal-handoff-json"));
        assert!(!notify_usage.contains("--attempt-id"));
    }

    #[test]
    fn internal_delivery_handoff_requires_a_bounded_attempt_identity() {
        let missing = run_notify_command(vec!["--internal-handoff-json".to_string()])
            .expect_err("internal handoff without an attempt should fail");
        assert_eq!(
            missing.to_string(),
            "--internal-handoff-json requires --attempt-id"
        );

        let unscoped = run_notify_command(vec![
            "--attempt-id".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        ])
        .expect_err("attempt identity outside an internal handoff should fail");
        assert_eq!(
            unscoped.to_string(),
            "--attempt-id requires --internal-handoff-json"
        );

        let invalid = run_notify_command(vec![
            "--internal-handoff-json".to_string(),
            "--attempt-id".to_string(),
            "not-an-attempt".to_string(),
        ])
        .expect_err("malformed attempt identity should fail");
        assert_eq!(
            invalid.to_string(),
            "--attempt-id must be exactly 32 hexadecimal characters"
        );
    }

    #[test]
    fn delivery_status_aggregation_and_exit_contract_cover_every_state() {
        let outcome = |status: DeliveryOutcomeState| RecommendationRunOutcome {
            db_name: "fixture.sqlite".to_string(),
            workflow: DeliveryWorkflow::Notify,
            mode: DeliveryMode::Execute,
            status,
            delivery_run_id: 1,
            candidate_article_ids: Vec::new(),
            subscribers: Vec::new(),
        };

        assert_eq!(aggregate_delivery_status(&[]), DeliveryOutcomeState::Idle);
        for status in [
            DeliveryOutcomeState::Idle,
            DeliveryOutcomeState::Skipped,
            DeliveryOutcomeState::Completed,
            DeliveryOutcomeState::Running,
            DeliveryOutcomeState::Cancelled,
            DeliveryOutcomeState::TimedOut,
            DeliveryOutcomeState::Failed,
            DeliveryOutcomeState::Unknown,
        ] {
            assert_eq!(aggregate_delivery_status(&[outcome(status)]), status);
        }
        let precedence = [
            DeliveryOutcomeState::Idle,
            DeliveryOutcomeState::Skipped,
            DeliveryOutcomeState::Completed,
            DeliveryOutcomeState::Running,
            DeliveryOutcomeState::Cancelled,
            DeliveryOutcomeState::TimedOut,
            DeliveryOutcomeState::Failed,
            DeliveryOutcomeState::Unknown,
        ];
        for (expected_index, expected) in precedence.iter().copied().enumerate() {
            let outcomes = precedence[..=expected_index]
                .iter()
                .copied()
                .map(&outcome)
                .collect::<Vec<_>>();
            assert_eq!(aggregate_delivery_status(&outcomes), expected);
        }
        for status in [
            DeliveryOutcomeState::Idle,
            DeliveryOutcomeState::Skipped,
            DeliveryOutcomeState::Completed,
        ] {
            ensure_delivery_command_success("notify", status)
                .expect("successful terminal state should exit zero");
        }
        for status in [
            DeliveryOutcomeState::Running,
            DeliveryOutcomeState::Cancelled,
            DeliveryOutcomeState::TimedOut,
            DeliveryOutcomeState::Failed,
            DeliveryOutcomeState::Unknown,
        ] {
            let error = ensure_delivery_command_success("push", status)
                .expect_err("non-success state should exit nonzero");
            assert_eq!(
                error.to_string(),
                format!("push delivery finished with {} status", status.as_str())
            );
        }
    }

    #[test]
    fn delivery_retry_arguments_are_bounded() {
        let root = temp_root("litradar-cli-delivery-retries");
        let auth_db_path = root.path().join("auth.sqlite");

        for retry_attempts in [0_usize, 3, 10] {
            for workflow in [DeliveryWorkflow::Notify, DeliveryWorkflow::Push] {
                let arguments = vec![
                    "--project-root".to_string(),
                    root.path().to_string_lossy().into_owned(),
                    "--auth-db".to_string(),
                    auth_db_path.to_string_lossy().into_owned(),
                    "--retries".to_string(),
                    retry_attempts.to_string(),
                ];
                let error = match workflow {
                    DeliveryWorkflow::Notify => run_notify_command(arguments),
                    DeliveryWorkflow::Push => run_push_command(arguments),
                }
                .expect_err("bounded retry count should reach secret-key validation");

                assert_eq!(error.to_string(), "--secret-key-file is required");
                assert!(!auth_db_path.exists());
            }
        }

        for retry_attempts in [11_usize, usize::MAX] {
            for workflow in [DeliveryWorkflow::Notify, DeliveryWorkflow::Push] {
                let arguments = vec![
                    "--project-root".to_string(),
                    root.path().to_string_lossy().into_owned(),
                    "--auth-db".to_string(),
                    auth_db_path.to_string_lossy().into_owned(),
                    "--retries".to_string(),
                    retry_attempts.to_string(),
                ];
                let error = match workflow {
                    DeliveryWorkflow::Notify => run_notify_command(arguments),
                    DeliveryWorkflow::Push => run_push_command(arguments),
                }
                .expect_err("oversized retry count should fail before runtime setup");

                assert_eq!(error.to_string(), "--retries must be between 0 and 10");
                assert!(!auth_db_path.exists());
            }
        }
    }

    #[test]
    fn delivery_targets_resolve_manifest_database() {
        let root = temp_root("litradar-cli-targets");
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
        let root = temp_root("litradar-cli-all-dbs");
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
    fn delivery_defaults_match_standalone_commands() {
        assert_eq!(normalize_db_name("utd24"), "utd24.sqlite");
        assert_eq!(normalize_db_name("data/index/utd24.sqlite"), "utd24.sqlite");
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
        let root = temp_root("litradar-cli-missing-manifest-db");
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
        let root = temp_root("litradar-cli-missing-db-targets");
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
        let root = temp_root("litradar-cli-project-path");
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
    fn index_command_notify_requires_update_before_live_execution() {
        let error = run_index_command(vec!["--notify".to_string()], Path::new("litradar"))
            .expect_err("notify handoff should require update mode");

        assert_eq!(error.to_string(), "--notify requires --update");
    }

    #[test]
    fn index_command_rejects_update_and_full_rescan_before_live_execution() {
        let error = run_index_command(
            vec!["--update".to_string(), "--full-rescan".to_string()],
            Path::new("litradar"),
        )
        .expect_err("conflicting synchronization modes should fail before live execution");

        assert_eq!(
            error.to_string(),
            "--update cannot be combined with --full-rescan"
        );
    }

    #[test]
    fn help_and_delivery_errors_return_before_execution() {
        run_scheduler_command(vec!["--help".to_string()], Path::new("litradar"))
            .expect("scheduler help should succeed");

        let error = run_notify_command(vec!["--unexpected".to_string()])
            .expect_err("unexpected delivery args should fail");

        assert_eq!(
            error.to_string(),
            "unexpected notify arguments: --unexpected"
        );
    }

    #[test]
    fn scheduler_command_requires_valid_task_id() {
        let root = temp_root("litradar-cli-scheduler-dispatch");
        let auth_db_path = root.path().join("auth.sqlite");

        let error = run_scheduler_command(
            vec![
                "--auth-db".to_string(),
                auth_db_path.to_string_lossy().into_owned(),
                "run-once".to_string(),
                "not-a-number".to_string(),
            ],
            Path::new("litradar"),
        )
        .expect_err("invalid scheduler task id should fail before execution");

        assert!(error.to_string().contains("invalid digit"));
        assert!(!auth_db_path.exists());
    }

    #[test]
    fn scheduler_startup_migration_precedes_job_load() {
        let root = temp_root("litradar-cli-scheduler-validate");
        let auth_db_path = root.path().join("auth.sqlite");
        let secret_key_file = root.path().join("secret.key");
        fs::write(&secret_key_file, [5_u8; 32]).expect("secret key should write");
        let index_path = root.path().join("data/index/current-with-orphan.sqlite");
        litradar_storage::migrate_index_database(&index_path, None)
            .expect("current index database should initialize");
        let connection = litradar_storage::open_sqlite_connection(&index_path)
            .expect("current index database should open");
        connection
            .pragma_update(None, "foreign_keys", false)
            .expect("foreign key enforcement should be disabled for the corruption fixture");
        connection
            .execute(
                "INSERT INTO article_retraction_dois (article_id, retraction_doi)
                 VALUES (999, '10.1000/orphan')",
                [],
            )
            .expect("foreign key violation should be installed with enforcement disabled");
        drop(connection);

        run_scheduler_command(
            vec![
                "--project-root".to_string(),
                root.path().to_string_lossy().into_owned(),
                "--auth-db".to_string(),
                auth_db_path.to_string_lossy().into_owned(),
                "--secret-key-file".to_string(),
                secret_key_file.to_string_lossy().into_owned(),
                "validate".to_string(),
            ],
            Path::new("litradar"),
        )
        .expect("scheduler validate should load jobs");
        assert_eq!(
            litradar_storage::count_users(&auth_db_path).expect("migrated users table should load"),
            0
        );
        let violation_count: i64 = litradar_storage::open_sqlite_connection(&index_path)
            .expect("current index database should reopen")
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .expect("foreign key violation count should load");
        assert_eq!(violation_count, 1);
    }

    #[test]
    fn removed_public_command_paths_are_rejected() {
        let index_fixture = run_index_command(vec!["fixture".to_string()], Path::new("litradar"))
            .expect_err("fixture command is removed");
        let notify_positional_dry_run = run_notify_command(vec!["dry-run".to_string()])
            .expect_err("positional notify dry-run is removed");
        let push_shadow =
            run_push_command(vec!["shadow".to_string()]).expect_err("push shadow is removed");
        let scheduler_dry_run =
            run_scheduler_command(vec!["dry-run".to_string()], Path::new("litradar"))
                .expect_err("scheduler dry-run alias is removed");

        assert!(index_fixture
            .to_string()
            .contains("unexpected index arguments: fixture"));
        assert!(notify_positional_dry_run
            .to_string()
            .contains("unexpected notify arguments: dry-run"));
        assert!(push_shadow
            .to_string()
            .contains("unexpected push arguments: shadow"));
        assert!(scheduler_dry_run.to_string().contains("scheduler validate"));
    }

    fn temp_root(prefix: &str) -> TempDir {
        Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("temp root should be created")
    }

    fn create_version_four_content_database(path: &Path) {
        litradar_storage::migrate_index_database(path, None)
            .expect("current content database should initialize");
        let connection = litradar_storage::open_sqlite_connection(path)
            .expect("content database should open for downgrade fixture");
        connection
            .execute_batch(
                "DROP TABLE article_search;
                 CREATE VIRTUAL TABLE article_search
                 USING fts5(
                     article_id UNINDEXED,
                     title,
                     abstract_text,
                     doi,
                     pmid,
                     authors,
                     journal_title,
                     tokenize = 'unicode61 remove_diacritics 2'
                 );
                 DROP TABLE article_retraction_dois;
                 ALTER TABLE articles ADD COLUMN retraction_doi TEXT;
                 DROP TABLE journal_identity_keys;
                 PRAGMA user_version = 4;",
            )
            .expect("version four fixture should be created");
    }

    fn content_database_version(path: &Path) -> i64 {
        litradar_storage::open_sqlite_connection(path)
            .expect("content database should open for version query")
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("content database version should read")
    }
}
