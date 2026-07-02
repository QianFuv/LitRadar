//! Scheduler and shell job execution utilities.

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::ScheduledTaskInfo;
use serde::Serialize;

/// Worker scheduler errors.
#[derive(Debug)]
pub enum SchedulerError {
    /// Storage repository failed.
    Storage(ps_storage::BusinessRepositoryError),
    /// Cron expression is invalid.
    InvalidCron(String),
}

impl fmt::Display for SchedulerError {
    /// Format the scheduler error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::InvalidCron(message) => formatter.write_str(message),
        }
    }
}

impl Error for SchedulerError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::InvalidCron(_) => None,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    /// Load scheduled jobs without executing commands.
    DryRun,
    /// Shadow load scheduled jobs without executing commands.
    Shadow,
    /// Execute commands and write back status.
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
    /// Shell command.
    pub command: String,
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
    /// Whether a shell command was executed.
    pub did_execute: bool,
    /// Stored status when a command was executed.
    pub status: Option<String>,
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

/// Load enabled scheduled jobs without executing commands.
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
        match validate_cron_expression(&task.cron) {
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
    let auth_db_path = auth_db_path.as_ref();
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
    let ran_at = current_unix_time();
    let status = execute_shell_command(auth_db_path, &task.command);
    ps_storage::record_scheduled_task_run(auth_db_path, task.id, &status, ran_at)?;
    Ok(RunTaskOutcome {
        found: true,
        did_execute: true,
        status: Some(status),
    })
}

fn scheduled_job(task: &ScheduledTaskInfo) -> ScheduledJob {
    ScheduledJob {
        id: task.id,
        job_id: format!("scheduled-task-{}", task.id),
        name: task.name.clone(),
        command: task.command.clone(),
        cron: task.cron.clone(),
        max_instances: 1,
        coalesce: true,
    }
}

fn execute_shell_command(auth_db_path: &Path, command: &str) -> String {
    let mut shell = shell_command(command);
    if let Ok(settings) = ps_storage::list_runtime_settings(auth_db_path) {
        for setting in settings {
            if setting.source != "database" {
                continue;
            }
            if setting.value.trim().is_empty() {
                shell.env_remove(setting.key);
            } else {
                shell.env(setting.key, setting.value);
            }
        }
    }
    match shell.output() {
        Ok(output) if output.status.success() => "success".to_string(),
        Ok(output) => match output.status.code() {
            Some(code) => format!("failed ({code})"),
            None => "failed".to_string(),
        },
        Err(error) => format!("error: {error}"),
    }
}

fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut shell = Command::new("cmd");
        shell.args(["/C", command]);
        shell
    } else {
        let mut shell = Command::new("sh");
        shell.args(["-c", command]);
        shell
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

fn current_unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use ps_storage::{create_scheduled_task, get_scheduled_task, initialize_auth_database};
    use tempfile::tempdir;

    use super::{load_scheduler_jobs, run_task_now, validate_cron_expression, SchedulerMode};

    #[test]
    fn loads_enabled_jobs_and_skips_invalid_cron() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let enabled = create_scheduled_task(&auth_db_path, "enabled", "echo ok", "* * * * *", true)
            .expect("enabled task should be created");
        create_scheduled_task(&auth_db_path, "disabled", "echo no", "* * * * *", false)
            .expect("disabled task should be created");
        let invalid = create_scheduled_task(&auth_db_path, "invalid", "echo bad", "not cron", true)
            .expect("invalid task should be created");

        let result = load_scheduler_jobs(&auth_db_path).expect("jobs should load");

        assert_eq!(result.jobs.len(), 1);
        assert_eq!(result.jobs[0].id, enabled.id);
        assert_eq!(
            result.jobs[0].job_id,
            format!("scheduled-task-{}", enabled.id)
        );
        assert_eq!(result.jobs[0].max_instances, 1);
        assert!(result.jobs[0].coalesce);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].id, invalid.id);
    }

    #[test]
    fn validates_cron_field_shapes() {
        assert!(validate_cron_expression("*/5 8-18 * jan mon-fri").is_ok());
        assert!(validate_cron_expression("60 * * * *").is_err());
        assert!(validate_cron_expression("* *").is_err());
        assert!(validate_cron_expression("* * * * */0").is_err());
    }

    #[test]
    fn run_now_writes_python_compatible_failure_status() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_scheduled_task(
            &auth_db_path,
            "failing",
            failing_command(),
            "* * * * *",
            true,
        )
        .expect("task should be created");

        let outcome =
            run_task_now(&auth_db_path, task.id, SchedulerMode::Execute).expect("task should run");
        let updated = get_scheduled_task(&auth_db_path, task.id)
            .expect("task lookup should succeed")
            .expect("task should exist");

        assert!(outcome.found);
        assert!(outcome.did_execute);
        assert_eq!(outcome.status.as_deref(), Some("failed (7)"));
        assert_eq!(updated.last_status, "failed (7)");
        assert!(updated.last_run_at.is_some());
    }

    #[test]
    fn dry_run_does_not_execute_or_write_status() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_scheduled_task(
            &auth_db_path,
            "dry-run",
            failing_command(),
            "* * * * *",
            true,
        )
        .expect("task should be created");

        let outcome =
            run_task_now(&auth_db_path, task.id, SchedulerMode::DryRun).expect("dry run succeeds");
        let updated = get_scheduled_task(&auth_db_path, task.id)
            .expect("task lookup should succeed")
            .expect("task should exist");

        assert!(outcome.found);
        assert!(!outcome.did_execute);
        assert_eq!(outcome.status, None);
        assert_eq!(updated.last_status, "");
        assert_eq!(updated.last_run_at, None);
    }

    fn failing_command() -> &'static str {
        if cfg!(windows) {
            "exit /B 7"
        } else {
            "exit 7"
        }
    }
}
