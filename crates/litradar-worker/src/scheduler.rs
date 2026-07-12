//! Scheduler validation and typed job execution utilities.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Timelike, Utc};
use litradar_domain::{
    validate_scheduled_task_timing, ScheduledDeliveryJob, ScheduledJobSpec, ScheduledTaskInfo,
};
use litradar_storage::ScheduledRunClaim;
use serde::Serialize;

const CATCH_UP_SECONDS: f64 = 86_400.0;
const RUN_LEASE_SECONDS: f64 = 90.0;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const PROCESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const MAX_CAPTURE_BYTES: usize = 2_048;
const MAX_OUTPUT_SUMMARY_CHARS: usize = 4_096;

/// Maximum age of a healthy persisted worker heartbeat.
pub const SCHEDULER_HEALTH_WINDOW_SECONDS: f64 = 90.0;

/// Shared cancellation signal for scheduler ticks and active child processes.
#[derive(Debug, Clone, Default)]
pub struct SchedulerCancellation {
    is_cancelled: Arc<AtomicBool>,
}

impl SchedulerCancellation {
    /// Create an active scheduler cancellation handle.
    ///
    /// # Returns
    ///
    /// Cancellation handle whose initial state is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation for current and future scheduled process work.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::SeqCst);
    }

    /// Return whether cancellation has been requested.
    ///
    /// # Returns
    ///
    /// True after any clone requests cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }
}

/// Worker scheduler errors.
#[derive(Debug)]
pub enum SchedulerError {
    /// Storage repository failed.
    Storage(litradar_storage::BusinessRepositoryError),
    /// Cron expression is invalid.
    InvalidCron(String),
    /// Typed job arguments are invalid.
    InvalidJob(String),
    /// A scheduler execution thread failed unexpectedly.
    ExecutionThread,
}

impl fmt::Display for SchedulerError {
    /// Format the scheduler error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::InvalidCron(message) => formatter.write_str(message),
            Self::InvalidJob(message) => formatter.write_str(message),
            Self::ExecutionThread => formatter.write_str("Scheduler execution thread failed"),
        }
    }
}

impl Error for SchedulerError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::InvalidCron(_) | Self::InvalidJob(_) | Self::ExecutionThread => None,
        }
    }
}

