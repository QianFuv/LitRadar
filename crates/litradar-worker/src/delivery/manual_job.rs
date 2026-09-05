//! Durable top-level manual delivery job execution.

use std::path::Path;

use litradar_domain::UserId;
use litradar_storage::{
    DeliveryRepositoryError, DeliveryRunClaimOutcome, DeliveryRunMode, DeliveryRunRecord,
    DeliveryRunStatus, DeliveryTriggerKind, SecretCodec, StorageConfig,
};

use super::{
    run_manual_weekly_push, DeliveryError, DeliveryExecutionControl, DeliveryExecutionControlError,
    ManualWeeklyPushConfig, ManualWeeklyPushOutcome, MANUAL_DELIVERY_AI_REQUEST_BUDGET,
};

const MANUAL_DELIVERY_HTTP_TIMEOUT_SECONDS: u64 = 120;
const MANUAL_DELIVERY_RETRY_ATTEMPTS: usize = 3;
const MANUAL_DELIVERY_DEDUPE_RETENTION_DAYS: i64 = 60;
const MANUAL_DELIVERY_LEASE_GRACE_SECONDS: f64 = 30.0;

/// Execute one persisted top-level manual delivery run.
///
/// The child process claims the row with a revision-fenced lease, loads all user settings from
/// SQLite, and persists a fixed terminal status before returning.
///
/// # Arguments
///
/// * `storage_config` - Authoritative project and authentication database paths.
/// * `secret_codec` - Deployment secret codec used to load integration credentials.
/// * `delivery_run_id` - Internal durable run identifier selected by the dispatcher.
/// * `owner_id` - Dispatcher-generated owner identity shared with process supervision.
///
/// # Returns
///
/// The terminal run, or an infrastructure error when durable ownership cannot be established.
pub fn run_manual_delivery_job(
    storage_config: &StorageConfig,
    secret_codec: SecretCodec,
    delivery_run_id: i64,
    owner_id: &str,
) -> Result<DeliveryRunRecord, DeliveryError> {
    let auth_db_path = storage_config.auth_db_path();
    let candidate = litradar_storage::load_delivery_run(auth_db_path, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if candidate.status.is_terminal() {
        return Ok(candidate);
    }

    let now = super::unix_time();
    let deadline_at = candidate.deadline_at.unwrap_or(now);
    let lease_seconds = (deadline_at - now)
        .max(1.0)
        .min(super::MANUAL_DELIVERY_JOB_DEADLINE_SECONDS as f64)
        + MANUAL_DELIVERY_LEASE_GRACE_SECONDS;
    let claimed = match litradar_storage::claim_delivery_run(
        auth_db_path,
        candidate.id,
        owner_id,
        candidate.revision,
        now,
        lease_seconds,
    ) {
        Ok(DeliveryRunClaimOutcome::Claimed(record)) => record,
        Ok(DeliveryRunClaimOutcome::Unavailable(record)) => return Ok(record),
        Ok(DeliveryRunClaimOutcome::Busy(_)) => return Err(DeliveryError::Busy),
        Err(DeliveryRepositoryError::Conflict) => {
            return litradar_storage::load_delivery_run(auth_db_path, delivery_run_id)?
                .ok_or(DeliveryRepositoryError::NotFound)
                .map_err(DeliveryError::from);
        }
        Err(error) => return Err(error.into()),
    };

    if !is_valid_manual_job(&claimed) {
        return finalize_owned_manual_run(
            auth_db_path,
            &claimed,
            owner_id,
            DeliveryRunStatus::Failed,
            None,
            Some("invalid_job_context"),
        );
    }
    if claimed.cancellation_requested {
        return finalize_owned_manual_run(
            auth_db_path,
            &claimed,
            owner_id,
            DeliveryRunStatus::Cancelled,
            None,
            Some("cancelled"),
        );
    }
    if now >= deadline_at {
        return finalize_owned_manual_run(
            auth_db_path,
            &claimed,
            owner_id,
            DeliveryRunStatus::TimedOut,
            None,
            Some("deadline_exceeded"),
        );
    }

    let running = match litradar_storage::start_delivery_run(
        auth_db_path,
        claimed.id,
        owner_id,
        claimed.revision,
        now,
    ) {
        Ok(record) => record,
        Err(DeliveryRepositoryError::Conflict) => {
            let current = litradar_storage::load_delivery_run(auth_db_path, claimed.id)?
                .ok_or(DeliveryRepositoryError::NotFound)?;
            if current.owner_id.as_deref() == Some(owner_id) && current.cancellation_requested {
                return finalize_owned_manual_run(
                    auth_db_path,
                    &current,
                    owner_id,
                    DeliveryRunStatus::Cancelled,
                    None,
                    Some("cancelled"),
                );
            }
            return Err(DeliveryRepositoryError::Conflict.into());
        }
        Err(error) => return Err(error.into()),
    };

    let cancellation_db_path = auth_db_path.to_path_buf();
    let execution_control =
        DeliveryExecutionControl::new(deadline_at, MANUAL_DELIVERY_AI_REQUEST_BUDGET, move || {
            litradar_storage::load_delivery_run(&cancellation_db_path, delivery_run_id)
                .map(|record| {
                    record
                        .map(|record| record.cancellation_requested || record.status.is_terminal())
                        .unwrap_or(true)
                })
                .map_err(|_| ())
        });
    let user_id = UserId(running.user_id.expect("validated manual job has a user"));
    let window_duration = std::time::Duration::try_from_secs_f64(running.created_at)
        .map_err(|_| DeliveryError::Manual("Manual job creation time is invalid".into()))?;
    let result = run_manual_weekly_push(&ManualWeeklyPushConfig {
        window_end: chrono::DateTime::from(std::time::UNIX_EPOCH + window_duration),
        storage_config: storage_config.clone(),
        secret_codec,
        user_id,
        attempt_id: running.external_id.clone(),
        ai_model: None,
        max_candidates: None,
        timeout_seconds: MANUAL_DELIVERY_HTTP_TIMEOUT_SECONDS,
        retry_attempts: MANUAL_DELIVERY_RETRY_ATTEMPTS,
        dedupe_retention_days: MANUAL_DELIVERY_DEDUPE_RETENTION_DAYS,
        execution_control: Some(execution_control),
    });
    let current = litradar_storage::load_delivery_run(auth_db_path, delivery_run_id)?
        .ok_or(DeliveryRepositoryError::NotFound)?;
    if current.status.is_terminal() {
        return Ok(current);
    }
    if current.owner_id.as_deref() != Some(owner_id) {
        return Err(DeliveryRepositoryError::Conflict.into());
    }

    match result {
        Ok(outcome) => finalize_manual_outcome(auth_db_path, &current, owner_id, &outcome),
        Err(error) => {
            let (status, error_code) = manual_error_terminal(&error);
            finalize_owned_manual_run(
                auth_db_path,
                &current,
                owner_id,
                status,
                None,
                Some(error_code),
            )
        }
    }
}

fn is_valid_manual_job(run: &DeliveryRunRecord) -> bool {
    run.trigger_kind == DeliveryTriggerKind::Manual
        && run.mode == DeliveryRunMode::Execute
        && run.user_id.is_some_and(|user_id| user_id > 0)
        && run.db_name.is_none()
        && run.deadline_at.is_some_and(|deadline| {
            deadline.is_finite()
                && deadline > run.created_at
                && deadline - run.created_at
                    <= super::MANUAL_DELIVERY_JOB_DEADLINE_SECONDS as f64 + 1.0
        })
}

fn finalize_manual_outcome(
    auth_db_path: &Path,
    run: &DeliveryRunRecord,
    owner_id: &str,
    outcome: &ManualWeeklyPushOutcome,
) -> Result<DeliveryRunRecord, DeliveryError> {
    let status = match outcome.status {
        litradar_domain::ManualPushState::Completed => DeliveryRunStatus::Completed,
        litradar_domain::ManualPushState::Unknown => DeliveryRunStatus::Unknown,
        litradar_domain::ManualPushState::Failed => DeliveryRunStatus::Failed,
        litradar_domain::ManualPushState::Cancelled => DeliveryRunStatus::Cancelled,
        litradar_domain::ManualPushState::TimedOut => DeliveryRunStatus::TimedOut,
        litradar_domain::ManualPushState::Idle
        | litradar_domain::ManualPushState::Pending
        | litradar_domain::ManualPushState::Running => DeliveryRunStatus::Failed,
    };
    let error_code = match status {
        DeliveryRunStatus::Unknown => Some("ambiguous_delivery"),
        DeliveryRunStatus::Failed => Some("delivery_failed"),
        _ => None,
    };
    let result_json = serde_json::to_string(outcome)
        .map_err(|_| DeliveryError::Manual("Manual delivery result serialization failed".into()))?;
    finalize_owned_manual_run(
        auth_db_path,
        run,
        owner_id,
        status,
        Some(&result_json),
        error_code,
    )
}

fn manual_error_terminal(error: &DeliveryError) -> (DeliveryRunStatus, &'static str) {
    match error {
        DeliveryError::Control(DeliveryExecutionControlError::Cancelled) => {
            (DeliveryRunStatus::Cancelled, "cancelled")
        }
        DeliveryError::Control(DeliveryExecutionControlError::TimedOut) => {
            (DeliveryRunStatus::TimedOut, "deadline_exceeded")
        }
        DeliveryError::Control(DeliveryExecutionControlError::StateUnavailable) => {
            (DeliveryRunStatus::Failed, "cancellation_state_unavailable")
        }
        DeliveryError::Control(DeliveryExecutionControlError::AiRequestBudgetExhausted) => {
            (DeliveryRunStatus::Failed, "ai_request_budget_exhausted")
        }
        DeliveryError::Busy => (DeliveryRunStatus::Failed, "workflow_busy"),
        DeliveryError::Index(_) => (DeliveryRunStatus::Failed, "index_storage_failed"),
        DeliveryError::Business(_) => (DeliveryRunStatus::Failed, "business_storage_failed"),
        DeliveryError::Durable(_) => (DeliveryRunStatus::Failed, "delivery_storage_failed"),
        DeliveryError::Auth(_) => (DeliveryRunStatus::Failed, "auth_storage_failed"),
        DeliveryError::Recommendation(_) => (DeliveryRunStatus::Failed, "recommendation_failed"),
        DeliveryError::Ai(_) => (DeliveryRunStatus::Failed, "ai_failed"),
        DeliveryError::PushPlus(_) => (DeliveryRunStatus::Failed, "pushplus_failed"),
        DeliveryError::Manual(_) => (DeliveryRunStatus::Failed, "manual_validation_failed"),
    }
}

fn finalize_owned_manual_run(
    auth_db_path: &Path,
    run: &DeliveryRunRecord,
    owner_id: &str,
    status: DeliveryRunStatus,
    result_json: Option<&str>,
    error_code: Option<&str>,
) -> Result<DeliveryRunRecord, DeliveryError> {
    litradar_storage::finalize_delivery_run(
        auth_db_path,
        run.id,
        owner_id,
        run.revision,
        status,
        result_json,
        error_code,
        super::unix_time(),
    )
    .map_err(DeliveryError::from)
}

#[cfg(test)]
mod tests {
    use litradar_storage::{
        DeliveryRunAdmissionOutcome, DeliveryRunCreate, DeliveryRunMode, DeliveryRunStatus,
        DeliveryTriggerKind, DeliveryWorkflow, SecretCodec, StorageConfig,
    };
    use tempfile::TempDir;

    use super::run_manual_delivery_job;

    #[test]
    fn manual_push_job_completes_from_authoritative_sqlite_state() {
        let fixture = ManualJobFixture::new();
        let run = fixture.admit("manual-complete", fixture.now + 60.0);

        let terminal = run_manual_delivery_job(
            &fixture.storage,
            fixture.codec.clone(),
            run.id,
            "manual-test-owner",
        )
        .expect("manual job should persist completion");

        assert_eq!(terminal.status, DeliveryRunStatus::Completed);
        assert_eq!(terminal.user_id, Some(fixture.user_id));
        assert!(terminal.owner_id.is_none());
        assert!(terminal.result_json.as_deref().is_some_and(|result| {
            result.contains("Recommendation settings are not enabled; skipped push")
        }));
    }

    #[test]
    fn manual_push_job_finalizes_an_expired_queued_run() {
        let fixture = ManualJobFixture::new();
        let run = fixture.admit_at("manual-expired", fixture.now - 10.0, fixture.now - 1.0);

        let terminal = run_manual_delivery_job(
            &fixture.storage,
            fixture.codec.clone(),
            run.id,
            "manual-test-owner",
        )
        .expect("expired job should reach a durable terminal state");

        assert_eq!(terminal.status, DeliveryRunStatus::TimedOut);
        assert_eq!(terminal.error_code.as_deref(), Some("deadline_exceeded"));
    }

    #[test]
    fn manual_push_job_reclaims_an_expired_owner_after_restart() {
        let fixture = ManualJobFixture::new();
        let run = fixture.admit("manual-restart", fixture.now + 60.0);
        let claimed = match litradar_storage::claim_delivery_run(
            fixture.storage.auth_db_path(),
            run.id,
            "crashed-owner",
            run.revision,
            fixture.now - 10.0,
            1.0,
        )
        .expect("fixture owner should claim")
        {
            litradar_storage::DeliveryRunClaimOutcome::Claimed(claimed) => claimed,
            other => panic!("unexpected fixture claim: {other:?}"),
        };
        litradar_storage::start_delivery_run(
            fixture.storage.auth_db_path(),
            claimed.id,
            "crashed-owner",
            claimed.revision,
            fixture.now - 10.0,
        )
        .expect("fixture owner should start before its lease expires");

        let terminal = run_manual_delivery_job(
            &fixture.storage,
            fixture.codec.clone(),
            run.id,
            "replacement-owner",
        )
        .expect("replacement child should reclaim the expired run");

        assert_eq!(terminal.status, DeliveryRunStatus::Completed);
        assert!(terminal.revision >= 4);
        assert!(terminal.owner_id.is_none());
    }

    struct ManualJobFixture {
        _directory: TempDir,
        storage: StorageConfig,
        codec: SecretCodec,
        user_id: i64,
        now: f64,
    }

    impl ManualJobFixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("manual job fixture should create");
            let storage = StorageConfig::from_project_root(directory.path());
            litradar_storage::migrate_storage(&storage).expect("fixture storage should migrate");
            let codec = SecretCodec::from_key([71_u8; 32]);
            let now = super::super::unix_time();
            let user = litradar_storage::bootstrap_admin(
                storage.auth_db_path(),
                "manual-job-user",
                "hash",
                "salt",
                now,
            )
            .expect("fixture user should create");
            Self {
                _directory: directory,
                storage,
                codec,
                user_id: user.id.value(),
                now,
            }
        }

        fn admit(
            &self,
            external_id: &str,
            deadline_at: f64,
        ) -> litradar_storage::DeliveryRunRecord {
            self.admit_at(external_id, self.now, deadline_at)
        }

        fn admit_at(
            &self,
            external_id: &str,
            created_at: f64,
            deadline_at: f64,
        ) -> litradar_storage::DeliveryRunRecord {
            match litradar_storage::admit_delivery_run(
                self.storage.auth_db_path(),
                &DeliveryRunCreate {
                    external_id: external_id.to_string(),
                    workflow: DeliveryWorkflow::Push,
                    scope_key: format!("manual-user-{}", self.user_id),
                    db_name: None,
                    trigger_kind: DeliveryTriggerKind::Manual,
                    mode: DeliveryRunMode::Execute,
                    user_id: Some(self.user_id),
                    deadline_at: Some(deadline_at),
                    created_at,
                },
            )
            .expect("manual run should admit")
            {
                DeliveryRunAdmissionOutcome::Enqueued(run) => run,
                _ => panic!("manual admission should enqueue a new run"),
            }
        }
    }
}
