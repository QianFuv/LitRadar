//! Scheduled task definitions, claims, runs, and worker state.

use super::shared::*;
use super::*;

/// Borrowed values used to create one scheduled task.
#[derive(Debug, Clone, Copy)]
pub struct ScheduledTaskCreateParams<'a> {
    /// Task name.
    pub name: &'a str,
    /// Validated job specification.
    pub job: &'a ScheduledJobSpec,
    /// Five-field cron expression.
    pub cron: &'a str,
    /// IANA time zone used for cron evaluation.
    pub timezone: &'a str,
    /// Maximum task runtime.
    pub timeout_seconds: u64,
    /// Whether missed slots collapse to the latest slot.
    pub coalesce: bool,
    /// Whether the task is enabled.
    pub enabled: bool,
}

/// Borrowed optional values used to update one scheduled task.
#[derive(Debug, Clone, Copy)]
pub struct ScheduledTaskUpdateParams<'a> {
    /// Scheduled task row identifier.
    pub task_id: i64,
    /// Optional replacement task name.
    pub name: Option<&'a str>,
    /// Optional replacement job specification.
    pub job: Option<&'a ScheduledJobSpec>,
    /// Optional replacement cron expression.
    pub cron: Option<&'a str>,
    /// Optional replacement IANA time zone.
    pub timezone: Option<&'a str>,
    /// Optional replacement timeout.
    pub timeout_seconds: Option<u64>,
    /// Optional replacement coalescing flag.
    pub coalesce: Option<bool>,
    /// Optional replacement enabled flag.
    pub enabled: Option<bool>,
}

/// Durable scheduled run claimed for one worker execution.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledRunClaim {
    /// Run row identifier.
    pub run_id: i64,
    /// Scheduled UTC Unix timestamp.
    pub scheduled_for: i64,
    /// Claiming worker identifier.
    pub worker_id: String,
    /// Current task definition used for execution.
    pub task: ScheduledTaskInfo,
}

/// List scheduled tasks.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Scheduled tasks ordered by creation time descending.
pub fn list_scheduled_tasks(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT id, name, job_spec, legacy_command, cron, timezone, timeout_seconds, coalesce, \
                enabled, last_run_at, last_status, created_at, updated_at \
         FROM scheduled_tasks ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map([], scheduled_task_from_row)?;
    collect_rows(rows)
}

fn validate_scheduled_job(job: &ScheduledJobSpec) -> Result<(), BusinessRepositoryError> {
    job.validate()
        .map_err(|error| BusinessRepositoryError::InvalidScheduledJob(error.to_string()))
}

fn validate_scheduled_timing(
    timezone: &str,
    timeout_seconds: u64,
) -> Result<(), BusinessRepositoryError> {
    validate_scheduled_task_timing(timezone, timeout_seconds)
        .map_err(|error| BusinessRepositoryError::InvalidScheduledTask(error.to_string()))
}

/// Get one scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
///
/// # Returns
///
/// Scheduled task payload when it exists.
pub fn get_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    get_scheduled_task_from_connection(&connection, task_id)
}