impl From<litradar_storage::BusinessRepositoryError> for SchedulerError {
    /// Convert storage repository errors into scheduler errors.
    fn from(error: litradar_storage::BusinessRepositoryError) -> Self {
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
    /// Explicit IANA time zone.
    pub timezone: String,
    /// Maximum execution time in seconds.
    pub timeout_seconds: u64,
    /// Whether missed runs coalesce.
    pub coalesce: bool,
    /// APScheduler-compatible `max_instances` setting.
    pub max_instances: i64,
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
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SchedulerExecutionResult {
    /// Scheduler mode.
    pub mode: SchedulerMode,
    /// Tick status.
    pub status: String,
    /// Current Unix minute bucket.
    pub minute_epoch: i64,
    /// Beginning of the evaluated persisted interval.
    pub checked_from: f64,
    /// End of the evaluated persisted interval.
    pub checked_to: f64,
    /// Runnable job count.
    pub jobs: usize,
    /// Enabled tasks skipped because they cannot be scheduled.
    pub skipped: Vec<SkippedScheduledTask>,
    /// Due task count before duplicate suppression.
    pub due: usize,
    /// Slots already represented by durable run rows.
    pub already_executed: usize,
    /// Newly queued durable run count.
    pub queued: usize,
    /// Runs claimed by this worker.
    pub claimed: usize,
    /// Executed tasks.
    pub executed: Vec<ScheduledTaskExecution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessExecution {
    status: String,
    output_summary: String,
}

trait ScheduledJobRunner {
    fn run(
        &mut self,
        auth_db_path: &Path,
        task: &ScheduledTaskInfo,
        on_heartbeat: &mut dyn FnMut(),
    ) -> ProcessExecution;
}

struct ProcessScheduledJobRunner {
    application_executable: PathBuf,
    cancellation: SchedulerCancellation,
    secret_key_file: PathBuf,
}

impl ScheduledJobRunner for ProcessScheduledJobRunner {
    fn run(
        &mut self,
        auth_db_path: &Path,
        task: &ScheduledTaskInfo,
        on_heartbeat: &mut dyn FnMut(),
    ) -> ProcessExecution {
        task.job.as_ref().map_or_else(
            || ProcessExecution {
                status: "error".to_string(),
                output_summary: "Legacy task requires a typed job".to_string(),
            },
            |job| {
                execute_scheduled_job(
                    auth_db_path,
                    &self.application_executable,
                    &self.secret_key_file,
                    job,
                    task.timeout_seconds,
                    &self.cancellation,
                    on_heartbeat,
                )
            },
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
    for task in litradar_storage::list_scheduled_tasks(auth_db_path)? {
        if !task.enabled {
            continue;
        }
        let validation = validate_task(&task);
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
/// * `application_executable` - Canonical application executable used for the task subprocess.
/// * `secret_key_file` - Raw 32-byte deployment secret key file.
/// * `task_id` - Scheduled task row identifier.
/// * `mode` - Scheduler execution mode.
///
/// # Returns
///
/// Manual run outcome.
pub fn run_task_now(
    auth_db_path: impl AsRef<Path>,
    application_executable: impl AsRef<Path>,
    secret_key_file: impl AsRef<Path>,
    task_id: i64,
    mode: SchedulerMode,
) -> Result<RunTaskOutcome, SchedulerError> {
    let mut runner = ProcessScheduledJobRunner {
        application_executable: application_executable.as_ref().to_path_buf(),
        cancellation: SchedulerCancellation::new(),
        secret_key_file: secret_key_file.as_ref().to_path_buf(),
    };
    run_task_now_with_runner(auth_db_path.as_ref(), task_id, mode, &mut runner)
}

fn run_task_now_with_runner(
    auth_db_path: &Path,
    task_id: i64,
    mode: SchedulerMode,
    runner: &mut impl ScheduledJobRunner,
) -> Result<RunTaskOutcome, SchedulerError> {
    let Some(task) = litradar_storage::get_scheduled_task(auth_db_path, task_id)? else {
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
    if task.job.is_none() {
        return Ok(RunTaskOutcome {
            found: true,
            did_execute: false,
            status: Some("blocked: legacy task requires a typed job".to_string()),
        });
    };
    validate_task(&task)?;
    let ran_at = current_unix_time();
    let execution = runner.run(auth_db_path, &task, &mut || {});
    litradar_storage::record_scheduled_task_run(auth_db_path, task.id, &execution.status, ran_at)?;
    Ok(RunTaskOutcome {
        found: true,
        did_execute: true,
        status: Some(execution.status),
    })
}

/// Execute due scheduled tasks once for the current minute.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `application_executable` - Canonical application executable used for child processes.
/// * `secret_key_file` - Raw 32-byte deployment secret key file.
/// * `worker_id` - Stable worker process identifier.
/// * `cancellation` - Shared shutdown cancellation signal.
///
/// # Returns
///
/// Execution tick result.
pub fn run_due_scheduler_once(
    auth_db_path: impl AsRef<Path>,
    application_executable: impl AsRef<Path>,
    secret_key_file: impl AsRef<Path>,
    worker_id: &str,
    cancellation: SchedulerCancellation,
) -> Result<SchedulerExecutionResult, SchedulerError> {
    let auth_db_path = auth_db_path.as_ref().to_path_buf();
    let application_executable = application_executable.as_ref().to_path_buf();
    let secret_key_file = secret_key_file.as_ref().to_path_buf();
    let (mut result, claims) =
        prepare_scheduler_tick(&auth_db_path, worker_id, current_unix_time())?;
    let executed = thread::scope(|scope| -> Result<Vec<_>, SchedulerError> {
        let handles = claims
            .into_iter()
            .map(|claim| {
                let auth_db_path = auth_db_path.clone();
                let application_executable = application_executable.clone();
                let cancellation = cancellation.clone();
                let secret_key_file = secret_key_file.clone();
                scope.spawn(move || {
                    let mut runner = ProcessScheduledJobRunner {
                        application_executable,
                        cancellation,
                        secret_key_file,
                    };
                    execute_scheduled_claim(&auth_db_path, claim, &mut runner)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().map_err(|_| SchedulerError::ExecutionThread)?)
            .collect()
    })?;
    result.executed = executed;
    Ok(result)
}

#[cfg(test)]
fn run_due_scheduler_once_at_with_runner(
    auth_db_path: &Path,
    worker_id: &str,
    now: f64,
    runner: &mut impl ScheduledJobRunner,
) -> Result<SchedulerExecutionResult, SchedulerError> {
    let (mut result, claims) = prepare_scheduler_tick(auth_db_path, worker_id, now)?;
    for claim in claims {
        result
            .executed
            .push(execute_scheduled_claim(auth_db_path, claim, runner)?);
    }
    Ok(result)
}

fn prepare_scheduler_tick(
    auth_db_path: &Path,
    worker_id: &str,
    now: f64,
) -> Result<(SchedulerExecutionResult, Vec<ScheduledRunClaim>), SchedulerError> {
    litradar_storage::record_scheduler_heartbeat(auth_db_path, worker_id, now)?;
    let previous_check = litradar_storage::get_scheduler_last_checked_at(auth_db_path)?;
    let checked_from = previous_check
        .filter(|previous| *previous <= now)
        .unwrap_or(now - CATCH_UP_SECONDS)
        .max(now - CATCH_UP_SECONDS);
    let mut jobs = 0;
    let mut skipped = Vec::new();
    let mut due = 0;
    let mut already_executed = 0;
    let mut queued = 0;

    for task in litradar_storage::list_scheduled_tasks(auth_db_path)? {
        if !task.enabled {
            continue;
        }
        match validate_task(&task) {
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
        let task_checked_from = checked_from.max(task.created_at - 0.001);
        let slots = scheduled_slots(&task, task_checked_from, now)?;
        due += slots.len();
        let represented_slots = if task.coalesce {
            usize::from(!slots.is_empty())
        } else {
            slots.len()
        };
        let inserted = litradar_storage::enqueue_scheduled_runs(auth_db_path, &task, &slots)?;
        queued += inserted;
        already_executed += represented_slots.saturating_sub(inserted);
    }
    litradar_storage::record_scheduler_check(auth_db_path, now)?;
    let claims = litradar_storage::claim_ready_scheduled_runs(
        auth_db_path,
        worker_id,
        now,
        RUN_LEASE_SECONDS,
    )?;
    let claimed = claims.len();

    Ok((
        SchedulerExecutionResult {
            mode: SchedulerMode::Execute,
            status: "running".to_string(),
            minute_epoch: (now as i64).div_euclid(60),
            checked_from,
            checked_to: now,
            jobs,
            skipped,
            due,
            already_executed,
            queued,
            claimed,
            executed: Vec::new(),
        },
        claims,
    ))
}

fn execute_scheduled_claim(
    auth_db_path: &Path,
    claim: ScheduledRunClaim,
    runner: &mut impl ScheduledJobRunner,
) -> Result<ScheduledTaskExecution, SchedulerError> {
    let started_at = current_unix_time();
    if !litradar_storage::start_scheduled_run(
        auth_db_path,
        claim.run_id,
        &claim.worker_id,
        started_at,
        RUN_LEASE_SECONDS,
    )? {
        return Ok(ScheduledTaskExecution {
            task_id: claim.task.id,
            job_id: format!("scheduled-task-{}", claim.task.id),
            name: claim.task.name,
            status: "unknown".to_string(),
        });
    }
    let mut heartbeat_error = None;
    let mut on_heartbeat = || {
        if heartbeat_error.is_none() {
            heartbeat_error = litradar_storage::heartbeat_scheduled_run(
                auth_db_path,
                claim.run_id,
                &claim.worker_id,
                current_unix_time(),
                RUN_LEASE_SECONDS,
            )
            .err();
        }
    };
    let execution = runner.run(auth_db_path, &claim.task, &mut on_heartbeat);
    let finished_at = current_unix_time();
    litradar_storage::finish_scheduled_run(
        auth_db_path,
        &claim,
        &execution.status,
        &execution.output_summary,
        finished_at,
    )?;
    if let Some(error) = heartbeat_error {
        return Err(error.into());
    }
    Ok(ScheduledTaskExecution {
        task_id: claim.task.id,
        job_id: format!("scheduled-task-{}", claim.task.id),
        name: claim.task.name,
        status: execution.status,
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
        timezone: task.timezone.clone(),
        timeout_seconds: task.timeout_seconds,
        max_instances: 1,
        coalesce: task.coalesce,
    }
}

fn validate_job(job: Option<&ScheduledJobSpec>) -> Result<(), SchedulerError> {
    let job = job.ok_or_else(|| {
        SchedulerError::InvalidJob("Legacy task requires a typed job".to_string())
    })?;
    job.validate()
        .map_err(|error| SchedulerError::InvalidJob(error.to_string()))
}

fn validate_task(task: &ScheduledTaskInfo) -> Result<(), SchedulerError> {
    validate_cron_expression(&task.cron)?;
    validate_job(task.job.as_ref())?;
    validate_scheduled_task_timing(&task.timezone, task.timeout_seconds)
        .map_err(|error| SchedulerError::InvalidJob(error.to_string()))
}

fn scheduled_slots(
    task: &ScheduledTaskInfo,
    checked_from: f64,
    checked_to: f64,
) -> Result<Vec<i64>, SchedulerError> {
    if checked_from >= checked_to {
        return Ok(Vec::new());
    }
    let timezone = task.timezone.parse::<chrono_tz::Tz>().map_err(|_| {
        SchedulerError::InvalidJob("timezone must be a valid IANA name".to_string())
    })?;
    let first_minute = (checked_from.floor() as i64).div_euclid(60) + 1;
    let last_minute = (checked_to.floor() as i64).div_euclid(60);
    let mut slots = Vec::new();
    for minute_epoch in first_minute..=last_minute {
        let scheduled_for = minute_epoch * 60;
        let Some(utc) = DateTime::<Utc>::from_timestamp(scheduled_for, 0) else {
            continue;
        };
        let local = utc.with_timezone(&timezone);
        let time = CronTime {
            minute: i64::from(local.minute()),
            hour: i64::from(local.hour()),
            day: i64::from(local.day()),
            month: i64::from(local.month()),
            weekday: i64::from(local.weekday().num_days_from_sunday()),
        };
        if cron_matches_time(&task.cron, time)? {
            slots.push(scheduled_for);
        }
    }
    Ok(slots)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduledProcess {
    executable: OsString,
    arguments: Vec<OsString>,
}

fn execute_scheduled_job(
    auth_db_path: &Path,
    application_executable: &Path,
    secret_key_file: &Path,
    job: &ScheduledJobSpec,
    timeout_seconds: u64,
    cancellation: &SchedulerCancellation,
    on_heartbeat: &mut dyn FnMut(),
) -> ProcessExecution {
    let processes =
        match scheduled_processes(auth_db_path, application_executable, secret_key_file, job) {
            Ok(processes) => processes,
            Err(error) => {
                return ProcessExecution {
                    status: "error".to_string(),
                    output_summary: error.to_string(),
                };
            }
        };
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let mut summaries = Vec::new();
    for process in processes {
        if cancellation.is_cancelled() {
            return ProcessExecution {
                status: "cancelled".to_string(),
                output_summary: bounded_output_summary(&summaries.join("\n")),
            };
        }
        let execution = execute_scheduled_process(process, deadline, cancellation, on_heartbeat);
        if !execution.output_summary.is_empty() {
            summaries.push(execution.output_summary);
        }
        if execution.status != "success" {
            return ProcessExecution {
                status: execution.status,
                output_summary: bounded_output_summary(&summaries.join("\n")),
            };
        }
    }
    ProcessExecution {
        status: "success".to_string(),
        output_summary: bounded_output_summary(&summaries.join("\n")),
    }
}

fn execute_scheduled_process(
    process: ScheduledProcess,
    deadline: Instant,
    cancellation: &SchedulerCancellation,
    on_heartbeat: &mut dyn FnMut(),
) -> ProcessExecution {
    let executable = process.executable.to_string_lossy().into_owned();
    if cancellation.is_cancelled() {
        return ProcessExecution {
            status: "cancelled".to_string(),
            output_summary: format!("{executable}: cancelled before start"),
        };
    }
    let mut command = Command::new(&process.executable);
    command
        .args(process.arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ProcessExecution {
                status: "error".to_string(),
                output_summary: format!("{executable}: {error}"),
            };
        }
    };
    let stdout_reader = child.stdout.take().map(spawn_bounded_reader);
    let stderr_reader = child.stderr.take().map(spawn_bounded_reader);
    let mut last_heartbeat = Instant::now();
    let (status, detail) = loop {
        if cancellation.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            break ("cancelled", format!("{executable}: cancelled"));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break ("success", String::new()),
            Ok(Some(status)) => {
                let detail = status.code().map_or_else(
                    || format!("{executable}: process failed"),
                    |code| format!("{executable}: exit code {code}"),
                );
                break ("failed", detail);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break ("timed_out", format!("{executable}: timed out"));
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break ("error", format!("{executable}: {error}"));
            }
        }
        if last_heartbeat.elapsed() >= PROCESS_HEARTBEAT_INTERVAL {
            on_heartbeat();
            last_heartbeat = Instant::now();
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    };
    let stdout = join_bounded_reader(stdout_reader);
    let stderr = join_bounded_reader(stderr_reader);
    let mut summary = detail;
    append_captured_output(&mut summary, "stdout", &stdout);
    append_captured_output(&mut summary, "stderr", &stderr);
    ProcessExecution {
        status: status.to_string(),
        output_summary: bounded_output_summary(&summary),
    }
}

fn spawn_bounded_reader(mut reader: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buffer = [0_u8; 1_024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let remaining = MAX_CAPTURE_BYTES.saturating_sub(captured.len());
                    captured.extend_from_slice(&buffer[..count.min(remaining)]);
                }
            }
        }
        captured
    })
}

fn join_bounded_reader(reader: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn append_captured_output(summary: &mut String, label: &str, output: &[u8]) {
    if output.is_empty() {
        return;
    }
    if !summary.is_empty() {
        summary.push('\n');
    }
    summary.push_str(label);
    summary.push_str(": ");
    summary.push_str(&String::from_utf8_lossy(output));
}

fn bounded_output_summary(summary: &str) -> String {
    summary.chars().take(MAX_OUTPUT_SUMMARY_CHARS).collect()
}

fn scheduled_processes(
    auth_db_path: &Path,
    application_executable: &Path,
    secret_key_file: &Path,
    job: &ScheduledJobSpec,
) -> Result<Vec<ScheduledProcess>, SchedulerError> {
    validate_job(Some(job))?;
    let mut processes = Vec::new();
    match job {
        ScheduledJobSpec::Index(index) => {
            let mut arguments = vec![OsString::from("index")];
            arguments.extend(auth_arguments(auth_db_path, secret_key_file));
            arguments.push("--update".into());
            if let Some(metadata_file) = index.metadata_file.as_deref() {
                arguments.push("--file".into());
                arguments.push(metadata_file.into());
            }
            processes.push(ScheduledProcess {
                executable: application_executable.as_os_str().to_owned(),
                arguments,
            });
            if index.notify {
                processes.push(delivery_process(
                    "notify",
                    auth_db_path,
                    application_executable,
                    secret_key_file,
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
                    application_executable,
                    secret_key_file,
                    &ScheduledDeliveryJob {
                        database: None,
                        max_candidates: None,
                    },
                ));
            }
        }
        ScheduledJobSpec::Notify(delivery) => {
            processes.push(delivery_process(
                "notify",
                auth_db_path,
                application_executable,
                secret_key_file,
                delivery,
            ));
        }
        ScheduledJobSpec::Push(delivery) => {
            processes.push(delivery_process(
                "push",
                auth_db_path,
                application_executable,
                secret_key_file,
                delivery,
            ));
        }
    }
    Ok(processes)
}

fn auth_arguments(auth_db_path: &Path, secret_key_file: &Path) -> Vec<OsString> {
    vec![
        "--auth-db".into(),
        auth_db_path.as_os_str().to_owned(),
        "--secret-key-file".into(),
        secret_key_file.as_os_str().to_owned(),
    ]
}

fn delivery_process(
    subcommand: &'static str,
    auth_db_path: &Path,
    application_executable: &Path,
    secret_key_file: &Path,
    job: &ScheduledDeliveryJob,
) -> ScheduledProcess {
    let mut arguments = vec![OsString::from(subcommand)];
    arguments.extend(auth_arguments(auth_db_path, secret_key_file));
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
        executable: application_executable.as_os_str().to_owned(),
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

fn current_unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

/// Generate one stable identifier for a worker process lifetime.
///
/// # Returns
///
/// Process-scoped identifier suitable for persisted claims and heartbeats.
pub fn scheduler_worker_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    format!("worker-{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    use litradar_storage::{
        create_scheduled_task, get_scheduled_task, initialize_auth_database,
        ScheduledTaskCreateParams,
    };
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
        set_task_created_at(&auth_db_path, unix_seconds(2026, 7, 6, 9, 0, 0) as f64);
        litradar_storage::record_scheduler_check(
            &auth_db_path,
            unix_seconds(2026, 7, 6, 10, 29, 0) as f64,
        )
        .expect("scheduler cursor should be set");

        let result = run_due_scheduler_once_at_with_runner(
            &auth_db_path,
            "worker-test",
            unix_seconds(2026, 7, 6, 10, 30, 0) as f64,
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
        assert!(updated_due.last_run_at.is_some());
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
        let now = unix_seconds(2026, 7, 6, 10, 30, 0) as f64;
        set_task_created_at(&auth_db_path, unix_seconds(2026, 7, 6, 9, 0, 0) as f64);
        litradar_storage::record_scheduler_check(&auth_db_path, now - 60.0)
            .expect("scheduler cursor should be set");

        let first =
            run_due_scheduler_once_at_with_runner(&auth_db_path, "worker-test", now, &mut runner)
                .expect("first tick should run");
        let second = run_due_scheduler_once_at_with_runner(
            &auth_db_path,
            "worker-test",
            now + 30.0,
            &mut runner,
        )
        .expect("second tick should run");

        assert_eq!(first.executed.len(), 1);
        assert_eq!(first.executed[0].task_id, task.id);
        assert_eq!(second.due, 0);
        assert_eq!(second.already_executed, 0);
        assert!(second.executed.is_empty());
        assert_eq!(runner.jobs, vec![index_job()]);
    }

    #[test]
    fn scheduler_continues_after_one_task_times_out() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let timed_out = create_index_task(&auth_db_path, "timed-out", "* * * * *", true);
        let successful = create_index_task(&auth_db_path, "successful", "* * * * *", true);
        let now = unix_seconds(2026, 7, 6, 10, 30, 0) as f64;
        set_task_created_at(&auth_db_path, now - 3_600.0);
        litradar_storage::record_scheduler_check(&auth_db_path, now - 60.0)
            .expect("scheduler cursor should be set");
        let mut runner = FixtureRunner::new(["timed_out", "success"]);

        let result =
            run_due_scheduler_once_at_with_runner(&auth_db_path, "worker-test", now, &mut runner)
                .expect("scheduler tick should isolate task outcomes");

        assert_eq!(result.executed.len(), 2);
        assert_eq!(result.executed[0].task_id, timed_out.id);
        assert_eq!(result.executed[0].status, "timed_out");
        assert_eq!(result.executed[1].task_id, successful.id);
        assert_eq!(result.executed[1].status, "success");
    }

    #[test]
    fn scheduler_persists_cancelled_claim_status() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_index_task(&auth_db_path, "cancelled", "* * * * *", true);
        let now = unix_seconds(2026, 7, 6, 10, 30, 0) as f64;
        set_task_created_at(&auth_db_path, now - 3_600.0);
        litradar_storage::record_scheduler_check(&auth_db_path, now - 60.0)
            .expect("scheduler cursor should be set");
        let mut runner = FixtureRunner::new(["cancelled"]);

        let result =
            run_due_scheduler_once_at_with_runner(&auth_db_path, "worker-test", now, &mut runner)
                .expect("cancelled task status should persist");
        let updated = get_scheduled_task(&auth_db_path, task.id)
            .expect("task lookup should succeed")
            .expect("task should exist");

        assert_eq!(result.executed[0].status, "cancelled");
        assert_eq!(updated.last_status, "cancelled");
        assert!(updated.last_run_at.is_some());
    }

    #[test]
    fn concurrent_fixture_workers_execute_one_side_effect_per_slot() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        create_index_task(&auth_db_path, "contended", "* * * * *", true);
        let now = unix_seconds(2026, 7, 6, 10, 30, 0) as f64;
        set_task_created_at(&auth_db_path, now - 3_600.0);
        litradar_storage::record_scheduler_check(&auth_db_path, now - 60.0)
            .expect("scheduler cursor should be set");
        let barrier = Arc::new(Barrier::new(3));
        let side_effects = Arc::new(AtomicUsize::new(0));

        let results = thread::scope(|scope| {
            let handles = (1..=2)
                .map(|worker_number| {
                    let auth_db_path = auth_db_path.clone();
                    let barrier = Arc::clone(&barrier);
                    let side_effects = Arc::clone(&side_effects);
                    scope.spawn(move || {
                        let mut runner = CountingRunner { side_effects };
                        barrier.wait();
                        run_due_scheduler_once_at_with_runner(
                            &auth_db_path,
                            &format!("worker-{worker_number}"),
                            now,
                            &mut runner,
                        )
                    })
                })
                .collect::<Vec<_>>();
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("fixture worker should not panic")
                        .expect("fixture worker should complete")
                })
                .collect::<Vec<_>>()
        });
        let status = litradar_storage::get_scheduler_status(&auth_db_path, now, 90.0, 10)
            .expect("scheduler status should load");

        assert_eq!(
            results
                .iter()
                .map(|result| result.executed.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(side_effects.load(Ordering::SeqCst), 1);
        assert_eq!(status.recent_runs.len(), 1);
        assert_eq!(status.recent_runs[0].status, "success");
    }

    #[test]
    fn scheduler_phase_offset_catches_the_latest_missed_slot() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let task = create_index_task(&auth_db_path, "phase-offset", "0 10 * * *", true);
        set_task_created_at(&auth_db_path, unix_seconds(2026, 7, 6, 9, 0, 0) as f64);
        litradar_storage::record_scheduler_check(
            &auth_db_path,
            unix_seconds(2026, 7, 6, 9, 59, 30) as f64,
        )
        .expect("scheduler cursor should be set");
        let mut runner = FixtureRunner::new(["success"]);

        let result = run_due_scheduler_once_at_with_runner(
            &auth_db_path,
            "worker-phase",
            unix_seconds(2026, 7, 6, 10, 2, 0) as f64,
            &mut runner,
        )
        .expect("phase-offset scheduler tick should run");

        assert_eq!(result.due, 1);
        assert_eq!(result.queued, 1);
        assert_eq!(result.claimed, 1);
        assert_eq!(result.executed[0].task_id, task.id);
        assert_eq!(result.executed[0].status, "success");
    }

    #[test]
    fn scheduler_timezone_handles_dst_gaps_and_repeated_minutes() {
        let mut task = scheduled_task_fixture("30 1 * * *", "Europe/London");
        task.coalesce = false;

        let spring_slots = scheduled_slots(
            &task,
            unix_seconds(2016, 3, 26, 23, 59, 0) as f64,
            unix_seconds(2016, 3, 27, 3, 0, 0) as f64,
        )
        .expect("spring DST slots should evaluate");
        let autumn_slots = scheduled_slots(
            &task,
            unix_seconds(2016, 10, 29, 23, 59, 0) as f64,
            unix_seconds(2016, 10, 30, 2, 0, 0) as f64,
        )
        .expect("autumn DST slots should evaluate");

        assert!(spring_slots.is_empty());
        assert_eq!(
            autumn_slots,
            vec![
                unix_seconds(2016, 10, 30, 0, 30, 0),
                unix_seconds(2016, 10, 30, 1, 30, 0),
            ]
        );
    }

    #[test]
    fn scheduler_process_timeout_terminates_child_and_bounds_output() {
        let executable = std::env::current_exe().expect("test executable should resolve");
        let process = ScheduledProcess {
            executable: executable.into_os_string(),
            arguments: vec![
                "--ignored".into(),
                "--exact".into(),
                "scheduler::tests::scheduler_timeout_child_fixture".into(),
                "--nocapture".into(),
            ],
        };
        let started = Instant::now();

        let result = execute_scheduled_process(
            process,
            Instant::now() + Duration::from_millis(150),
            &SchedulerCancellation::new(),
            &mut || {},
        );

        assert_eq!(result.status, "timed_out");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            bounded_output_summary(&"x".repeat(MAX_OUTPUT_SUMMARY_CHARS * 2))
                .chars()
                .count(),
            MAX_OUTPUT_SUMMARY_CHARS
        );
    }

    #[test]
    fn scheduler_cancellation_terminates_and_waits_for_active_child() {
        let executable = std::env::current_exe().expect("test executable should resolve");
        let process = ScheduledProcess {
            executable: executable.into_os_string(),
            arguments: vec![
                "--ignored".into(),
                "--exact".into(),
                "scheduler::tests::scheduler_timeout_child_fixture".into(),
                "--nocapture".into(),
            ],
        };
        let cancellation = SchedulerCancellation::new();
        let cancellation_request = cancellation.clone();
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            cancellation_request.cancel();
        });
        let started = Instant::now();

        let result = execute_scheduled_process(
            process,
            Instant::now() + Duration::from_secs(5),
            &cancellation,
            &mut || {},
        );
        cancel_thread
            .join()
            .expect("cancellation request should complete");

        assert_eq!(result.status, "cancelled");
        assert!(result.output_summary.contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    #[ignore = "helper process for scheduler timeout coverage"]
    fn scheduler_timeout_child_fixture() {
        thread::sleep(Duration::from_secs(5));
    }

    #[test]
    fn scheduler_legacy_task_cannot_run_manually() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let connection = litradar_storage::open_sqlite_connection(&auth_db_path)
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
    fn scheduler_builds_same_application_subcommands_and_allowlisted_arguments() {
        let auth_db_path = Path::new("data/auth.sqlite");
        let processes = scheduled_processes(
            auth_db_path,
            Path::new("/app/litradar"),
            Path::new("secret.key"),
            &ScheduledJobSpec::Index(litradar_domain::ScheduledIndexJob {
                metadata_file: Some("journals.csv".to_string()),
                notify: true,
                push: true,
            }),
        )
        .expect("index process plan should build");

        assert_eq!(
            processes
                .iter()
                .map(|process| process.executable.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["/app/litradar", "/app/litradar", "/app/litradar"]
        );
        assert_eq!(
            process_arguments(&processes[0]),
            vec![
                "index",
                "--auth-db",
                "data/auth.sqlite",
                "--secret-key-file",
                "secret.key",
                "--update",
                "--file",
                "journals.csv",
            ]
        );
        assert_eq!(
            process_arguments(&processes[1]),
            vec![
                "notify",
                "--auth-db",
                "data/auth.sqlite",
                "--secret-key-file",
                "secret.key",
                "--no-dry-run",
            ]
        );

        let push = scheduled_processes(
            auth_db_path,
            Path::new("/app/litradar"),
            Path::new("secret.key"),
            &ScheduledJobSpec::Push(ScheduledDeliveryJob {
                database: Some("journals.sqlite".to_string()),
                max_candidates: Some(100),
            }),
        )
        .expect("push process plan should build");
        assert_eq!(push[0].executable.to_string_lossy(), "/app/litradar");
        assert_eq!(
            process_arguments(&push[0]),
            vec![
                "push",
                "--auth-db",
                "data/auth.sqlite",
                "--secret-key-file",
                "secret.key",
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
        fn run(
            &mut self,
            _auth_db_path: &Path,
            task: &ScheduledTaskInfo,
            on_heartbeat: &mut dyn FnMut(),
        ) -> ProcessExecution {
            self.jobs
                .push(task.job.clone().expect("fixture task should have a job"));
            on_heartbeat();
            ProcessExecution {
                status: self.statuses.pop().unwrap_or_else(|| "success".to_string()),
                output_summary: "fixture output".to_string(),
            }
        }
    }

    struct CountingRunner {
        side_effects: Arc<AtomicUsize>,
    }

    impl ScheduledJobRunner for CountingRunner {
        fn run(
            &mut self,
            _auth_db_path: &Path,
            _task: &ScheduledTaskInfo,
            on_heartbeat: &mut dyn FnMut(),
        ) -> ProcessExecution {
            self.side_effects.fetch_add(1, Ordering::SeqCst);
            on_heartbeat();
            ProcessExecution {
                status: "success".to_string(),
                output_summary: "fixture output".to_string(),
            }
        }
    }

    fn index_job() -> ScheduledJobSpec {
        ScheduledJobSpec::Index(litradar_domain::ScheduledIndexJob {
            metadata_file: None,
            notify: false,
            push: false,
        })
    }

    fn scheduled_task_fixture(cron: &str, timezone: &str) -> ScheduledTaskInfo {
        ScheduledTaskInfo {
            id: 1,
            name: "fixture".to_string(),
            job: Some(index_job()),
            legacy_command: None,
            cron: cron.to_string(),
            timezone: timezone.to_string(),
            timeout_seconds: 60,
            coalesce: true,
            enabled: true,
            last_run_at: None,
            last_status: String::new(),
            created_at: 0.0,
            updated_at: 0.0,
        }
    }

    fn create_index_task(
        auth_db_path: &Path,
        name: &str,
        cron: &str,
        enabled: bool,
    ) -> ScheduledTaskInfo {
        create_scheduled_task(
            auth_db_path,
            ScheduledTaskCreateParams {
                name,
                job: &index_job(),
                cron,
                timezone: "UTC",
                timeout_seconds: 60,
                coalesce: true,
                enabled,
            },
        )
        .expect("index task should be created")
    }

    fn set_task_created_at(auth_db_path: &Path, created_at: f64) {
        let connection = litradar_storage::open_sqlite_connection(auth_db_path)
            .expect("auth database should open for fixture setup");
        connection
            .execute(
                "UPDATE scheduled_tasks SET created_at = ?1, updated_at = ?1",
                [created_at],
            )
            .expect("task timestamps should update");
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
