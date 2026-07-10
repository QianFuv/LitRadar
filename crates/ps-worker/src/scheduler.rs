//! Scheduler validation and typed job execution utilities.

use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::{ScheduledDeliveryJob, ScheduledJobSpec, ScheduledTaskInfo};
use serde::Serialize;

/// Worker scheduler errors.
#[derive(Debug)]
pub enum SchedulerError {
    /// Storage repository failed.
    Storage(ps_storage::BusinessRepositoryError),
    /// Cron expression is invalid.
    InvalidCron(String),
    /// Typed job arguments are invalid.
    InvalidJob(String),
}

impl fmt::Display for SchedulerError {
    /// Format the scheduler error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::InvalidCron(message) => formatter.write_str(message),
            Self::InvalidJob(message) => formatter.write_str(message),
        }
    }
}

impl Error for SchedulerError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::InvalidCron(_) | Self::InvalidJob(_) => None,
        }
    }
}

impl From<ps_storage::BusinessRepositoryError> for SchedulerError {
    /// Convert storage repository errors into scheduler errors.
    fn from(error: ps_storage::BusinessRepositoryError) -> Self {
        Self::Storage(error)
    }
}

/// Scheduler execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerMode {
    /// Load scheduled jobs without executing processes.
    DryRun,
    /// Execute typed jobs and write back status.
    Execute,
}

/// Runnable scheduler job metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduledJob {
    /// Scheduled task row identifier.
    pub id: i64,
    /// Stable scheduler job identifier.
    pub job_id: String,
    /// Display name.
    pub name: String,
    /// Validated typed job specification.
    pub job: ScheduledJobSpec,
    /// Five-field cron expression.
    pub cron: String,
    /// APScheduler-compatible `max_instances` setting.
    pub max_instances: i64,
    /// APScheduler-compatible coalescing setting.
    pub coalesce: bool,
}

/// Scheduled task skipped while loading jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedScheduledTask {
    /// Scheduled task row identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Invalid cron expression.
    pub cron: String,
    /// Skip reason.
    pub reason: String,
}

/// Loaded scheduler jobs and skipped tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchedulerLoadResult {
    /// Runnable jobs.
    pub jobs: Vec<ScheduledJob>,
    /// Enabled tasks skipped because they cannot be scheduled.
    pub skipped: Vec<SkippedScheduledTask>,
}

/// Manual task run outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunTaskOutcome {
    /// Whether the task row exists.
    pub found: bool,
    /// Whether a typed job was executed.
    pub did_execute: bool,
    /// Execution status or a reason the task could not execute.
    pub status: Option<String>,
}

/// Scheduler execution slot used to avoid duplicate same-minute runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScheduledRunSlot {
    /// Scheduled task row identifier.
    pub task_id: i64,
    /// Unix minute bucket.
    pub minute_epoch: i64,
}

/// One scheduled task execution record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduledTaskExecution {
    /// Scheduled task row identifier.
    pub task_id: i64,
    /// Stable scheduler job identifier.
    pub job_id: String,
    /// Display name.
    pub name: String,
    /// Stored execution status.
    pub status: String,
}

/// One scheduler execute-loop tick result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchedulerExecutionResult {
    /// Scheduler mode.
    pub mode: SchedulerMode,
    /// Tick status.
    pub status: String,
    /// Current Unix minute bucket.
    pub minute_epoch: i64,
    /// Runnable job count.
    pub jobs: usize,
    /// Enabled tasks skipped because they cannot be scheduled.
    pub skipped: Vec<SkippedScheduledTask>,
    /// Due task count before duplicate suppression.
    pub due: usize,
    /// Same-minute duplicate count.
    pub already_executed: usize,
    /// Executed tasks.
    pub executed: Vec<ScheduledTaskExecution>,
}

trait ScheduledJobRunner {
    fn run(&mut self, auth_db_path: &Path, task: &ScheduledTaskInfo) -> String;
}

struct ProcessScheduledJobRunner;

impl ScheduledJobRunner for ProcessScheduledJobRunner {
    fn run(&mut self, auth_db_path: &Path, task: &ScheduledTaskInfo) -> String {
        task.job.as_ref().map_or_else(
            || "blocked: legacy task requires a typed job".to_string(),
            |job| execute_scheduled_job(auth_db_path, job),
        )
    }
}