/// Create a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task` - Validated task creation values.
///
/// # Returns
///
/// Created task payload.
pub fn create_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task: ScheduledTaskCreateParams<'_>,
) -> Result<ScheduledTaskInfo, BusinessRepositoryError> {
    validate_scheduled_job(task.job)?;
    validate_scheduled_timing(task.timezone, task.timeout_seconds)?;
    let connection = open_business_connection(auth_db_path)?;
    let now = now_seconds();
    let job_spec = serde_json::to_string(task.job)?;
    connection.execute(
        "INSERT INTO scheduled_tasks \
         (name, job_spec, legacy_command, cron, timezone, timeout_seconds, coalesce, enabled, \
          last_run_at, last_status, created_at, updated_at) \
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, NULL, '', ?8, ?9)",
        params![
            task.name,
            job_spec,
            task.cron,
            task.timezone,
            task.timeout_seconds,
            task.coalesce as i64,
            task.enabled as i64,
            now,
            now
        ],
    )?;
    get_scheduled_task_from_connection(&connection, connection.last_insert_rowid())?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

/// Update a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task` - Scheduled task identifier and optional replacement values.
///
/// # Returns
///
/// Updated task payload or None.
pub fn update_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task: ScheduledTaskUpdateParams<'_>,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let Some(current) = get_scheduled_task_from_connection(&connection, task.task_id)? else {
        return Ok(None);
    };
    let next_job = task.job.or(current.job.as_ref());
    if let Some(next_job) = next_job {
        validate_scheduled_job(next_job)?;
    }
    let next_timezone = task.timezone.unwrap_or(&current.timezone);
    let next_timeout_seconds = task.timeout_seconds.unwrap_or(current.timeout_seconds);
    validate_scheduled_timing(next_timezone, next_timeout_seconds)?;
    let next_enabled = task.enabled.unwrap_or(current.enabled);
    if next_job.is_none() && next_enabled {
        return Err(BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled);
    }
    let job_spec = next_job.map(serde_json::to_string).transpose()?;
    let legacy_command = if task.job.is_some() {
        None
    } else {
        current.legacy_command.as_deref()
    };
    connection.execute(
        "UPDATE scheduled_tasks SET name = ?1, job_spec = ?2, legacy_command = ?3, cron = ?4, \
         timezone = ?5, timeout_seconds = ?6, coalesce = ?7, enabled = ?8, \
         updated_at = ?9 WHERE id = ?10",
        params![
            task.name.unwrap_or(&current.name),
            job_spec,
            legacy_command,
            task.cron.unwrap_or(&current.cron),
            next_timezone,
            next_timeout_seconds,
            task.coalesce.unwrap_or(current.coalesce) as i64,
            next_enabled as i64,
            now_seconds(),
            task.task_id
        ],
    )?;
    get_scheduled_task_from_connection(&connection, task.task_id)
}

/// Delete a scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_scheduled_task(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute("DELETE FROM scheduled_tasks WHERE id = ?1", [task_id])?;
    Ok(count > 0)
}

/// Record one scheduled task run result.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task_id` - Scheduled task row identifier.
/// * `status` - Python-compatible status string.
/// * `ran_at` - Unix timestamp when the job started.
///
/// # Returns
///
/// True when a task row was updated.
pub fn record_scheduled_task_run(
    auth_db_path: impl AsRef<Path>,
    task_id: i64,
    status: &str,
    ran_at: f64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "UPDATE scheduled_tasks SET last_run_at = ?1, last_status = ?2, \
         updated_at = ?3 WHERE id = ?4",
        rusqlite::params![ran_at, status, now_seconds(), task_id],
    )?;
    Ok(count > 0)
}

/// Read the persisted scheduler cursor.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Last completed scheduler check, or None before the first check.
pub fn get_scheduler_last_checked_at(
    auth_db_path: impl AsRef<Path>,
) -> Result<Option<f64>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT last_checked_at FROM scheduler_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(BusinessRepositoryError::from)
}

/// Advance the scheduler cursor without allowing time to move backward.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `checked_at` - Completed wall-clock check timestamp.
///
/// # Returns
///
/// Empty result after the cursor is persisted.
pub fn record_scheduler_check(
    auth_db_path: impl AsRef<Path>,
    checked_at: f64,
) -> Result<(), BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection.execute(
        "UPDATE scheduler_state
         SET last_checked_at = CASE
             WHEN last_checked_at IS NULL OR last_checked_at < ?1 THEN ?1
             ELSE last_checked_at
         END
         WHERE id = 1",
        [checked_at],
    )?;
    Ok(())
}

