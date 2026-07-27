//! Durable SQLite delivery admission, lease, checkpoint, and recovery helpers.

use std::time::{SystemTime, UNIX_EPOCH};

use litradar_storage::{
    DeliveryCheckpointRecord, DeliveryCheckpointStatus, DeliveryCheckpointUpdate,
    DeliveryLeaseAcquireOutcome, DeliveryLeaseRecord, DeliveryRunAdmissionOutcome,
    DeliveryRunClaimOutcome, DeliveryRunCreate, DeliveryRunMode, DeliveryRunRecord,
    DeliveryRunStatus, DeliveryTriggerKind,
};

use super::*;

pub(super) const DELIVERY_LEASE_SECONDS: f64 = 3_600.0;

pub(super) enum DurableDeliveryAdmission {
    Owned(Box<DurableDeliveryRun>),
    Terminal(Box<DeliveryRunRecord>),
}

pub(super) struct DurableDeliveryRun {
    pub(super) run: DeliveryRunRecord,
    pub(super) lease: DeliveryLeaseRecord,
    pub(super) checkpoint: Option<DeliveryCheckpointRecord>,
    pub(super) owner_id: String,
    pub(super) workflow: litradar_storage::DeliveryWorkflow,
    pub(super) db_name: String,
    pub(super) did_take_over_competing_run: bool,
}

impl DurableDeliveryRun {
    pub(super) fn renew(&mut self, auth_db_path: &Path) -> Result<f64, DeliveryError> {
        let now = unix_now();
        self.run = litradar_storage::renew_delivery_run(
            auth_db_path,
            self.run.id,
            &self.owner_id,
            self.run.revision,
            now,
            DELIVERY_LEASE_SECONDS,
        )?;
        self.lease = litradar_storage::renew_delivery_lease(
            auth_db_path,
            self.workflow,
            &self.db_name,
            self.run.id,
            &self.owner_id,
            self.lease.revision,
            now,
            DELIVERY_LEASE_SECONDS,
        )?;
        Ok(now)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize_with_checkpoint(
        &mut self,
        auth_db_path: &Path,
        run_status: DeliveryRunStatus,
        checkpoint_status: DeliveryCheckpointStatus,
        snapshot: &RecommendationSnapshot,
        last_completed_run_at: Option<String>,
        result_json: Option<&str>,
        error_code: Option<&str>,
        now: f64,
    ) -> Result<(), DeliveryError> {
        let finalization = litradar_storage::finalize_delivery_run_with_checkpoint(
            auth_db_path,
            self.run.id,
            &self.owner_id,
            self.run.revision,
            run_status,
            result_json,
            error_code,
            self.workflow,
            &self.db_name,
            self.checkpoint.as_ref().map(|record| record.revision),
            &DeliveryCheckpointUpdate {
                status: checkpoint_status,
                snapshot_json: serde_json::to_string(snapshot).map_err(|_| {
                    DeliveryError::Manual("Delivery checkpoint serialization failed".to_string())
                })?,
                last_completed_run_at,
                updated_at: now,
            },
            self.lease.revision,
        )?;
        self.run = finalization.run;
        self.checkpoint = Some(finalization.checkpoint);
        self.lease = finalization.lease;
        Ok(())
    }

    pub(super) fn fail_best_effort(&mut self, auth_db_path: &Path, error_code: &str) {
        let now = unix_now();
        let recovery = litradar_storage::reconcile_delivery_run_after_takeover(
            auth_db_path,
            self.run.id,
            &self.owner_id,
            self.run.revision,
            now,
        )
        .ok();
        let has_ambiguous_attempt = recovery
            .is_some_and(|result| result.unknown_item_count > 0 || result.unknown_dedupe_count > 0);
        if self.run.status.is_active() {
            let terminal_status = if has_ambiguous_attempt {
                DeliveryRunStatus::Unknown
            } else {
                DeliveryRunStatus::Failed
            };
            let terminal_error_code = if has_ambiguous_attempt {
                "ambiguous_delivery"
            } else {
                error_code
            };
            let checkpoint_update = DeliveryCheckpointUpdate {
                status: if has_ambiguous_attempt {
                    DeliveryCheckpointStatus::Unknown
                } else {
                    DeliveryCheckpointStatus::Failed
                },
                snapshot_json: self
                    .checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoint.snapshot_json.clone())
                    .unwrap_or_else(|| "{}".to_string()),
                last_completed_run_at: self
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.last_completed_run_at.clone()),
                updated_at: now,
            };
            if let Ok(finalization) = litradar_storage::finalize_delivery_run_with_checkpoint(
                auth_db_path,
                self.run.id,
                &self.owner_id,
                self.run.revision,
                terminal_status,
                None,
                Some(terminal_error_code),
                self.workflow,
                &self.db_name,
                self.checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoint.revision),
                &checkpoint_update,
                self.lease.revision,
            ) {
                self.run = finalization.run;
                self.checkpoint = Some(finalization.checkpoint);
                self.lease = finalization.lease;
                return;
            }
            if let Ok(run) = litradar_storage::finalize_delivery_run(
                auth_db_path,
                self.run.id,
                &self.owner_id,
                self.run.revision,
                terminal_status,
                None,
                Some(terminal_error_code),
                now,
            ) {
                self.run = run;
            }
        }
        let _ = litradar_storage::release_delivery_lease(
            auth_db_path,
            self.workflow,
            &self.db_name,
            self.run.id,
            &self.owner_id,
            self.lease.revision,
            now,
        );
    }
}