/// Validate a five-field crontab expression.
///
/// # Arguments
///
/// * `cron` - Five-field cron expression.
///
/// # Returns
///
/// `Ok(())` when the expression is valid.
pub fn validate_cron_expression(cron: &str) -> Result<(), SchedulerError> {
    let fields = cron.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(SchedulerError::InvalidCron(
            "Cron expression must contain exactly five fields".to_string(),
        ));
    }
    validate_cron_field(fields[0], 0, 59, &[])?;
    validate_cron_field(fields[1], 0, 23, &[])?;
    validate_cron_field(fields[2], 1, 31, &[])?;
    validate_cron_field(
        fields[3],
        1,
        12,
        &[
            ("jan", 1),
            ("feb", 2),
            ("mar", 3),
            ("apr", 4),
            ("may", 5),
            ("jun", 6),
            ("jul", 7),
            ("aug", 8),
            ("sep", 9),
            ("oct", 10),
            ("nov", 11),
            ("dec", 12),
        ],
    )?;
    validate_cron_field(
        fields[4],
        0,
        7,
        &[
            ("sun", 0),
            ("mon", 1),
            ("tue", 2),
            ("wed", 3),
            ("thu", 4),
            ("fri", 5),
            ("sat", 6),
        ],
    )?;
    Ok(())
}

/// Load enabled scheduled jobs without executing processes.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Runnable jobs and skipped invalid tasks.
pub fn load_scheduler_jobs(
    auth_db_path: impl AsRef<Path>,
) -> Result<SchedulerLoadResult, SchedulerError> {
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for task in ps_storage::list_scheduled_tasks(auth_db_path)? {
        if !task.enabled {
            continue;
        }
        let validation =
            validate_cron_expression(&task.cron).and_then(|()| validate_job(task.job.as_ref()));
        match validation {
            Ok(()) => jobs.push(scheduled_job(&task)),
            Err(error) => skipped.push(SkippedScheduledTask {
                id: task.id,
                name: task.name,
                cron: task.cron,
                reason: error.to_string(),
            }),
        }
    }
    Ok(SchedulerLoadResult { jobs, skipped })
}

/// Run a scheduled task immediately.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
/// * `mode` - Scheduler execution mode.
///
/// # Returns
///
/// Manual run outcome.
pub fn run_task_now(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
    mode: SchedulerMode,
) -> Result<RunTaskOutcome, SchedulerError> {
    let mut runner = ProcessScheduledJobRunner;
    run_task_now_with_runner(auth_db_path.as_ref(), task_id, mode, &mut runner)
}

fn run_task_now_with_runner(
    auth_db_path: &Path,
    task_id: i64,
    mode: SchedulerMode,
    runner: &mut impl ScheduledJobRunner,
) -> Result<RunTaskOutcome, SchedulerError> {
    let Some(task) = ps_storage::get_scheduled_task(auth_db_path, task_id)? else {
        return Ok(RunTaskOutcome {
            found: false,
            did_execute: false,
            status: None,
        });
    };
    if mode != SchedulerMode::Execute {
        return Ok(RunTaskOutcome {
            found: true,
            did_execute: false,
            status: None,
        });
    }
    let Some(job) = task.job.as_ref() else {
        return Ok(RunTaskOutcome {
            found: true,
            did_execute: false,
            status: Some("blocked: legacy task requires a typed job".to_string()),
        });
    };
    validate_job(Some(job))?;
    let ran_at = current_unix_time();
    let status = runner.run(auth_db_path, &task);
    ps_storage::record_scheduled_task_run(auth_db_path, task.id, &status, ran_at)?;
    Ok(RunTaskOutcome {
        found: true,
        did_execute: true,
        status: Some(status),
    })
}

/// Execute due scheduled tasks once for the current minute.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `executed_slots` - Mutable same-minute execution guard.
///
/// # Returns
///
/// Execution tick result.
pub fn run_due_scheduler_once(
    auth_db_path: impl AsRef<Path>,
    executed_slots: &mut BTreeSet<ScheduledRunSlot>,
) -> Result<SchedulerExecutionResult, SchedulerError> {
    let mut runner = ProcessScheduledJobRunner;
    run_due_scheduler_once_at_with_runner(
        auth_db_path.as_ref(),
        current_unix_time(),
        executed_slots,
        &mut runner,
    )
}