/// Persist one scheduler worker heartbeat.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `worker_id` - Stable worker process identifier.
/// * `heartbeat_at` - Current Unix timestamp.
///
/// # Returns
///
/// Empty result after the heartbeat is persisted.
pub fn record_scheduler_heartbeat(
    auth_db_path: impl AsRef<Path>,
    worker_id: &str,
    heartbeat_at: f64,
) -> Result<(), BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection.execute(
        "INSERT INTO scheduler_workers (worker_id, started_at, heartbeat_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(worker_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at, heartbeat_at],
    )?;
    connection.execute(
        "INSERT INTO service_heartbeats (service, instance_id, started_at, heartbeat_at)
         VALUES ('worker', ?1, ?2, ?2)
         ON CONFLICT(service, instance_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at],
    )?;
    connection.execute(
        "DELETE FROM scheduler_workers WHERE worker_id <> ?1 AND heartbeat_at < ?2",
        params![worker_id, heartbeat_at - 604_800.0],
    )?;
    Ok(())
}

/// Queue durable scheduled slots for one task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `task` - Current scheduled task definition.
/// * `scheduled_slots` - UTC Unix minute timestamps.
///
/// # Returns
///
/// Number of newly inserted run rows.
pub fn enqueue_scheduled_runs(
    auth_db_path: impl AsRef<Path>,
    task: &ScheduledTaskInfo,
    scheduled_slots: &[i64],
) -> Result<usize, BusinessRepositoryError> {
    if scheduled_slots.is_empty() {
        return Ok(0);
    }
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if task.coalesce {
        let latest_slot = scheduled_slots[scheduled_slots.len() - 1];
        let latest_pending = transaction.query_row(
            "SELECT MAX(scheduled_for) FROM scheduled_task_runs
             WHERE task_id = ?1 AND status = 'pending'",
            [task.id],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        let selected_slot = latest_pending.map_or(latest_slot, |pending| pending.max(latest_slot));
        transaction.execute(
            "DELETE FROM scheduled_task_runs
             WHERE task_id = ?1 AND status = 'pending' AND scheduled_for < ?2",
            params![task.id, selected_slot],
        )?;
        let inserted = if selected_slot == latest_slot {
            transaction.execute(
                "INSERT OR IGNORE INTO scheduled_task_runs
                 (task_id, task_name, scheduled_for, status)
                 VALUES (?1, ?2, ?3, 'pending')",
                params![task.id, task.name, selected_slot],
            )?
        } else {
            0
        };
        transaction.commit()?;
        return Ok(inserted);
    }
    let mut inserted = 0;
    for scheduled_for in scheduled_slots {
        inserted += transaction.execute(
            "INSERT OR IGNORE INTO scheduled_task_runs
             (task_id, task_name, scheduled_for, status)
             VALUES (?1, ?2, ?3, 'pending')",
            params![task.id, task.name, scheduled_for],
        )?;
    }
    transaction.commit()?;
    Ok(inserted)
}

/// Reconcile stale runs and claim one pending run per available task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `worker_id` - Claiming worker identifier.
/// * `claimed_at` - Deterministic claim timestamp.
/// * `lease_seconds` - Claim lease duration.
///
/// # Returns
///
/// Claims owned by the requesting worker.
pub fn claim_ready_scheduled_runs(
    auth_db_path: impl AsRef<Path>,
    worker_id: &str,
    claimed_at: f64,
    lease_seconds: f64,
) -> Result<Vec<ScheduledRunClaim>, BusinessRepositoryError> {
    let auth_db_path = auth_db_path.as_ref();
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "UPDATE scheduled_task_runs
         SET status = 'unknown', finished_at = ?1, claim_expires_at = NULL
         WHERE status = 'running' AND claim_expires_at <= ?1",
        [claimed_at],
    )?;
    transaction.execute(
        "UPDATE scheduled_tasks
         SET last_run_at = ?1, last_status = 'unknown', updated_at = ?1
         WHERE id IN (
             SELECT task_id FROM scheduled_task_runs
             WHERE status = 'unknown' AND finished_at = ?1
         )",
        [claimed_at],
    )?;
    transaction.execute(
        "UPDATE scheduled_task_runs
         SET status = 'pending', worker_id = NULL, claim_expires_at = NULL,
             claimed_at = NULL
         WHERE status = 'claimed' AND claim_expires_at <= ?1",
        [claimed_at],
    )?;

    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT run.id, run.task_id, run.scheduled_for
             FROM scheduled_task_runs AS run
             JOIN scheduled_tasks AS task ON task.id = run.task_id
             WHERE run.status = 'pending'
               AND task.enabled = 1
               AND task.job_spec IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM scheduled_task_runs AS active
                   WHERE active.task_id = run.task_id
                     AND active.status IN ('claimed', 'running')
               )
             ORDER BY run.scheduled_for, run.id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut claimed_rows = Vec::new();
    for (run_id, task_id, scheduled_for) in candidates {
        let count = transaction.execute(
            "UPDATE scheduled_task_runs
             SET status = 'claimed', worker_id = ?1, claimed_at = ?2,
                 claim_expires_at = ?3
             WHERE id = ?4 AND status = 'pending'
               AND NOT EXISTS (
                   SELECT 1 FROM scheduled_task_runs AS active
                   WHERE active.task_id = ?5
                     AND active.status IN ('claimed', 'running')
               )",
            params![
                worker_id,
                claimed_at,
                claimed_at + lease_seconds,
                run_id,
                task_id
            ],
        )?;
        if count > 0 {
            claimed_rows.push((run_id, task_id, scheduled_for));
        }
    }
    transaction.commit()?;

    let mut claims = Vec::new();
    for (run_id, task_id, scheduled_for) in claimed_rows {
        let Some(task) = get_scheduled_task(auth_db_path, task_id)? else {
            fail_unexecutable_claim(auth_db_path, run_id, worker_id, claimed_at)?;
            continue;
        };
        if !task.enabled || task.job.is_none() {
            fail_unexecutable_claim(auth_db_path, run_id, worker_id, claimed_at)?;
            continue;
        }
        claims.push(ScheduledRunClaim {
            run_id,
            scheduled_for,
            worker_id: worker_id.to_string(),
            task,
        });
    }
    Ok(claims)
}