pub(super) fn admit_durable_delivery_run(
    config: &RecommendationRunConfig,
    subscriber_user_id: Option<UserId>,
    external_id: &str,
) -> Result<DurableDeliveryAdmission, DeliveryError> {
    let now = unix_now();
    let workflow = storage_workflow(config.workflow);
    let owner_id = format!("worker-{}", litradar_storage::random_hex(16)?);
    let trigger_kind = if subscriber_user_id.is_some() {
        DeliveryTriggerKind::Manual
    } else {
        DeliveryTriggerKind::Scheduled
    };
    let admission = litradar_storage::admit_delivery_run(
        &config.auth_db_path,
        &DeliveryRunCreate {
            external_id: external_id.to_string(),
            workflow,
            scope_key: config.db_name.clone(),
            db_name: Some(config.db_name.clone()),
            trigger_kind,
            mode: storage_mode(config.mode),
            user_id: subscriber_user_id.map(UserId::value),
            deadline_at: None,
            created_at: now,
        },
    )?;
    let candidate = match admission {
        DeliveryRunAdmissionOutcome::Enqueued(record)
        | DeliveryRunAdmissionOutcome::Existing(record) => record,
        DeliveryRunAdmissionOutcome::Busy(_) => return Err(DeliveryError::Busy),
    };
    if candidate.status.is_terminal() {
        return Ok(DurableDeliveryAdmission::Terminal(Box::new(candidate)));
    }
    let (claimed, did_take_over_competing_run) =
        claim_candidate_or_expired_competitor(&config.auth_db_path, candidate, &owner_id, now)?;
    if claimed.status.is_terminal() {
        return Ok(DurableDeliveryAdmission::Terminal(Box::new(claimed)));
    }
    let expected_user_id = subscriber_user_id.map(UserId::value);
    if claimed.mode != storage_mode(config.mode)
        || claimed.trigger_kind != trigger_kind
        || claimed.user_id != expected_user_id
    {
        let _ = litradar_storage::finalize_delivery_run(
            &config.auth_db_path,
            claimed.id,
            &owner_id,
            claimed.revision,
            DeliveryRunStatus::Failed,
            None,
            Some("recovery_context_mismatch"),
            now,
        );
        return Err(DeliveryError::Manual(
            "Recovered delivery run does not match the current invocation".to_string(),
        ));
    }
    let lease = match litradar_storage::acquire_delivery_lease(
        &config.auth_db_path,
        workflow,
        &config.db_name,
        claimed.id,
        &owner_id,
        now,
        DELIVERY_LEASE_SECONDS,
    )? {
        DeliveryLeaseAcquireOutcome::Acquired(record) => record,
        DeliveryLeaseAcquireOutcome::Busy(_) => {
            let _ = litradar_storage::finalize_delivery_run(
                &config.auth_db_path,
                claimed.id,
                &owner_id,
                claimed.revision,
                DeliveryRunStatus::Skipped,
                None,
                Some("workflow_lease_busy"),
                now,
            );
            return Err(DeliveryError::Busy);
        }
    };
    let mut context = DurableDeliveryRun {
        run: claimed,
        lease,
        checkpoint: None,
        owner_id,
        workflow,
        db_name: config.db_name.clone(),
        did_take_over_competing_run,
    };
    context.checkpoint = match litradar_storage::load_delivery_checkpoint(
        &config.auth_db_path,
        workflow,
        &config.db_name,
    ) {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            let error = DeliveryError::from(error);
            context.fail_best_effort(&config.auth_db_path, "checkpoint_load_failed");
            return Err(error);
        }
    };
    if let Err(error) = litradar_storage::reconcile_delivery_run_after_takeover(
        &config.auth_db_path,
        context.run.id,
        &context.owner_id,
        context.run.revision,
        now,
    ) {
        let error = DeliveryError::from(error);
        context.fail_best_effort(&config.auth_db_path, "recovery_failed");
        return Err(error);
    }
    context.run = match litradar_storage::start_delivery_run(
        &config.auth_db_path,
        context.run.id,
        &context.owner_id,
        context.run.revision,
        now,
    ) {
        Ok(run) => run,
        Err(error) => {
            let error = DeliveryError::from(error);
            context.fail_best_effort(&config.auth_db_path, "run_start_failed");
            return Err(error);
        }
    };
    if config.dedupe_retention_days > 0 {
        let delivered_before = (now - (config.dedupe_retention_days as f64) * 86_400.0).max(0.0);
        if let Err(error) = litradar_storage::cleanup_confirmed_delivery_dedupe(
            &config.auth_db_path,
            workflow,
            &config.db_name,
            delivered_before,
        ) {
            let error = DeliveryError::from(error);
            context.fail_best_effort(&config.auth_db_path, "dedupe_cleanup_failed");
            return Err(error);
        }
    }
    Ok(DurableDeliveryAdmission::Owned(Box::new(context)))
}