fn run_due_scheduler_once_at_with_runner(
    auth_db_path: &Path,
    now: f64,
    executed_slots: &mut BTreeSet<ScheduledRunSlot>,
    runner: &mut impl ScheduledJobRunner,
) -> Result<SchedulerExecutionResult, SchedulerError> {
    let current_minute = (now as i64).div_euclid(60);
    let current_time = cron_time_from_unix_seconds(now as i64);
    let mut jobs = 0;
    let mut skipped = Vec::new();
    let mut due = 0;
    let mut already_executed = 0;
    let mut executed = Vec::new();

    for task in ps_storage::list_scheduled_tasks(auth_db_path)? {
        if !task.enabled {
            continue;
        }
        let validation =
            validate_cron_expression(&task.cron).and_then(|()| validate_job(task.job.as_ref()));
        match validation {
            Ok(()) => jobs += 1,
            Err(error) => {
                skipped.push(SkippedScheduledTask {
                    id: task.id,
                    name: task.name,
                    cron: task.cron,
                    reason: error.to_string(),
                });
                continue;
            }
        }
        if !cron_matches_time(&task.cron, current_time)? {
            continue;
        }
        due += 1;
        let slot = ScheduledRunSlot {
            task_id: task.id,
            minute_epoch: current_minute,
        };
        if !executed_slots.insert(slot) {
            already_executed += 1;
            continue;
        }
        let status = runner.run(auth_db_path, &task);
        ps_storage::record_scheduled_task_run(auth_db_path, task.id, &status, now)?;
        executed.push(ScheduledTaskExecution {
            task_id: task.id,
            job_id: format!("scheduled-task-{}", task.id),
            name: task.name,
            status,
        });
    }

    Ok(SchedulerExecutionResult {
        mode: SchedulerMode::Execute,
        status: "running".to_string(),
        minute_epoch: current_minute,
        jobs,
        skipped,
        due,
        already_executed,
        executed,
    })
}

fn scheduled_job(task: &ScheduledTaskInfo) -> ScheduledJob {
    ScheduledJob {
        id: task.id,
        job_id: format!("scheduled-task-{}", task.id),
        name: task.name.clone(),
        job: task
            .job
            .clone()
            .expect("validated runnable task should have a typed job"),
        cron: task.cron.clone(),
        max_instances: 1,
        coalesce: true,
    }
}