/// Mark a claimed scheduled run as started.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `run_id` - Run row identifier.
/// * `worker_id` - Owning worker identifier.
/// * `started_at` - Execution start timestamp.
/// * `lease_seconds` - Running lease duration.
///
/// # Returns
///
/// True when the owning claim transitioned to running.
pub fn start_scheduled_run(
    auth_db_path: impl AsRef<Path>,
    run_id: i64,
    worker_id: &str,
    started_at: f64,
    lease_seconds: f64,
) -> Result<bool, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let count = connection.execute(
        "UPDATE scheduled_task_runs
         SET status = 'running', started_at = ?1, claim_expires_at = ?2
         WHERE id = ?3 AND worker_id = ?4 AND status = 'claimed'",
        params![started_at, started_at + lease_seconds, run_id, worker_id],
    )?;
    Ok(count > 0)
}

/// Renew a running claim and worker heartbeat together.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `run_id` - Run row identifier.
/// * `worker_id` - Owning worker identifier.
/// * `heartbeat_at` - Current Unix timestamp.
/// * `lease_seconds` - Running lease duration.
///
/// # Returns
///
/// True when the run lease was renewed.
pub fn heartbeat_scheduled_run(
    auth_db_path: impl AsRef<Path>,
    run_id: i64,
    worker_id: &str,
    heartbeat_at: f64,
    lease_seconds: f64,
) -> Result<bool, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "INSERT INTO scheduler_workers (worker_id, started_at, heartbeat_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(worker_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at, heartbeat_at],
    )?;
    transaction.execute(
        "INSERT INTO service_heartbeats (service, instance_id, started_at, heartbeat_at)
         VALUES ('worker', ?1, ?2, ?2)
         ON CONFLICT(service, instance_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![worker_id, heartbeat_at],
    )?;
    let count = transaction.execute(
        "UPDATE scheduled_task_runs SET claim_expires_at = ?1
         WHERE id = ?2 AND worker_id = ?3 AND status = 'running'",
        params![heartbeat_at + lease_seconds, run_id, worker_id],
    )?;
    transaction.commit()?;
    Ok(count > 0)
}