pub(super) fn checkpoint_snapshot(
    context: &DurableDeliveryRun,
) -> Result<RecommendationSnapshot, DeliveryError> {
    context
        .checkpoint
        .as_ref()
        .map(|record| {
            serde_json::from_str(&record.snapshot_json).map_err(|_| {
                DeliveryError::Manual("Stored delivery checkpoint is invalid".to_string())
            })
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(super) fn delivery_dedupe_map(
    config: &RecommendationRunConfig,
    context: &DurableDeliveryRun,
) -> Result<BTreeMap<String, String>, DeliveryError> {
    let rows = litradar_storage::list_delivery_dedupe_for_scope(
        &config.auth_db_path,
        context.workflow,
        &config.db_name,
    )?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                format!("{}:{}", row.user_id, row.article_id),
                row.legacy_delivered_at
                    .unwrap_or_else(|| row.delivered_at.unwrap_or(row.reserved_at).to_string()),
            )
        })
        .collect())
}

pub(super) fn terminal_status_text(status: DeliveryRunStatus) -> &'static str {
    match status {
        DeliveryRunStatus::Completed => "completed",
        DeliveryRunStatus::Failed => "failed",
        DeliveryRunStatus::Cancelled => "cancelled",
        DeliveryRunStatus::TimedOut => "timed_out",
        DeliveryRunStatus::Skipped => "skipped",
        DeliveryRunStatus::Unknown => "unknown",
        DeliveryRunStatus::Queued
        | DeliveryRunStatus::Claimed
        | DeliveryRunStatus::Running
        | DeliveryRunStatus::Cancelling => "running",
    }
}

pub(super) fn storage_workflow(workflow: DeliveryWorkflow) -> litradar_storage::DeliveryWorkflow {
    match workflow {
        DeliveryWorkflow::Notify => litradar_storage::DeliveryWorkflow::Notify,
        DeliveryWorkflow::Push => litradar_storage::DeliveryWorkflow::Push,
    }
}

pub(super) fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn storage_mode(mode: DeliveryMode) -> DeliveryRunMode {
    match mode {
        DeliveryMode::DryRun => DeliveryRunMode::DryRun,
        DeliveryMode::Execute => DeliveryRunMode::Execute,
    }
}

fn claim_candidate_or_expired_competitor(
    auth_db_path: &Path,
    candidate: DeliveryRunRecord,
    owner_id: &str,
    now: f64,
) -> Result<(DeliveryRunRecord, bool), DeliveryError> {
    match litradar_storage::claim_delivery_run(
        auth_db_path,
        candidate.id,
        owner_id,
        candidate.revision,
        now,
        DELIVERY_LEASE_SECONDS,
    )? {
        DeliveryRunClaimOutcome::Claimed(record) => Ok((record, false)),
        DeliveryRunClaimOutcome::Unavailable(record) if record.status.is_terminal() => {
            Ok((record, false))
        }
        DeliveryRunClaimOutcome::Unavailable(_) => Err(DeliveryError::Busy),
        DeliveryRunClaimOutcome::Busy(active) => {
            if candidate.status == DeliveryRunStatus::Queued {
                let _ = litradar_storage::request_delivery_run_cancellation(
                    auth_db_path,
                    candidate.id,
                    candidate.revision,
                    now,
                );
            }
            if !active
                .lease_expires_at
                .is_some_and(|expires_at| expires_at <= now)
            {
                return Err(DeliveryError::Busy);
            }
            match litradar_storage::claim_delivery_run(
                auth_db_path,
                active.id,
                owner_id,
                active.revision,
                now,
                DELIVERY_LEASE_SECONDS,
            )? {
                DeliveryRunClaimOutcome::Claimed(record) => Ok((record, true)),
                DeliveryRunClaimOutcome::Unavailable(record) if record.status.is_terminal() => {
                    Ok((record, true))
                }
                DeliveryRunClaimOutcome::Busy(_) | DeliveryRunClaimOutcome::Unavailable(_) => {
                    Err(DeliveryError::Busy)
                }
            }
        }
    }
}