fn validate_job(job: Option<&ScheduledJobSpec>) -> Result<(), SchedulerError> {
    let job = job.ok_or_else(|| {
        SchedulerError::InvalidJob("Legacy task requires a typed job".to_string())
    })?;
    job.validate()
        .map_err(|error| SchedulerError::InvalidJob(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduledProcess {
    executable: &'static str,
    arguments: Vec<OsString>,
}

fn execute_scheduled_job(auth_db_path: &Path, job: &ScheduledJobSpec) -> String {
    let processes = match scheduled_processes(auth_db_path, job) {
        Ok(processes) => processes,
        Err(error) => return format!("error: {error}"),
    };
    for process in processes {
        let mut command = Command::new(process.executable);
        command.args(process.arguments);
        match command.output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                return output
                    .status
                    .code()
                    .map_or_else(|| "failed".to_string(), |code| format!("failed ({code})"));
            }
            Err(error) => return format!("error: {error}"),
        }
    }
    "success".to_string()
}

fn scheduled_processes(
    auth_db_path: &Path,
    job: &ScheduledJobSpec,
) -> Result<Vec<ScheduledProcess>, SchedulerError> {
    validate_job(Some(job))?;
    let mut processes = Vec::new();
    match job {
        ScheduledJobSpec::Index(index) => {
            let mut arguments = auth_arguments(auth_db_path);
            arguments.push("--update".into());
            if let Some(metadata_file) = index.metadata_file.as_deref() {
                arguments.push("--file".into());
                arguments.push(metadata_file.into());
            }
            processes.push(ScheduledProcess {
                executable: "index",
                arguments,
            });
            if index.notify {
                processes.push(delivery_process(
                    "notify",
                    auth_db_path,
                    &ScheduledDeliveryJob {
                        database: None,
                        max_candidates: None,
                    },
                ));
            }
            if index.push {
                processes.push(delivery_process(
                    "push",
                    auth_db_path,
                    &ScheduledDeliveryJob {
                        database: None,
                        max_candidates: None,
                    },
                ));
            }
        }
        ScheduledJobSpec::Notify(delivery) => {
            processes.push(delivery_process("notify", auth_db_path, delivery));
        }
        ScheduledJobSpec::Push(delivery) => {
            processes.push(delivery_process("push", auth_db_path, delivery));
        }
    }
    Ok(processes)
}

fn auth_arguments(auth_db_path: &Path) -> Vec<OsString> {
    vec!["--auth-db".into(), auth_db_path.as_os_str().to_owned()]
}

fn delivery_process(
    executable: &'static str,
    auth_db_path: &Path,
    job: &ScheduledDeliveryJob,
) -> ScheduledProcess {
    let mut arguments = auth_arguments(auth_db_path);
    arguments.push("--no-dry-run".into());
    if let Some(database) = job.database.as_deref() {
        arguments.push("--db".into());
        arguments.push(database.into());
    }
    if let Some(max_candidates) = job.max_candidates {
        arguments.push("--max-candidates".into());
        arguments.push(max_candidates.to_string().into());
    }
    ScheduledProcess {
        executable,
        arguments,
    }
}

fn validate_cron_field(
    field: &str,
    minimum: i64,
    maximum: i64,
    names: &[(&str, i64)],
) -> Result<(), SchedulerError> {
    for part in field.split(',') {
        validate_cron_part(part.trim(), minimum, maximum, names)?;
    }
    Ok(())
}

fn validate_cron_part(
    part: &str,
    minimum: i64,
    maximum: i64,
    names: &[(&str, i64)],
) -> Result<(), SchedulerError> {
    if part.is_empty() {
        return Err(invalid_cron("empty cron field part"));
    }
    let (base, step) = part
        .split_once('/')
        .map_or((part, None), |(base, step)| (base, Some(step)));
    if let Some(step) = step {
        let step = step
            .parse::<i64>()
            .map_err(|_| invalid_cron("cron step must be a positive integer"))?;
        if step <= 0 {
            return Err(invalid_cron("cron step must be a positive integer"));
        }
    }
    if base == "*" {
        return Ok(());
    }
    if let Some((start, end)) = base.split_once('-') {
        let start = cron_value(start, minimum, maximum, names)?;
        let end = cron_value(end, minimum, maximum, names)?;
        if start > end {
            return Err(invalid_cron(
                "cron range start must be less than or equal to end",
            ));
        }
        return Ok(());
    }
    cron_value(base, minimum, maximum, names)?;
    Ok(())
}

fn cron_value(
    value: &str,
    minimum: i64,
    maximum: i64,
    names: &[(&str, i64)],
) -> Result<i64, SchedulerError> {
    let normalized = value.trim().to_ascii_lowercase();
    let parsed = names
        .iter()
        .find_map(|(name, number)| (*name == normalized).then_some(*number))
        .map(Ok)
        .unwrap_or_else(|| {
            normalized
                .parse::<i64>()
                .map_err(|_| invalid_cron("cron field contains an invalid value"))
        })?;
    if parsed < minimum || parsed > maximum {
        return Err(invalid_cron(
            "cron field value is outside the allowed range",
        ));
    }
    Ok(parsed)
}

fn invalid_cron(message: &str) -> SchedulerError {
    SchedulerError::InvalidCron(message.to_string())
}

#[derive(Debug, Clone, Copy)]
struct CronTime {
    minute: i64,
    hour: i64,
    day: i64,
    month: i64,
    weekday: i64,
}

fn cron_matches_time(cron: &str, time: CronTime) -> Result<bool, SchedulerError> {
    let fields = cron.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(SchedulerError::InvalidCron(
            "Cron expression must contain exactly five fields".to_string(),
        ));
    }
    Ok(cron_field_matches(fields[0], time.minute, 0, 59, &[])?
        && cron_field_matches(fields[1], time.hour, 0, 23, &[])?
        && cron_field_matches(fields[2], time.day, 1, 31, &[])?
        && cron_field_matches(
            fields[3],
            time.month,
            1,
            12,
            &[
                ("jan", 1),
                ("feb", 2),
                ("mar", 3),
                ("apr", 4),
                ("may", 5),
                ("jun", 6),
                ("jul", 7),
                ("aug", 8),
                ("sep", 9),
                ("oct", 10),
                ("nov", 11),
                ("dec", 12),
            ],
        )?
        && cron_field_matches(
            fields[4],
            time.weekday,
            0,
            7,
            &[
                ("sun", 0),
                ("mon", 1),
                ("tue", 2),
                ("wed", 3),
                ("thu", 4),
                ("fri", 5),
                ("sat", 6),
            ],
        )?)
}