/// Finish one claimed or running scheduled task.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `claim` - Durable run claim.
/// * `status` - Terminal run status.
/// * `output_summary` - Bounded internal output summary.
/// * `finished_at` - Completion timestamp.
///
/// # Returns
///
/// True when the owning run was finalized.
pub fn finish_scheduled_run(
    auth_db_path: impl AsRef<Path>,
    claim: &ScheduledRunClaim,
    status: &str,
    output_summary: &str,
    finished_at: f64,
) -> Result<bool, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count = transaction.execute(
        "UPDATE scheduled_task_runs
         SET status = ?1, finished_at = ?2, claim_expires_at = NULL,
             output_summary = ?3
         WHERE id = ?4 AND worker_id = ?5 AND status IN ('claimed', 'running')",
        params![
            status,
            finished_at,
            output_summary,
            claim.run_id,
            claim.worker_id
        ],
    )?;
    if count > 0 {
        transaction.execute(
            "UPDATE scheduled_tasks
             SET last_run_at = ?1, last_status = ?2, updated_at = ?1
             WHERE id = ?3",
            params![finished_at, status, claim.task.id],
        )?;
    }
    transaction.commit()?;
    Ok(count > 0)
}

/// Read scheduler cursor, worker heartbeats, and recent run statuses.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `current_time` - Current Unix timestamp used for health classification.
/// * `healthy_window_seconds` - Maximum healthy heartbeat age.
/// * `run_limit` - Maximum recent run count.
///
/// # Returns
///
/// Administrator scheduler status payload.
pub fn get_scheduler_status(
    auth_db_path: impl AsRef<Path>,
    current_time: f64,
    healthy_window_seconds: f64,
    run_limit: usize,
) -> Result<SchedulerStatusResponse, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let last_checked_at = connection.query_row(
        "SELECT last_checked_at FROM scheduler_state WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let workers = {
        let mut statement = connection.prepare(
            "SELECT worker_id, started_at, heartbeat_at
             FROM scheduler_workers ORDER BY heartbeat_at DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                let heartbeat_at = row.get::<_, f64>(2)?;
                Ok(SchedulerWorkerInfo {
                    worker_id: row.get(0)?,
                    started_at: row.get(1)?,
                    heartbeat_at,
                    is_healthy: heartbeat_at >= current_time - healthy_window_seconds,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let recent_runs = {
        let mut statement = connection.prepare(
            "SELECT id, task_id, task_name, scheduled_for, status, worker_id,
                    claimed_at, started_at, finished_at
             FROM scheduled_task_runs ORDER BY scheduled_for DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([run_limit as i64], scheduled_task_run_from_row)?;
        collect_rows(rows)?
    };
    Ok(SchedulerStatusResponse {
        last_checked_at,
        workers,
        recent_runs,
    })
}

fn fail_unexecutable_claim(
    auth_db_path: &Path,
    run_id: i64,
    worker_id: &str,
    finished_at: f64,
) -> Result<(), BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection.execute(
        "UPDATE scheduled_task_runs
         SET status = 'error', finished_at = ?1, claim_expires_at = NULL,
             output_summary = 'Task is no longer executable'
         WHERE id = ?2 AND worker_id = ?3 AND status = 'claimed'",
        params![finished_at, run_id, worker_id],
    )?;
    Ok(())
}
fn scheduled_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTaskInfo> {
    let job_spec = row.get::<_, Option<String>>(2)?;
    let job = job_spec
        .as_deref()
        .map(|value| {
            serde_json::from_str(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
            })
        })
        .transpose()?;
    Ok(ScheduledTaskInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        job,
        legacy_command: row.get(3)?,
        cron: row.get(4)?,
        timezone: row.get(5)?,
        timeout_seconds: row.get(6)?,
        coalesce: row.get::<_, i64>(7)? != 0,
        enabled: row.get::<_, i64>(8)? != 0,
        last_run_at: row.get(9)?,
        last_status: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn scheduled_task_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTaskRunInfo> {
    Ok(ScheduledTaskRunInfo {
        id: row.get(0)?,
        task_id: row.get(1)?,
        task_name: row.get(2)?,
        scheduled_for: row.get(3)?,
        status: row.get(4)?,
        worker_id: row.get(5)?,
        claimed_at: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

fn get_scheduled_task_from_connection(
    connection: &Connection,
    task_id: i64,
) -> Result<Option<ScheduledTaskInfo>, BusinessRepositoryError> {
    connection
        .query_row(
            "SELECT id, name, job_spec, legacy_command, cron, timezone, timeout_seconds, coalesce, \
                    enabled, last_run_at, last_status, created_at, updated_at \
             FROM scheduled_tasks WHERE id = ?1",
            [task_id],
            scheduled_task_from_row,
        )
        .optional()
        .map_err(BusinessRepositoryError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use ps_domain::{ScheduledIndexJob, ScheduledJobSpec};
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::migrate_auth_database;

    #[test]
    fn scheduler_repository_validates_typed_jobs_and_replaces_legacy_rows() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let valid_job = ScheduledJobSpec::Index(ScheduledIndexJob {
            metadata_file: Some("journals.csv".to_string()),
            notify: true,
            push: false,
        });
        let created = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Typed index",
                job: &valid_job,
                cron: "0 1 * * *",
                timezone: "UTC",
                timeout_seconds: 3_600,
                coalesce: true,
                enabled: true,
            },
        )
        .expect("typed task should be created");

        assert_eq!(created.job.as_ref(), Some(&valid_job));
        assert_eq!(created.legacy_command, None);

        let invalid_job = ScheduledJobSpec::Index(ScheduledIndexJob {
            metadata_file: Some("../journals.csv".to_string()),
            notify: false,
            push: false,
        });
        let error = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Invalid index",
                job: &invalid_job,
                cron: "0 1 * * *",
                timezone: "UTC",
                timeout_seconds: 3_600,
                coalesce: true,
                enabled: true,
            },
        )
        .expect_err("unsafe path should be rejected");
        assert!(matches!(
            error,
            BusinessRepositoryError::InvalidScheduledJob(_)
        ));

        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO scheduled_tasks
                 (name, job_spec, legacy_command, cron, enabled, last_status, created_at, updated_at)
                 VALUES ('Legacy', NULL, 'index --update && push', '0 2 * * *', 0, '', 1.0, 1.0)",
                [],
            )
            .expect("legacy fixture should insert");
        let legacy_id = connection.last_insert_rowid();
        drop(connection);

        let error = update_scheduled_task(
            &auth_db_path,
            ScheduledTaskUpdateParams {
                task_id: legacy_id,
                name: None,
                job: None,
                cron: None,
                timezone: None,
                timeout_seconds: None,
                coalesce: None,
                enabled: Some(true),
            },
        )
        .expect_err("legacy task should not be enabled");
        assert!(matches!(
            error,
            BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled
        ));

        let replaced = update_scheduled_task(
            &auth_db_path,
            ScheduledTaskUpdateParams {
                task_id: legacy_id,
                name: None,
                job: Some(&valid_job),
                cron: None,
                timezone: None,
                timeout_seconds: None,
                coalesce: None,
                enabled: Some(true),
            },
        )
        .expect("legacy task should accept a typed replacement")
        .expect("legacy task should still exist");
        assert_eq!(replaced.job, Some(valid_job));
        assert_eq!(replaced.legacy_command, None);
        assert!(replaced.enabled);
    }

    #[test]
    fn scheduler_claims_are_unique_and_follow_crash_recovery_rules() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let task = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Durable task",
                job: &ScheduledJobSpec::Index(ScheduledIndexJob {
                    metadata_file: None,
                    notify: false,
                    push: false,
                }),
                cron: "* * * * *",
                timezone: "UTC",
                timeout_seconds: 60,
                coalesce: false,
                enabled: true,
            },
        )
        .expect("task should be created");
        assert_eq!(
            enqueue_scheduled_runs(&auth_db_path, &task, &[60]).expect("run should be queued"),
            1
        );

        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for worker_id in ["worker-a", "worker-b"] {
            let auth_db_path = auth_db_path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                claim_ready_scheduled_runs(&auth_db_path, worker_id, 100.0, 10.0)
                    .expect("concurrent claim should complete")
            }));
        }
        barrier.wait();
        let claims = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("claim thread should finish"))
            .collect::<Vec<_>>();
        assert_eq!(claims.len(), 1);
        let original_run_id = claims[0].run_id;

        let reclaimed = claim_ready_scheduled_runs(&auth_db_path, "worker-c", 111.0, 10.0)
            .expect("stale unstarted claim should be reclaimed");
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].run_id, original_run_id);
        assert_eq!(reclaimed[0].worker_id, "worker-c");
        assert!(
            start_scheduled_run(&auth_db_path, original_run_id, "worker-c", 112.0, 10.0)
                .expect("reclaimed run should start")
        );

        let after_running_expiry =
            claim_ready_scheduled_runs(&auth_db_path, "worker-d", 123.0, 10.0)
                .expect("stale running reconciliation should complete");
        assert!(after_running_expiry.is_empty());
        let status = get_scheduler_status(&auth_db_path, 123.0, 90.0, 10)
            .expect("scheduler status should load");
        assert_eq!(status.recent_runs[0].status, "unknown");
        assert_eq!(status.recent_runs[0].id, original_run_id);
    }

    #[test]
    fn scheduler_cursor_heartbeat_and_coalescing_are_persistent() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let task = create_scheduled_task(
            &auth_db_path,
            ScheduledTaskCreateParams {
                name: "Coalesced task",
                job: &ScheduledJobSpec::Index(ScheduledIndexJob {
                    metadata_file: None,
                    notify: false,
                    push: false,
                }),
                cron: "* * * * *",
                timezone: "UTC",
                timeout_seconds: 60,
                coalesce: true,
                enabled: true,
            },
        )
        .expect("task should be created");

        record_scheduler_check(&auth_db_path, 100.0).expect("cursor should advance");
        record_scheduler_check(&auth_db_path, 90.0).expect("older cursor should be ignored");
        assert_eq!(
            get_scheduler_last_checked_at(&auth_db_path).expect("cursor should load"),
            Some(100.0)
        );
        record_scheduler_heartbeat(&auth_db_path, "worker-a", 110.0)
            .expect("heartbeat should persist");
        assert!(
            crate::has_recent_service_heartbeat(&auth_db_path, 150.0, 60.0)
                .expect("restore-safety heartbeat should load")
        );
        assert!(
            get_scheduler_status(&auth_db_path, 150.0, 60.0, 10)
                .expect("healthy status should load")
                .workers[0]
                .is_healthy
        );
        assert!(
            !get_scheduler_status(&auth_db_path, 200.0, 60.0, 10)
                .expect("stale status should load")
                .workers[0]
                .is_healthy
        );

        assert_eq!(
            enqueue_scheduled_runs(&auth_db_path, &task, &[60, 120, 180])
                .expect("coalesced slots should queue"),
            1
        );
        assert_eq!(
            enqueue_scheduled_runs(&auth_db_path, &task, &[120])
                .expect("an older competing tick should not replace the latest slot"),
            0
        );
        let runs = get_scheduler_status(&auth_db_path, 200.0, 60.0, 10)
            .expect("run status should load")
            .recent_runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].scheduled_for, 180);
    }
}