fn cron_field_matches(
    field: &str,
    value: i64,
    minimum: i64,
    maximum: i64,
    names: &[(&str, i64)],
) -> Result<bool, SchedulerError> {
    for part in field.split(',') {
        if cron_part_matches(part.trim(), value, minimum, maximum, names)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn cron_part_matches(
    part: &str,
    value: i64,
    minimum: i64,
    maximum: i64,
    names: &[(&str, i64)],
) -> Result<bool, SchedulerError> {
    if part.is_empty() {
        return Err(invalid_cron("empty cron field part"));
    }
    let (base, step) = part
        .split_once('/')
        .map_or((part, None), |(base, step)| (base, Some(step)));
    let step = step
        .map(|step| {
            let step = step
                .parse::<i64>()
                .map_err(|_| invalid_cron("cron step must be a positive integer"))?;
            if step <= 0 {
                return Err(invalid_cron("cron step must be a positive integer"));
            }
            Ok(step)
        })
        .transpose()?;
    let (start, end) = if base == "*" {
        (minimum, maximum)
    } else if let Some((start, end)) = base.split_once('-') {
        let start = cron_value(start, minimum, maximum, names)?;
        let end = cron_value(end, minimum, maximum, names)?;
        if start > end {
            return Err(invalid_cron(
                "cron range start must be less than or equal to end",
            ));
        }
        (start, end)
    } else {
        let parsed = cron_value(base, minimum, maximum, names)?;
        (parsed, parsed)
    };
    if !cron_value_matches_range(value, start, end, maximum) {
        return Ok(false);
    }
    Ok(step.is_none_or(|step| (value - start).rem_euclid(step) == 0))
}

fn cron_value_matches_range(value: i64, start: i64, end: i64, maximum: i64) -> bool {
    if maximum == 7 && value == 0 && start <= 7 && end >= 7 {
        return true;
    }
    value >= start && value <= end
}

fn cron_time_from_unix_seconds(seconds: i64) -> CronTime {
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (_year, month, day) = civil_from_days(days);
    CronTime {
        minute: (day_seconds % 3_600) / 60,
        hour: day_seconds / 3_600,
        day,
        month,
        weekday: (days + 4).rem_euclid(7),
    }
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

fn current_unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use ps_storage::{create_scheduled_task, get_scheduled_task, initialize_auth_database};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn scheduler_loads_enabled_jobs_and_skips_invalid_cron() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let enabled = create_index_task(&auth_db_path, "enabled", "* * * * *", true);
        create_index_task(&auth_db_path, "disabled", "* * * * *", false);
        let invalid = create_index_task(&auth_db_path, "invalid", "not cron", true);

        let result = load_scheduler_jobs(&auth_db_path).expect("jobs should load");

        assert_eq!(result.jobs.len(), 1);
        assert_eq!(result.jobs[0].id, enabled.id);
        assert_eq!(
            result.jobs[0].job_id,
            format!("scheduled-task-{}", enabled.id)
        );
        assert_eq!(result.jobs[0].job, index_job());
        assert_eq!(result.jobs[0].max_instances, 1);
        assert!(result.jobs[0].coalesce);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].id, invalid.id);
    }

    #[test]
    fn scheduler_validates_cron_field_shapes() {
        assert!(validate_cron_expression("*/5 8-18 * jan mon-fri").is_ok());
        assert!(validate_cron_expression("60 * * * *").is_err());
        assert!(validate_cron_expression("* *").is_err());
        assert!(validate_cron_expression("* * * * */0").is_err());
    }

    #[test]
    fn scheduler_run_now_records_runner_status() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_index_task(&auth_db_path, "failing", "* * * * *", true);
        let mut runner = FixtureRunner::new(["failed (7)"]);

        let outcome =
            run_task_now_with_runner(&auth_db_path, task.id, SchedulerMode::Execute, &mut runner)
                .expect("task should run");
        let updated = get_scheduled_task(&auth_db_path, task.id)
            .expect("task lookup should succeed")
            .expect("task should exist");

        assert!(outcome.found);
        assert!(outcome.did_execute);
        assert_eq!(outcome.status.as_deref(), Some("failed (7)"));
        assert_eq!(updated.last_status, "failed (7)");
        assert!(updated.last_run_at.is_some());
        assert_eq!(runner.jobs, vec![index_job()]);
    }

    #[test]
    fn scheduler_dry_run_does_not_execute_or_write_status() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_index_task(&auth_db_path, "dry-run", "* * * * *", true);
        let mut runner = FixtureRunner::new(["unexpected"]);

        let outcome =
            run_task_now_with_runner(&auth_db_path, task.id, SchedulerMode::DryRun, &mut runner)
                .expect("dry run succeeds");
        let updated = get_scheduled_task(&auth_db_path, task.id)
            .expect("task lookup should succeed")
            .expect("task should exist");

        assert!(outcome.found);
        assert!(!outcome.did_execute);
        assert_eq!(outcome.status, None);
        assert_eq!(updated.last_status, "");
        assert_eq!(updated.last_run_at, None);
        assert!(runner.jobs.is_empty());
    }

    #[test]
    fn scheduler_execute_once_runs_due_tasks_and_skips_invalid_cron() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let due = create_index_task(&auth_db_path, "due", "* * * * *", true);
        let not_due = create_index_task(&auth_db_path, "not-due", "31 10 6 7 *", true);
        let invalid = create_index_task(&auth_db_path, "invalid", "bad", true);
        let mut runner = FixtureRunner::new(["success"]);
        let mut executed_slots = BTreeSet::new();

        let result = run_due_scheduler_once_at_with_runner(
            &auth_db_path,
            unix_seconds(2026, 7, 6, 10, 30, 0) as f64,
            &mut executed_slots,
            &mut runner,
        )
        .expect("scheduler tick should run");
        let updated_due = get_scheduled_task(&auth_db_path, due.id)
            .expect("due task lookup should succeed")
            .expect("due task should exist");
        let updated_not_due = get_scheduled_task(&auth_db_path, not_due.id)
            .expect("not-due task lookup should succeed")
            .expect("not-due task should exist");
        let updated_invalid = get_scheduled_task(&auth_db_path, invalid.id)
            .expect("invalid task lookup should succeed")
            .expect("invalid task should exist");

        assert_eq!(result.jobs, 2);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].id, invalid.id);
        assert_eq!(result.due, 1);
        assert_eq!(result.already_executed, 0);
        assert_eq!(result.executed.len(), 1);
        assert_eq!(result.executed[0].task_id, due.id);
        assert_eq!(runner.jobs, vec![index_job()]);
        assert_eq!(updated_due.last_status, "success");
        assert_eq!(
            updated_due.last_run_at,
            Some(unix_seconds(2026, 7, 6, 10, 30, 0) as f64)
        );
        assert_eq!(updated_not_due.last_status, "");
        assert_eq!(updated_invalid.last_status, "");
    }

    #[test]
    fn scheduler_execute_once_does_not_duplicate_same_minute_runs() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_index_task(&auth_db_path, "due", "* * * * *", true);
        let mut runner = FixtureRunner::new(["success", "unexpected"]);
        let mut executed_slots = BTreeSet::new();
        let now = unix_seconds(2026, 7, 6, 10, 30, 0) as f64;

        let first = run_due_scheduler_once_at_with_runner(
            &auth_db_path,
            now,
            &mut executed_slots,
            &mut runner,
        )
        .expect("first tick should run");
        let second = run_due_scheduler_once_at_with_runner(
            &auth_db_path,
            now + 30.0,
            &mut executed_slots,
            &mut runner,
        )
        .expect("second tick should run");

        assert_eq!(first.executed.len(), 1);
        assert_eq!(first.executed[0].task_id, task.id);
        assert_eq!(second.due, 1);
        assert_eq!(second.already_executed, 1);
        assert!(second.executed.is_empty());
        assert_eq!(runner.jobs, vec![index_job()]);
    }

    #[test]
    fn scheduler_legacy_task_cannot_run_manually() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let connection = ps_storage::open_sqlite_connection(&auth_db_path)
            .expect("auth database should open for fixture setup");
        connection
            .execute(
                "INSERT INTO scheduled_tasks
                 (name, job_spec, legacy_command, cron, enabled, last_status, created_at, updated_at)
                 VALUES ('Legacy', NULL, 'index --update', '* * * * *', 0, '', 1.0, 1.0)",
                [],
            )
            .expect("legacy fixture should insert");
        let task_id = connection.last_insert_rowid();
        drop(connection);
        let mut runner = FixtureRunner::new(["unexpected"]);

        let outcome =
            run_task_now_with_runner(&auth_db_path, task_id, SchedulerMode::Execute, &mut runner)
                .expect("legacy task should be handled safely");

        assert!(outcome.found);
        assert!(!outcome.did_execute);
        assert_eq!(
            outcome.status.as_deref(),
            Some("blocked: legacy task requires a typed job")
        );
        assert!(runner.jobs.is_empty());
    }

    #[test]
    fn scheduler_builds_fixed_processes_and_allowlisted_arguments() {
        let auth_db_path = Path::new("data/auth.sqlite");
        let processes = scheduled_processes(
            auth_db_path,
            &ScheduledJobSpec::Index(ps_domain::ScheduledIndexJob {
                metadata_file: Some("journals.csv".to_string()),
                notify: true,
                push: true,
            }),
        )
        .expect("index process plan should build");

        assert_eq!(
            processes
                .iter()
                .map(|process| process.executable)
                .collect::<Vec<_>>(),
            vec!["index", "notify", "push"]
        );
        assert_eq!(
            process_arguments(&processes[0]),
            vec![
                "--auth-db",
                "data/auth.sqlite",
                "--update",
                "--file",
                "journals.csv",
            ]
        );
        assert_eq!(
            process_arguments(&processes[1]),
            vec!["--auth-db", "data/auth.sqlite", "--no-dry-run"]
        );

        let push = scheduled_processes(
            auth_db_path,
            &ScheduledJobSpec::Push(ScheduledDeliveryJob {
                database: Some("journals.sqlite".to_string()),
                max_candidates: Some(100),
            }),
        )
        .expect("push process plan should build");
        assert_eq!(push[0].executable, "push");
        assert_eq!(
            process_arguments(&push[0]),
            vec![
                "--auth-db",
                "data/auth.sqlite",
                "--no-dry-run",
                "--db",
                "journals.sqlite",
                "--max-candidates",
                "100",
            ]
        );
    }

    struct FixtureRunner {
        statuses: Vec<String>,
        jobs: Vec<ScheduledJobSpec>,
    }

    impl FixtureRunner {
        fn new(statuses: impl IntoIterator<Item = &'static str>) -> Self {
            let mut statuses = statuses.into_iter().map(str::to_string).collect::<Vec<_>>();
            statuses.reverse();
            Self {
                statuses,
                jobs: Vec::new(),
            }
        }
    }

    impl ScheduledJobRunner for FixtureRunner {
        fn run(&mut self, _auth_db_path: &Path, task: &ScheduledTaskInfo) -> String {
            self.jobs
                .push(task.job.clone().expect("fixture task should have a job"));
            self.statuses.pop().unwrap_or_else(|| "success".to_string())
        }
    }

    fn index_job() -> ScheduledJobSpec {
        ScheduledJobSpec::Index(ps_domain::ScheduledIndexJob {
            metadata_file: None,
            notify: false,
            push: false,
        })
    }

    fn create_index_task(
        auth_db_path: &Path,
        name: &str,
        cron: &str,
        enabled: bool,
    ) -> ScheduledTaskInfo {
        create_scheduled_task(auth_db_path, name, &index_job(), cron, enabled)
            .expect("index task should be created")
    }

    fn process_arguments(process: &ScheduledProcess) -> Vec<String> {
        process
            .arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn unix_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
        days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
    }

    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = year - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month_prime = month + if month > 2 { -3 } else { 9 };
        let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }
}
