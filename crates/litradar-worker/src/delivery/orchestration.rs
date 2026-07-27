//! End-to-end recommendation delivery orchestration.

use std::time::Instant;

use super::candidates::*;
use super::folder::*;
use super::manifests::*;
use super::notify::*;
use super::state::*;
use super::*;

/// Run notification or tracking delivery.
///
/// # Arguments
///
/// * `config` - Worker run configuration.
///
/// # Returns
///
/// Dry-run or execution outcome.
pub fn run_recommendation_delivery(
    config: &RecommendationRunConfig,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    let timeout_seconds = config.timeout_seconds.max(1);
    let mut ai_selector = DefaultDeliveryAiSelector::live(
        timeout_seconds,
        config.retry_attempts,
        config.auth_db_path.clone(),
        config.execution_control.clone(),
    );
    let mut pushplus_sender = LiveDeliveryPushPlusSender::new(
        timeout_seconds,
        config.retry_attempts,
        config.execution_control.clone(),
    )?;
    run_recommendation_delivery_with_services_for_user(
        config,
        None,
        &mut ai_selector,
        &mut pushplus_sender,
    )
}

/// Run notification or tracking delivery for one user.
///
/// # Arguments
///
/// * `config` - Worker run configuration.
/// * `user_id` - User whose subscriber settings should run.
///
/// # Returns
///
/// Dry-run or execution outcome.
pub fn run_recommendation_delivery_for_user(
    config: &RecommendationRunConfig,
    user_id: UserId,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    let timeout_seconds = config.timeout_seconds.max(1);
    let mut ai_selector = DefaultDeliveryAiSelector::live(
        timeout_seconds,
        config.retry_attempts,
        config.auth_db_path.clone(),
        config.execution_control.clone(),
    );
    let mut pushplus_sender = LiveDeliveryPushPlusSender::new(
        timeout_seconds,
        config.retry_attempts,
        config.execution_control.clone(),
    )?;
    run_recommendation_delivery_with_services_for_user(
        config,
        Some(user_id),
        &mut ai_selector,
        &mut pushplus_sender,
    )
}

/// Run a manual weekly push for one authenticated user.
///
/// # Arguments
///
/// * `config` - Manual weekly push configuration.
///
/// # Returns
///
/// Aggregated manual push result across selected change manifests.
pub fn run_manual_weekly_push(
    config: &ManualWeeklyPushConfig,
) -> Result<ManualWeeklyPushOutcome, DeliveryError> {
    let started_at = Instant::now();
    let manual_span = tracing::info_span!(
        "delivery.manual",
        component = "delivery",
        workflow = "manual_weekly_push",
        mode = "execute",
        user_id = config.user_id.value(),
    );
    manual_span.in_scope(|| {
        tracing::info!(
            event = "delivery.manual.started",
            component = "delivery",
            outcome = "started",
        );
        let result = run_manual_weekly_push_inner(config);
        emit_manual_delivery_terminal(&result, started_at);
        result
    })
}

fn run_manual_weekly_push_inner(
    config: &ManualWeeklyPushConfig,
) -> Result<ManualWeeklyPushOutcome, DeliveryError> {
    check_manual_execution_control(config)?;
    let settings = litradar_storage::get_notification_settings(
        config.storage_config.auth_db_path(),
        &config.secret_codec,
        config.user_id,
    )?;
    let Some(settings) = settings.filter(|item| item.enabled) else {
        return Ok(manual_outcome(
            ManualPushState::Completed,
            "Recommendation settings are not enabled; skipped push",
            None,
            None,
        ));
    };

    let delivery_method = nonempty_text(&settings.delivery_method).unwrap_or("folder");
    let folder = litradar_storage::get_tracking_folder(
        config.storage_config.auth_db_path(),
        config.user_id,
    )?;
    let requires_tracking_folder = delivery_method == "folder" || settings.sync_to_tracking_folder;
    if requires_tracking_folder && folder.is_none() {
        return Err(DeliveryError::Manual(
            "No tracking folder configured. Create a folder and set it as tracking first."
                .to_string(),
        ));
    }

    let manifests = manual_weekly_manifests(
        config.storage_config.project_root(),
        &settings.selected_databases,
    )?;
    if manifests.is_empty() {
        let message = if settings.selected_databases.is_empty() {
            "No new weekly articles available"
        } else {
            "No new weekly articles available in selected databases"
        };
        return Ok(manual_outcome(
            ManualPushState::Completed,
            message,
            folder.as_ref().map(|item| item.id),
            folder.as_ref().map(|item| item.name.clone()),
        ));
    }

    if settings.keywords.is_empty() && settings.directions.is_empty() {
        return Ok(manual_outcome(
            ManualPushState::Completed,
            "No keywords or directions configured; skipped push",
            folder.as_ref().map(|item| item.id),
            folder.as_ref().map(|item| item.name.clone()),
        ));
    }

    let workflow = if delivery_method == "pushplus" {
        DeliveryWorkflow::Notify
    } else {
        DeliveryWorkflow::Push
    };
    let mut outcomes = Vec::new();
    for manifest in manifests {
        check_manual_execution_control(config)?;
        let index_db_path = config
            .storage_config
            .resolve_index_db_path(Some(&manifest.db_name))
            .map_err(litradar_storage::IndexRepositoryError::from)?;
        outcomes.push(run_recommendation_delivery_for_user(
            &RecommendationRunConfig {
                auth_db_path: config.storage_config.auth_db_path().to_path_buf(),
                secret_codec: config.secret_codec.clone(),
                index_db_path,
                db_name: manifest.db_name,
                changes_file: Some(manifest.path),
                ai_model: config.ai_model.clone(),
                max_candidates: config.max_candidates,
                timeout_seconds: config.timeout_seconds,
                retry_attempts: config.retry_attempts,
                dedupe_retention_days: config.dedupe_retention_days,
                mode: DeliveryMode::Execute,
                workflow,
                trigger: DeliveryTrigger::Scheduled,
                execution_control: config.execution_control.clone(),
            },
            config.user_id,
        )?);
    }

    Ok(manual_outcome_from_delivery(
        delivery_method,
        folder.as_ref().map(|item| item.id),
        folder.as_ref().map(|item| item.name.clone()),
        &outcomes,
    ))
}

fn check_manual_execution_control(config: &ManualWeeklyPushConfig) -> Result<(), DeliveryError> {
    config
        .execution_control
        .as_ref()
        .map(DeliveryExecutionControl::check)
        .transpose()?;
    Ok(())
}

fn check_execution_control(config: &RecommendationRunConfig) -> Result<(), DeliveryError> {
    config
        .execution_control
        .as_ref()
        .map(DeliveryExecutionControl::check)
        .transpose()?;
    Ok(())
}

#[cfg(test)]
fn run_recommendation_delivery_with_services(
    config: &RecommendationRunConfig,
    ai_selector: &mut impl DeliveryAiSelector,
    pushplus_sender: &mut impl DeliveryPushPlusSender,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    run_recommendation_delivery_with_services_for_user(config, None, ai_selector, pushplus_sender)
}

fn run_recommendation_delivery_with_services_for_user(
    config: &RecommendationRunConfig,
    subscriber_user_id: Option<UserId>,
    ai_selector: &mut impl DeliveryAiSelector,
    pushplus_sender: &mut impl DeliveryPushPlusSender,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    let started_at = Instant::now();
    let workflow_span = tracing::info_span!(
        "delivery.workflow",
        component = "delivery",
        workflow = delivery_workflow_kind(config.workflow),
        mode = delivery_mode_kind(config.mode),
        user_id = tracing::field::Empty,
    );
    if let Some(user_id) = subscriber_user_id {
        workflow_span.record("user_id", user_id.value());
    }
    workflow_span.in_scope(|| {
        tracing::info!(
            event = "delivery.workflow.started",
            component = "delivery",
            outcome = "started",
        );
        let result = execute_recommendation_delivery_with_services_for_user(
            config,
            subscriber_user_id,
            ai_selector,
            pushplus_sender,
        );
        emit_delivery_workflow_terminal(&result, started_at);
        result
    })
}

fn execute_recommendation_delivery_with_services_for_user(
    config: &RecommendationRunConfig,
    subscriber_user_id: Option<UserId>,
    ai_selector: &mut impl DeliveryAiSelector,
    pushplus_sender: &mut impl DeliveryPushPlusSender,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    let manifest = config
        .changes_file
        .as_ref()
        .map(|path| load_change_manifest(path, &config.db_name))
        .transpose()?;
    let source_external_id = match manifest.as_ref().and_then(|value| value.run_id.clone()) {
        Some(run_id) => run_id,
        None => format!("run-{}", litradar_storage::random_hex(16)?),
    };
    let external_id = subscriber_user_id.map_or(source_external_id.clone(), |user_id| {
        format!(
            "user-run-{}",
            litradar_domain::stable_sqlite_id(
                format!("{}:{}", source_external_id, user_id.value()),
                "manual-delivery",
            )
        )
    });
    let mut context = match admit_durable_delivery_run(config, subscriber_user_id, &external_id)? {
        DurableDeliveryAdmission::Terminal(run) => {
            return Ok(outcome(
                config,
                run.id,
                terminal_status_text(run.status),
                Vec::new(),
                Vec::new(),
            ));
        }
        DurableDeliveryAdmission::Owned(context) => *context,
    };
    let run_label = context.run.external_id.clone();
    let result = execute_owned_delivery(
        config,
        subscriber_user_id,
        manifest,
        &run_label,
        &mut context,
        ai_selector,
        pushplus_sender,
    );
    if let Err(error) = &result {
        context.fail_best_effort(&config.auth_db_path, delivery_error_kind(error));
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn execute_owned_delivery(
    config: &RecommendationRunConfig,
    subscriber_user_id: Option<UserId>,
    manifest: Option<litradar_recommend::ChangeManifest>,
    run_label: &str,
    context: &mut DurableDeliveryRun,
    ai_selector: &mut impl DeliveryAiSelector,
    pushplus_sender: &mut impl DeliveryPushPlusSender,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    check_execution_control(config)?;
    let previous_snapshot = checkpoint_snapshot(context)?;
    let previous_completed_at = context
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.last_completed_run_at.clone());
    let current_snapshot = RecommendationSnapshot {
        issue_article_counts: litradar_storage::collect_issue_article_counts(
            &config.index_db_path,
        )?,
        inpress_article_counts: litradar_storage::collect_inpress_article_counts(
            &config.index_db_path,
        )?,
    };
    let existing_items =
        litradar_storage::list_delivery_run_items(&config.auth_db_path, context.run.id)?;
    if context.did_take_over_competing_run
        && !existing_items
            .iter()
            .any(|item| item.item_kind != litradar_storage::DeliveryItemKind::Subscriber)
    {
        return Err(DeliveryError::Manual(
            "Recovered delivery run has no durable input items".to_string(),
        ));
    }
    let input = if existing_items
        .iter()
        .any(|item| item.item_kind != litradar_storage::DeliveryItemKind::Subscriber)
    {
        delivery_input_from_items(&existing_items)
    } else {
        let input = delivery_input_from_source(manifest, &previous_snapshot, &current_snapshot);
        litradar_storage::ensure_delivery_run_items(
            &config.auth_db_path,
            context.run.id,
            &input.item_creates(),
            unix_now(),
        )?;
        input
    };
    if input.is_empty() {
        let now = unix_now();
        context.finalize_with_checkpoint(
            &config.auth_db_path,
            litradar_storage::DeliveryRunStatus::Skipped,
            litradar_storage::DeliveryCheckpointStatus::Idle,
            &current_snapshot,
            previous_completed_at,
            Some(r#"{"candidate_count":0,"subscriber_count":0}"#),
            None,
            now,
        )?;
        return Ok(outcome(
            config,
            context.run.id,
            DeliveryOutcomeState::Idle,
            Vec::new(),
            Vec::new(),
        ));
    }

    let candidates = load_durable_candidates(config, &input)?;
    let candidates = deduplicate_candidates(candidates);
    let candidate_article_ids = candidates
        .iter()
        .map(|candidate| candidate.article_id)
        .collect::<Vec<_>>();
    litradar_storage::ensure_delivery_run_items(
        &config.auth_db_path,
        context.run.id,
        &candidate_article_ids
            .iter()
            .map(|article_id| litradar_storage::DeliveryRunItemCreate {
                item_kind: litradar_storage::DeliveryItemKind::Article,
                item_key: article_id.to_string(),
                user_id: None,
                article_id: Some(*article_id),
            })
            .collect::<Vec<_>>(),
        unix_now(),
    )?;
    finalize_progress_items(config, context)?;

    if candidates.is_empty() {
        let completed_at = utc_now_iso();
        context.finalize_with_checkpoint(
            &config.auth_db_path,
            litradar_storage::DeliveryRunStatus::Completed,
            litradar_storage::DeliveryCheckpointStatus::Completed,
            &current_snapshot,
            Some(completed_at),
            Some(r#"{"candidate_count":0,"subscriber_count":0}"#),
            None,
            unix_now(),
        )?;
        return Ok(outcome(
            config,
            context.run.id,
            DeliveryOutcomeState::Completed,
            Vec::new(),
            Vec::new(),
        ));
    }

    let subscribers = filtered_subscribers(
        &config.auth_db_path,
        &config.secret_codec,
        &config.db_name,
        config.workflow,
        subscriber_user_id,
    )?;
    if subscribers.is_empty() {
        context.finalize_with_checkpoint(
            &config.auth_db_path,
            litradar_storage::DeliveryRunStatus::Skipped,
            litradar_storage::DeliveryCheckpointStatus::Skipped,
            &previous_snapshot,
            previous_completed_at,
            Some(&run_result_json(candidate_article_ids.len(), 0, 0, 0)?),
            None,
            unix_now(),
        )?;
        return Ok(outcome(
            config,
            context.run.id,
            DeliveryOutcomeState::Skipped,
            candidate_article_ids,
            Vec::new(),
        ));
    }
    let subscriber_items = litradar_storage::ensure_delivery_run_items(
        &config.auth_db_path,
        context.run.id,
        &subscribers
            .iter()
            .map(|subscriber| {
                let user_id = subscriber_id_value(subscriber)?;
                Ok(litradar_storage::DeliveryRunItemCreate {
                    item_kind: litradar_storage::DeliveryItemKind::Subscriber,
                    item_key: subscriber.subscriber_id.clone(),
                    user_id: Some(user_id),
                    article_id: None,
                })
            })
            .collect::<Result<Vec<_>, DeliveryError>>()?,
        unix_now(),
    )?;
    let items_by_key = subscriber_items
        .into_iter()
        .map(|item| (item.item_key.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let global_config = load_global_config(config)?;
    let mut defaults = load_defaults();
    if let Some(max_candidates) = config.max_candidates {
        defaults.max_candidates = max_candidates.max(1);
    }
    let candidates_for_model = candidates
        .iter()
        .take(defaults.max_candidates)
        .cloned()
        .collect::<Vec<_>>();
    let candidates_by_id = candidates_by_id(&candidates);
    let mut delivery_dedupe = delivery_dedupe_map(config, context)?;
    let mut plans = Vec::new();
    for subscriber in subscribers {
        check_execution_control(config)?;
        context.renew(&config.auth_db_path)?;
        let item = items_by_key
            .get(&subscriber.subscriber_id)
            .ok_or_else(|| DeliveryError::Manual("Subscriber item is unavailable".to_string()))?;
        let plan = if item.status.is_terminal() {
            plan_from_terminal_item(config, &subscriber, item)?
        } else {
            let claimed_item = litradar_storage::claim_delivery_run_item(
                &config.auth_db_path,
                context.run.id,
                &context.owner_id,
                context.run.revision,
                item.id,
                &context.owner_id,
                unix_now(),
                DELIVERY_LEASE_SECONDS,
            )?;
            process_subscriber(
                config,
                context,
                &claimed_item,
                &subscriber,
                &global_config,
                &defaults,
                run_label,
                &candidates_for_model,
                &candidates_by_id,
                &mut delivery_dedupe,
                ai_selector,
                pushplus_sender,
            )?
        };
        plans.push(plan);
    }

    let has_unknown = plans
        .iter()
        .any(|plan| plan.status == SubscriberDeliveryState::Unknown);
    let has_failure = plans
        .iter()
        .any(|plan| plan.status == SubscriberDeliveryState::Error);
    let (run_status, checkpoint_status, outcome_status, checkpoint_snapshot, error_code) =
        if has_unknown {
            (
                litradar_storage::DeliveryRunStatus::Unknown,
                litradar_storage::DeliveryCheckpointStatus::Unknown,
                DeliveryOutcomeState::Unknown,
                &previous_snapshot,
                Some("ambiguous_delivery"),
            )
        } else if has_failure {
            (
                litradar_storage::DeliveryRunStatus::Failed,
                litradar_storage::DeliveryCheckpointStatus::Failed,
                DeliveryOutcomeState::Failed,
                &previous_snapshot,
                Some("subscriber_failed"),
            )
        } else {
            (
                litradar_storage::DeliveryRunStatus::Completed,
                litradar_storage::DeliveryCheckpointStatus::Completed,
                DeliveryOutcomeState::Completed,
                &current_snapshot,
                None,
            )
        };
    let selected_count = plans
        .iter()
        .map(|plan| plan.selected_article_ids.len())
        .sum();
    let message_count = plans
        .iter()
        .filter(|plan| plan.message_id.is_some())
        .count();
    let result_json = run_result_json(
        candidate_article_ids.len(),
        plans.len(),
        selected_count,
        message_count,
    )?;
    context.finalize_with_checkpoint(
        &config.auth_db_path,
        run_status,
        checkpoint_status,
        checkpoint_snapshot,
        if run_status == litradar_storage::DeliveryRunStatus::Completed {
            Some(utc_now_iso())
        } else {
            previous_completed_at
        },
        Some(&result_json),
        error_code,
        unix_now(),
    )?;
    Ok(outcome(
        config,
        context.run.id,
        outcome_status,
        candidate_article_ids,
        plans,
    ))
}

struct DeliveryInput {
    issue_keys: Vec<String>,
    inpress_keys: Vec<String>,
    article_ids: Vec<i64>,
}

impl DeliveryInput {
    fn is_empty(&self) -> bool {
        self.issue_keys.is_empty() && self.inpress_keys.is_empty() && self.article_ids.is_empty()
    }

    fn item_creates(&self) -> Vec<litradar_storage::DeliveryRunItemCreate> {
        self.issue_keys
            .iter()
            .map(|key| litradar_storage::DeliveryRunItemCreate {
                item_kind: litradar_storage::DeliveryItemKind::Issue,
                item_key: key.clone(),
                user_id: None,
                article_id: None,
            })
            .chain(
                self.inpress_keys
                    .iter()
                    .map(|key| litradar_storage::DeliveryRunItemCreate {
                        item_kind: litradar_storage::DeliveryItemKind::InPress,
                        item_key: key.clone(),
                        user_id: None,
                        article_id: None,
                    }),
            )
            .chain(self.article_ids.iter().map(|article_id| {
                litradar_storage::DeliveryRunItemCreate {
                    item_kind: litradar_storage::DeliveryItemKind::Article,
                    item_key: article_id.to_string(),
                    user_id: None,
                    article_id: Some(*article_id),
                }
            }))
            .collect()
    }
}

fn delivery_input_from_source(
    manifest: Option<litradar_recommend::ChangeManifest>,
    previous_snapshot: &RecommendationSnapshot,
    current_snapshot: &RecommendationSnapshot,
) -> DeliveryInput {
    match manifest {
        Some(manifest) => DeliveryInput {
            issue_keys: manifest.pending_issue_keys,
            inpress_keys: manifest.pending_inpress_keys,
            article_ids: manifest.pending_article_ids,
        },
        None => DeliveryInput {
            issue_keys: compute_changed_issue_keys(
                &previous_snapshot.issue_article_counts,
                &current_snapshot.issue_article_counts,
            ),
            inpress_keys: compute_changed_inpress_keys(
                &previous_snapshot.inpress_article_counts,
                &current_snapshot.inpress_article_counts,
            ),
            article_ids: Vec::new(),
        },
    }
}

fn delivery_input_from_items(items: &[litradar_storage::DeliveryRunItemRecord]) -> DeliveryInput {
    let mut issue_keys = Vec::new();
    let mut inpress_keys = Vec::new();
    let mut article_ids = Vec::new();
    for item in items {
        match item.item_kind {
            litradar_storage::DeliveryItemKind::Issue => issue_keys.push(item.item_key.clone()),
            litradar_storage::DeliveryItemKind::InPress => {
                inpress_keys.push(item.item_key.clone());
            }
            litradar_storage::DeliveryItemKind::Article => {
                if let Some(article_id) = item.article_id {
                    article_ids.push(article_id);
                }
            }
            litradar_storage::DeliveryItemKind::Subscriber => {}
        }
    }
    issue_keys.sort();
    issue_keys.dedup();
    inpress_keys.sort();
    inpress_keys.dedup();
    article_ids.sort_unstable();
    article_ids.dedup();
    DeliveryInput {
        issue_keys,
        inpress_keys,
        article_ids,
    }
}

fn load_durable_candidates(
    config: &RecommendationRunConfig,
    input: &DeliveryInput,
) -> Result<Vec<ArticleCandidateInfo>, DeliveryError> {
    if !input.article_ids.is_empty() {
        return Ok(litradar_storage::fetch_candidates_for_article_ids(
            &config.index_db_path,
            &input.article_ids,
        )?);
    }
    let mut candidates = litradar_storage::fetch_candidates_for_issue_keys(
        &config.index_db_path,
        &input.issue_keys,
    )?;
    candidates.extend(litradar_storage::fetch_candidates_for_inpress_keys(
        &config.index_db_path,
        &input.inpress_keys,
    )?);
    Ok(candidates)
}

fn finalize_progress_items(
    config: &RecommendationRunConfig,
    context: &mut DurableDeliveryRun,
) -> Result<(), DeliveryError> {
    context.renew(&config.auth_db_path)?;
    let items = litradar_storage::list_delivery_run_items(&config.auth_db_path, context.run.id)?;
    for item in items {
        if item.item_kind == litradar_storage::DeliveryItemKind::Subscriber
            || item.status.is_terminal()
        {
            continue;
        }
        let claimed = litradar_storage::claim_delivery_run_item(
            &config.auth_db_path,
            context.run.id,
            &context.owner_id,
            context.run.revision,
            item.id,
            &context.owner_id,
            unix_now(),
            DELIVERY_LEASE_SECONDS,
        )?;
        litradar_storage::finalize_delivery_run_item(
            &config.auth_db_path,
            claimed.id,
            &context.owner_id,
            claimed.revision,
            litradar_storage::DeliveryItemStatus::Succeeded,
            None,
            None,
            unix_now(),
        )?;
    }
    Ok(())
}

fn subscriber_id_value(subscriber: &NotificationSubscriberInfo) -> Result<i64, DeliveryError> {
    subscriber.subscriber_id.parse::<i64>().map_err(|_| {
        DeliveryError::Manual("Subscriber identifier is not a positive integer".to_string())
    })
}

#[derive(Deserialize)]
struct PersistedSubscriberResult {
    #[serde(default)]
    selected_article_ids: Vec<i64>,
    #[serde(default)]
    folder_synced_count: usize,
    #[serde(default)]
    message_id: Option<String>,
}

fn plan_from_terminal_item(
    config: &RecommendationRunConfig,
    subscriber: &NotificationSubscriberInfo,
    item: &litradar_storage::DeliveryRunItemRecord,
) -> Result<SubscriberDeliveryPlan, DeliveryError> {
    let mut result = item
        .result_json
        .as_deref()
        .map(serde_json::from_str::<PersistedSubscriberResult>)
        .transpose()
        .map_err(|_| DeliveryError::Manual("Stored subscriber result is invalid".to_string()))?
        .unwrap_or(PersistedSubscriberResult {
            selected_article_ids: Vec::new(),
            folder_synced_count: 0,
            message_id: None,
        });
    if result.selected_article_ids.is_empty()
        && item.status == litradar_storage::DeliveryItemStatus::Unknown
    {
        let dedupe = litradar_storage::list_delivery_dedupe_for_scope(
            &config.auth_db_path,
            storage_workflow(config.workflow),
            &config.db_name,
        )?;
        result.selected_article_ids = dedupe
            .iter()
            .filter(|row| {
                row.delivery_run_id == Some(item.delivery_run_id)
                    && row.user_id == item.user_id.unwrap_or_default()
            })
            .map(|row| row.article_id)
            .collect();
        result.selected_article_ids.sort_unstable();
        result.selected_article_ids.dedup();
        result.message_id = dedupe.into_iter().find_map(|row| row.message_id);
    }
    let status = match item.status {
        litradar_storage::DeliveryItemStatus::Succeeded => SubscriberDeliveryState::Ok,
        litradar_storage::DeliveryItemStatus::Skipped => SubscriberDeliveryState::Skipped,
        litradar_storage::DeliveryItemStatus::Unknown => SubscriberDeliveryState::Unknown,
        litradar_storage::DeliveryItemStatus::Failed
        | litradar_storage::DeliveryItemStatus::Cancelled => SubscriberDeliveryState::Error,
        litradar_storage::DeliveryItemStatus::Pending
        | litradar_storage::DeliveryItemStatus::Claimed
        | litradar_storage::DeliveryItemStatus::Sending => {
            return Err(DeliveryError::Manual(
                "Subscriber item is not terminal".to_string(),
            ));
        }
    };
    let favorite_writes = favorite_writes(config, subscriber, &result.selected_article_ids);
    if result.folder_synced_count == 0
        && item.status == litradar_storage::DeliveryItemStatus::Unknown
    {
        result.folder_synced_count = favorite_writes.len();
    }
    Ok(SubscriberDeliveryPlan {
        subscriber_id: subscriber.subscriber_id.clone(),
        delivery_method: subscriber.delivery_method.clone(),
        status,
        error: item.error_code.clone(),
        selected_article_ids: result.selected_article_ids.clone(),
        message_title: None,
        message_content: None,
        message_id: result.message_id,
        favorite_writes,
        folder_synced_count: result.folder_synced_count,
        would_send_pushplus: false,
    })
}

fn run_result_json(
    candidate_count: usize,
    subscriber_count: usize,
    selected_count: usize,
    message_count: usize,
) -> Result<String, DeliveryError> {
    serde_json::to_string(&serde_json::json!({
        "candidate_count": candidate_count,
        "subscriber_count": subscriber_count,
        "selected_count": selected_count,
        "message_count": message_count,
    }))
    .map_err(|_| DeliveryError::Manual("Delivery result serialization failed".to_string()))
}

fn filtered_subscribers(
    auth_db_path: &Path,
    secret_codec: &litradar_storage::SecretCodec,
    db_name: &str,
    workflow: DeliveryWorkflow,
    subscriber_user_id: Option<UserId>,
) -> Result<Vec<NotificationSubscriberInfo>, DeliveryError> {
    let subscribers = match subscriber_user_id {
        Some(user_id) => {
            litradar_storage::get_notification_subscriber(auth_db_path, secret_codec, user_id)?
                .into_iter()
                .collect()
        }
        None => litradar_storage::list_notification_subscribers(auth_db_path, secret_codec)?,
    };
    Ok(subscribers
        .into_iter()
        .filter(|subscriber| is_database_selected(&subscriber.selected_databases, db_name))
        .filter(|subscriber| match workflow {
            DeliveryWorkflow::Notify => {
                subscriber.delivery_method == "pushplus"
                    && !subscriber.pushplus_token.trim().is_empty()
            }
            DeliveryWorkflow::Push => {
                subscriber.delivery_method == "folder" && subscriber.tracking_folder_id.is_some()
            }
        })
        .collect())
}

fn emit_delivery_workflow_terminal(
    result: &Result<RecommendationRunOutcome, DeliveryError>,
    started_at: Instant,
) {
    match result {
        Ok(outcome) => {
            let subscriber_count = outcome.subscribers.len();
            let selected_count = outcome
                .subscribers
                .iter()
                .map(|subscriber| subscriber.selected_article_ids.len())
                .sum::<usize>();
            let folder_synced_count = outcome
                .subscribers
                .iter()
                .map(|subscriber| subscriber.folder_synced_count)
                .sum::<usize>();
            let message_count = outcome
                .subscribers
                .iter()
                .filter(|subscriber| subscriber.message_id.is_some())
                .count();
            let failed_subscriber_count = outcome
                .subscribers
                .iter()
                .filter(|subscriber| subscriber.status == SubscriberDeliveryState::Error)
                .count();
            if matches!(
                outcome.status,
                DeliveryOutcomeState::Failed | DeliveryOutcomeState::Unknown
            ) {
                tracing::warn!(
                    event = "delivery.workflow.failed",
                    component = "delivery",
                    outcome = "failure",
                    status = outcome.status.as_str(),
                    candidate_count = outcome.candidate_article_ids.len(),
                    subscriber_count,
                    selected_count,
                    folder_synced_count,
                    message_count,
                    failed_subscriber_count,
                    duration_ms = elapsed_millis(started_at),
                );
            } else {
                tracing::info!(
                    event = "delivery.workflow.completed",
                    component = "delivery",
                    outcome = "success",
                    status = outcome.status.as_str(),
                    candidate_count = outcome.candidate_article_ids.len(),
                    subscriber_count,
                    selected_count,
                    folder_synced_count,
                    message_count,
                    failed_subscriber_count,
                    duration_ms = elapsed_millis(started_at),
                );
            }
        }
        Err(error) => tracing::warn!(
            event = "delivery.workflow.failed",
            component = "delivery",
            outcome = "failure",
            status = "error",
            error_kind = delivery_error_kind(error),
            duration_ms = elapsed_millis(started_at),
        ),
    }
}

fn emit_manual_delivery_terminal(
    result: &Result<ManualWeeklyPushOutcome, DeliveryError>,
    started_at: Instant,
) {
    match result {
        Ok(outcome)
            if matches!(
                outcome.status,
                ManualPushState::Failed | ManualPushState::Unknown
            ) =>
        {
            tracing::warn!(
                event = "delivery.manual.failed",
                component = "delivery",
                outcome = "failure",
                status = outcome.status.as_str(),
                selected_count = outcome.selected,
                delivered_count = outcome.pushed,
                candidate_count = outcome.total_candidates.unwrap_or(0),
                duration_ms = elapsed_millis(started_at),
            )
        }
        Ok(outcome) => tracing::info!(
            event = "delivery.manual.completed",
            component = "delivery",
            outcome = "success",
            status = outcome.status.as_str(),
            selected_count = outcome.selected,
            delivered_count = outcome.pushed,
            candidate_count = outcome.total_candidates.unwrap_or(0),
            duration_ms = elapsed_millis(started_at),
        ),
        Err(error) => tracing::warn!(
            event = "delivery.manual.failed",
            component = "delivery",
            outcome = "failure",
            status = "error",
            error_kind = delivery_error_kind(error),
            duration_ms = elapsed_millis(started_at),
        ),
    }
}

fn delivery_error_kind(error: &DeliveryError) -> &'static str {
    match error {
        DeliveryError::Index(_) => "index_storage",
        DeliveryError::Business(_) => "business_storage",
        DeliveryError::Durable(_) => "delivery_storage",
        DeliveryError::Auth(_) => "auth_storage",
        DeliveryError::Recommendation(_) => "recommendation",
        DeliveryError::Ai(_) => "ai",
        DeliveryError::PushPlus(_) => "pushplus",
        DeliveryError::Manual(_) => "manual_validation",
        DeliveryError::Busy => "busy",
        DeliveryError::Control(error) => error.as_str(),
    }
}

fn delivery_workflow_kind(workflow: DeliveryWorkflow) -> &'static str {
    match workflow {
        DeliveryWorkflow::Notify => "notify",
        DeliveryWorkflow::Push => "push",
    }
}

fn delivery_mode_kind(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::DryRun => "dry_run",
        DeliveryMode::Execute => "execute",
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn manual_outcome(
    status: ManualPushState,
    message: &str,
    folder_id: Option<i64>,
    folder_name: Option<String>,
) -> ManualWeeklyPushOutcome {
    ManualWeeklyPushOutcome {
        status,
        message: message.to_string(),
        pushed: 0,
        selected: 0,
        total_candidates: None,
        summary: String::new(),
        folder_id,
        folder_name,
    }
}

fn manual_outcome_from_delivery(
    delivery_method: &str,
    folder_id: Option<i64>,
    folder_name: Option<String>,
    outcomes: &[RecommendationRunOutcome],
) -> ManualWeeklyPushOutcome {
    let mut pushed = 0_i64;
    let mut selected = 0_i64;
    let mut total_candidates = 0_i64;
    let mut pushplus_messages = 0_i64;
    let mut selected_databases = BTreeSet::new();
    let mut errors = Vec::new();
    let mut has_unknown = false;
    let mut skip_messages = Vec::new();

    for outcome in outcomes {
        total_candidates += outcome.candidate_article_ids.len() as i64;
        if outcome.status == DeliveryOutcomeState::Failed {
            errors.push(format!("{} delivery failed", outcome.db_name));
        } else if outcome.status == DeliveryOutcomeState::Unknown {
            has_unknown = true;
            errors.push(format!("{} delivery outcome is unknown", outcome.db_name));
        }
        for subscriber in &outcome.subscribers {
            selected += subscriber.selected_article_ids.len() as i64;
            pushed += subscriber.folder_synced_count as i64;
            if subscriber.message_id.is_some() {
                pushplus_messages += 1;
            }
            if !subscriber.selected_article_ids.is_empty() {
                selected_databases.insert(outcome.db_name.clone());
            }
            if let Some(error) = &subscriber.error {
                skip_messages.push(error.clone());
            }
        }
    }

    if !errors.is_empty() {
        return ManualWeeklyPushOutcome {
            status: if has_unknown {
                ManualPushState::Unknown
            } else {
                ManualPushState::Failed
            },
            message: errors.join("; "),
            pushed,
            selected,
            total_candidates: Some(total_candidates),
            summary: String::new(),
            folder_id,
            folder_name,
        };
    }

    let message = if selected > 0 && delivery_method == "pushplus" {
        let message_suffix = if pushplus_messages == 1 { "" } else { "s" };
        let article_suffix = if selected == 1 { "" } else { "s" };
        let database_suffix = if selected_databases.len() == 1 {
            ""
        } else {
            "s"
        };
        let mut message = format!(
            "PushPlus sent successfully ({pushplus_messages} message{message_suffix}); selected {selected} article{article_suffix} across {} database{database_suffix}",
            selected_databases.len()
        );
        if pushed > 0 {
            let synced_suffix = if pushed == 1 { "" } else { "s" };
            message.push_str(&format!(
                "; synced {pushed} article{synced_suffix} to the tracking folder"
            ));
        }
        message
    } else if selected == 0 {
        skip_messages
            .into_iter()
            .next()
            .unwrap_or_else(|| "AI selection found no matching articles".to_string())
    } else {
        String::new()
    };

    ManualWeeklyPushOutcome {
        status: ManualPushState::Completed,
        message,
        pushed,
        selected,
        total_candidates: Some(total_candidates),
        summary: String::new(),
        folder_id,
        folder_name,
    }
}

fn nonempty_text(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[allow(clippy::too_many_arguments)]
fn process_subscriber(
    config: &RecommendationRunConfig,
    context: &DurableDeliveryRun,
    item: &litradar_storage::DeliveryRunItemRecord,
    subscriber: &NotificationSubscriberInfo,
    global_config: &NotificationGlobalConfig,
    defaults: &NotificationDefaults,
    run_label: &str,
    candidates_for_model: &[ArticleCandidateInfo],
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &mut BTreeMap<String, String>,
    ai_selector: &mut impl DeliveryAiSelector,
    pushplus_sender: &mut impl DeliveryPushPlusSender,
) -> Result<SubscriberDeliveryPlan, DeliveryError> {
    let mut plan = match build_subscriber_plan(
        SubscriberPlanRequest {
            config,
            subscriber,
            global_config,
            defaults,
            run_id: run_label,
            candidates_for_model,
            candidates_by_id,
            delivery_dedupe,
        },
        ai_selector,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            let result_json = subscriber_result_json(&[], 0, None)?;
            litradar_storage::finalize_delivery_run_item(
                &config.auth_db_path,
                item.id,
                &context.owner_id,
                item.revision,
                litradar_storage::DeliveryItemStatus::Failed,
                Some(&result_json),
                Some("selection_failed"),
                unix_now(),
            )?;
            return Ok(failed_plan(subscriber, &error.to_string()));
        }
    };
    if plan.status == SubscriberDeliveryState::Skipped {
        let result_json = subscriber_result_json(&[], 0, None)?;
        litradar_storage::finalize_delivery_run_item(
            &config.auth_db_path,
            item.id,
            &context.owner_id,
            item.revision,
            litradar_storage::DeliveryItemStatus::Skipped,
            Some(&result_json),
            None,
            unix_now(),
        )?;
        return Ok(plan);
    }
    if config.mode == DeliveryMode::DryRun {
        let result_json =
            subscriber_result_json(&plan.selected_article_ids, plan.folder_synced_count, None)?;
        litradar_storage::finalize_delivery_run_item(
            &config.auth_db_path,
            item.id,
            &context.owner_id,
            item.revision,
            litradar_storage::DeliveryItemStatus::Succeeded,
            Some(&result_json),
            None,
            unix_now(),
        )?;
        return Ok(plan);
    }

    check_execution_control(config)?;

    let user_id = subscriber_id_value(subscriber)?;
    let mut reservations = Vec::new();
    for article_id in &plan.selected_article_ids {
        match litradar_storage::reserve_delivery_dedupe(
            &config.auth_db_path,
            context.workflow,
            &config.db_name,
            user_id,
            *article_id,
            context.run.id,
            &context.owner_id,
            unix_now(),
        )? {
            litradar_storage::DeliveryDedupeReserveOutcome::Reserved(record) => {
                reservations.push(record);
            }
            litradar_storage::DeliveryDedupeReserveOutcome::Existing(_) => {
                release_reservations(config, context, &reservations)?;
                let skipped = skipped_plan(subscriber, "Articles were already delivered");
                let result_json = subscriber_result_json(&[], 0, None)?;
                litradar_storage::finalize_delivery_run_item(
                    &config.auth_db_path,
                    item.id,
                    &context.owner_id,
                    item.revision,
                    litradar_storage::DeliveryItemStatus::Skipped,
                    Some(&result_json),
                    None,
                    unix_now(),
                )?;
                return Ok(skipped);
            }
        }
    }
    if let Err(error) = execute_favorite_writes(config, &plan.favorite_writes) {
        release_reservations(config, context, &reservations)?;
        let result_json =
            subscriber_result_json(&plan.selected_article_ids, plan.folder_synced_count, None)?;
        litradar_storage::finalize_delivery_run_item(
            &config.auth_db_path,
            item.id,
            &context.owner_id,
            item.revision,
            litradar_storage::DeliveryItemStatus::Failed,
            Some(&result_json),
            Some("favorite_write_failed"),
            unix_now(),
        )?;
        plan.status = SubscriberDeliveryState::Error;
        plan.error = Some(error.to_string());
        return Ok(plan);
    }

    let resolutions = reservations
        .iter()
        .map(|record| litradar_storage::DeliveryDedupeResolution {
            id: record.id,
            expected_revision: record.revision,
        })
        .collect::<Vec<_>>();
    if config.workflow == DeliveryWorkflow::Notify {
        check_execution_control(config)?;
        let sending = litradar_storage::mark_delivery_run_item_sending(
            &config.auth_db_path,
            item.id,
            &context.owner_id,
            item.revision,
            unix_now(),
        )?;
        let title = plan
            .message_title
            .as_deref()
            .ok_or_else(|| DeliveryError::PushPlus("PushPlus title is unavailable".into()))?;
        let content = plan
            .message_content
            .as_deref()
            .ok_or_else(|| DeliveryError::PushPlus("PushPlus content is unavailable".into()))?;
        match pushplus_sender.send(&pushplus_message(subscriber, global_config, title, content)) {
            Ok(message_id) => {
                plan.message_id = Some(message_id.clone());
                let result_json = subscriber_result_json(
                    &plan.selected_article_ids,
                    plan.folder_synced_count,
                    Some(&message_id),
                )?;
                litradar_storage::finalize_delivery_attempt(
                    &config.auth_db_path,
                    sending.id,
                    &context.owner_id,
                    sending.revision,
                    litradar_storage::DeliveryItemStatus::Succeeded,
                    Some(&result_json),
                    None,
                    context.run.id,
                    &resolutions,
                    litradar_storage::DeliveryDedupeStatus::Confirmed,
                    Some(&message_id),
                    unix_now(),
                )?;
            }
            Err(error) => {
                let result_json = subscriber_result_json(
                    &plan.selected_article_ids,
                    plan.folder_synced_count,
                    None,
                )?;
                litradar_storage::finalize_delivery_attempt(
                    &config.auth_db_path,
                    sending.id,
                    &context.owner_id,
                    sending.revision,
                    litradar_storage::DeliveryItemStatus::Unknown,
                    Some(&result_json),
                    Some("ambiguous_delivery"),
                    context.run.id,
                    &resolutions,
                    litradar_storage::DeliveryDedupeStatus::Unknown,
                    None,
                    unix_now(),
                )?;
                plan.status = SubscriberDeliveryState::Unknown;
                plan.error = Some(error.to_string());
                return Ok(plan);
            }
        }
    } else {
        let result_json =
            subscriber_result_json(&plan.selected_article_ids, plan.folder_synced_count, None)?;
        litradar_storage::finalize_delivery_attempt(
            &config.auth_db_path,
            item.id,
            &context.owner_id,
            item.revision,
            litradar_storage::DeliveryItemStatus::Succeeded,
            Some(&result_json),
            None,
            context.run.id,
            &resolutions,
            litradar_storage::DeliveryDedupeStatus::Confirmed,
            None,
            unix_now(),
        )?;
    }
    for article_id in &plan.selected_article_ids {
        delivery_dedupe.insert(
            format!("{}:{article_id}", subscriber.subscriber_id),
            utc_now_iso(),
        );
    }
    Ok(plan)
}

fn release_reservations(
    config: &RecommendationRunConfig,
    context: &DurableDeliveryRun,
    reservations: &[litradar_storage::DeliveryDedupeRecord],
) -> Result<(), DeliveryError> {
    litradar_storage::release_delivery_dedupe_reservations(
        &config.auth_db_path,
        context.run.id,
        &context.owner_id,
        &reservations
            .iter()
            .map(|record| litradar_storage::DeliveryDedupeResolution {
                id: record.id,
                expected_revision: record.revision,
            })
            .collect::<Vec<_>>(),
    )?;
    Ok(())
}

fn subscriber_result_json(
    selected_article_ids: &[i64],
    folder_synced_count: usize,
    message_id: Option<&str>,
) -> Result<String, DeliveryError> {
    serde_json::to_string(&serde_json::json!({
        "selected_article_ids": selected_article_ids,
        "folder_synced_count": folder_synced_count,
        "message_id": message_id,
    }))
    .map_err(|_| DeliveryError::Manual("Subscriber result serialization failed".to_string()))
}

struct SubscriberPlanRequest<'a> {
    config: &'a RecommendationRunConfig,
    subscriber: &'a NotificationSubscriberInfo,
    global_config: &'a NotificationGlobalConfig,
    defaults: &'a NotificationDefaults,
    run_id: &'a str,
    candidates_for_model: &'a [ArticleCandidateInfo],
    candidates_by_id: &'a BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &'a BTreeMap<String, String>,
}

fn build_subscriber_plan(
    request: SubscriberPlanRequest<'_>,
    ai_selector: &mut impl DeliveryAiSelector,
) -> Result<SubscriberDeliveryPlan, DeliveryError> {
    let SubscriberPlanRequest {
        config,
        subscriber,
        global_config,
        defaults,
        candidates_for_model,
        candidates_by_id,
        delivery_dedupe,
        run_id,
    } = request;
    let selection = ai_selector.select_for_subscriber(DeliveryAiSelectionRequest {
        subscriber,
        global_config,
        defaults,
        override_model: config.ai_model.as_deref(),
        candidates_for_model,
        candidates_by_id,
        delivery_dedupe,
    })?;
    if let Some(reason) = selection.skip_reason {
        return Ok(skipped_plan(subscriber, &reason));
    }
    if selection.accepted.is_empty() {
        return Ok(skipped_plan(
            subscriber,
            "AI selection found no matching articles",
        ));
    }
    let selected_article_ids = selection
        .accepted
        .iter()
        .map(|selection| selection.article_id)
        .collect::<Vec<_>>();
    let favorite_writes = favorite_writes(config, subscriber, &selected_article_ids);
    let (message_title, message_content, would_send_pushplus) =
        if config.workflow == DeliveryWorkflow::Notify {
            (
                Some(build_message_title(&config.db_name, run_id)),
                Some(build_markdown_content(
                    &config.db_name,
                    run_id,
                    subscriber,
                    &selection.summary,
                    &selection.accepted,
                    candidates_by_id,
                )),
                true,
            )
        } else {
            (None, None, false)
        };
    Ok(SubscriberDeliveryPlan {
        subscriber_id: subscriber.subscriber_id.clone(),
        delivery_method: subscriber.delivery_method.clone(),
        status: SubscriberDeliveryState::Ok,
        error: None,
        selected_article_ids,
        message_title,
        message_content,
        message_id: None,
        folder_synced_count: favorite_writes.len(),
        favorite_writes,
        would_send_pushplus,
    })
}

fn failed_plan(subscriber: &NotificationSubscriberInfo, reason: &str) -> SubscriberDeliveryPlan {
    SubscriberDeliveryPlan {
        subscriber_id: subscriber.subscriber_id.clone(),
        delivery_method: subscriber.delivery_method.clone(),
        status: SubscriberDeliveryState::Error,
        error: Some(reason.to_string()),
        selected_article_ids: Vec::new(),
        message_title: None,
        message_content: None,
        message_id: None,
        favorite_writes: Vec::new(),
        folder_synced_count: 0,
        would_send_pushplus: false,
    }
}

fn skipped_plan(subscriber: &NotificationSubscriberInfo, reason: &str) -> SubscriberDeliveryPlan {
    SubscriberDeliveryPlan {
        subscriber_id: subscriber.subscriber_id.clone(),
        delivery_method: subscriber.delivery_method.clone(),
        status: SubscriberDeliveryState::Skipped,
        error: Some(reason.to_string()),
        selected_article_ids: Vec::new(),
        message_title: None,
        message_content: None,
        message_id: None,
        favorite_writes: Vec::new(),
        folder_synced_count: 0,
        would_send_pushplus: false,
    }
}

fn outcome(
    config: &RecommendationRunConfig,
    delivery_run_id: i64,
    status: DeliveryOutcomeState,
    candidate_article_ids: Vec<i64>,
    subscribers: Vec<SubscriberDeliveryPlan>,
) -> RecommendationRunOutcome {
    RecommendationRunOutcome {
        db_name: config.db_name.clone(),
        workflow: config.workflow,
        mode: config.mode,
        status,
        delivery_run_id,
        candidate_article_ids,
        subscribers,
    }
}

fn load_global_config(
    config: &RecommendationRunConfig,
) -> Result<NotificationGlobalConfig, DeliveryError> {
    Ok(NotificationGlobalConfig {
        ai_base_url: litradar_storage::canonicalize_outbound_base_url(DEFAULT_OPENAI_BASE_URL)?,
        ai_allowed_base_urls: litradar_storage::load_ai_allowed_base_urls(&config.auth_db_path)?,
        ai_api_key: String::new(),
        pushplus_channel: PUSHPLUS_CHANNEL.to_string(),
        pushplus_template: "markdown".to_string(),
        pushplus_topic: None,
        pushplus_option: None,
        ai_system_prompt: None,
    })
}

fn load_defaults() -> NotificationDefaults {
    NotificationDefaults {
        max_candidates: 120,
        ai_model: DEFAULT_OPENAI_MODEL.to_string(),
        temperature: 0.2,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use litradar_domain::{NotificationSettingsUpdate, RankedSelectionInfo, UserId};
    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::ai::test_support::CapturedLogs;
    use crate::delivery::candidates::{
        AiSelectionOutcome, DeliveryAiSelectionRequest, DeliveryAiSelector,
    };
    use crate::delivery::notify::DeliveryPushPlusSender;
    use crate::pushplus::PushPlusMessage;

    #[test]
    fn dry_run_push_plans_folder_writes_without_side_effects() {
        let fixture = DeliveryFixture::new(notification_settings("folder", true, vec![]));

        let (outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(DeliveryWorkflow::Push, DeliveryMode::DryRun, None, None),
            vec![selection_outcome(&[101, 102], "")],
            Vec::new(),
        )
        .expect("push dry-run should build a plan");

        assert_eq!(outcome.status, DeliveryOutcomeState::Completed);
        assert_eq!(outcome.candidate_article_ids, vec![102, 101]);
        assert_eq!(outcome.subscribers.len(), 1);
        let plan = &outcome.subscribers[0];
        assert_eq!(plan.status, SubscriberDeliveryState::Ok);
        assert_eq!(plan.selected_article_ids, vec![101, 102]);
        assert_eq!(plan.folder_synced_count, 2);
        assert_eq!(plan.favorite_writes.len(), 2);
        assert!(!plan.would_send_pushplus);
        assert_eq!(favorite_count(&fixture.auth_db_path), 0);
        let checkpoint = delivery_checkpoint(&fixture, DeliveryWorkflow::Push);
        assert_eq!(
            checkpoint.status,
            litradar_storage::DeliveryCheckpointStatus::Completed
        );
        assert!(delivery_dedupe(&fixture, DeliveryWorkflow::Push).is_empty());
        assert!(!fixture.root.path().join("state/fixture.json").exists());
    }

    #[test]
    fn dry_run_notify_plans_pushplus_without_sending() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));

        let (outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(
                DeliveryWorkflow::Notify,
                DeliveryMode::DryRun,
                None,
                Some(1),
            ),
            vec![selection_outcome(&[102], "AI summary")],
            Vec::new(),
        )
        .expect("notify dry-run should build a PushPlus plan");

        assert_eq!(outcome.status, DeliveryOutcomeState::Completed);
        assert_eq!(outcome.subscribers.len(), 1);
        let plan = &outcome.subscribers[0];
        assert_eq!(plan.status, SubscriberDeliveryState::Ok);
        assert_eq!(plan.selected_article_ids, vec![102]);
        assert_eq!(plan.folder_synced_count, 1);
        assert!(plan.would_send_pushplus);
        assert!(plan
            .message_title
            .as_deref()
            .expect("title should be planned")
            .contains("fixture.sqlite"));
        assert!(plan
            .message_content
            .as_deref()
            .expect("content should be planned")
            .contains("Rust migration"));
        assert!(plan
            .message_content
            .as_deref()
            .expect("content should be planned")
            .contains("AI summary"));
        assert_eq!(favorite_count(&fixture.auth_db_path), 0);
    }

    #[test]
    fn delivery_aggregate_events_omit_user_article_and_message_content() {
        let sentinel = "delivery-preference-article-message-sentinel";
        let mut settings = notification_settings("pushplus", true, vec![]);
        settings.keywords = vec![sentinel.to_string()];
        settings.directions = vec![sentinel.to_string()];
        settings.pushplus_token = Some(Some(sentinel.to_string()));
        settings.ai_api_key = Some(Some(sentinel.to_string()));
        settings.ai_system_prompt = sentinel.to_string();
        let fixture = DeliveryFixture::new(settings);
        let logs = CapturedLogs::default();

        let (outcome, pushplus_sender) = logs
            .capture(|| {
                run_fixture_delivery(
                    &fixture.config(DeliveryWorkflow::Notify, DeliveryMode::DryRun, None, None),
                    vec![selection_outcome(&[101], sentinel)],
                    Vec::new(),
                )
            })
            .expect("dry-run delivery should complete");

        assert_eq!(outcome.status, DeliveryOutcomeState::Completed);
        assert!(pushplus_sender.messages.is_empty());
        let events = logs.events();
        let completed = events
            .iter()
            .find(|event| event["event"] == "delivery.workflow.completed")
            .expect("delivery aggregate should be logged");
        assert_eq!(completed["candidate_count"], 2);
        assert_eq!(completed["subscriber_count"], 1);
        assert_eq!(completed["selected_count"], 1);
        assert_eq!(completed["span"]["workflow"], "notify");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "delivery.workflow.completed")
                .count(),
            1
        );
        assert!(!logs.text().contains(sentinel));
    }

    #[test]
    fn execute_notify_sends_pushplus_and_records_message_id() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));

        let (outcome, pushplus_sender) = run_fixture_delivery(
            &fixture.config(DeliveryWorkflow::Notify, DeliveryMode::Execute, None, None),
            vec![selection_outcome(&[101, 102], "")],
            vec![Ok("msg-1".to_string())],
        )
        .expect("notify execute should send PushPlus");

        assert_eq!(outcome.status, DeliveryOutcomeState::Completed);
        assert_eq!(outcome.subscribers[0].message_id.as_deref(), Some("msg-1"));
        assert_eq!(pushplus_sender.messages.len(), 1);
        assert_eq!(pushplus_sender.messages[0].token, "token");
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let items = litradar_storage::list_delivery_run_items(
            &fixture.auth_db_path,
            outcome.delivery_run_id,
        )
        .expect("run items should load");
        let subscriber_item = items
            .iter()
            .find(|item| item.item_kind == litradar_storage::DeliveryItemKind::Subscriber)
            .expect("subscriber item should exist");
        assert_eq!(
            subscriber_item.status,
            litradar_storage::DeliveryItemStatus::Succeeded
        );
        assert!(subscriber_item
            .result_json
            .as_deref()
            .is_some_and(|result| result.contains("msg-1")));
        let dedupe = delivery_dedupe(&fixture, DeliveryWorkflow::Notify);
        assert_eq!(dedupe.len(), 2);
        assert!(dedupe.iter().all(|row| {
            row.status == litradar_storage::DeliveryDedupeStatus::Confirmed
                && row.message_id.as_deref() == Some("msg-1")
        }));
    }

    #[test]
    fn execute_notify_pushplus_failure_persists_unknown_without_replay() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));

        let (outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(DeliveryWorkflow::Notify, DeliveryMode::Execute, None, None),
            vec![selection_outcome(&[101, 102], "")],
            vec![Err(DeliveryError::PushPlus("send failed".to_string()))],
        )
        .expect("notify execute should record PushPlus failure");

        assert_eq!(outcome.status, DeliveryOutcomeState::Unknown);
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let dedupe = delivery_dedupe(&fixture, DeliveryWorkflow::Notify);
        assert_eq!(dedupe.len(), 2);
        assert!(dedupe
            .iter()
            .all(|row| row.status == litradar_storage::DeliveryDedupeStatus::Unknown));
        let run =
            litradar_storage::load_delivery_run(&fixture.auth_db_path, outcome.delivery_run_id)
                .expect("run should load")
                .expect("run should exist");
        assert_eq!(run.status, litradar_storage::DeliveryRunStatus::Unknown);
        assert!(!format!("{run:?}").contains("send failed"));
    }

    #[test]
    fn execute_push_writes_folder_state_and_dedupe() {
        let fixture = DeliveryFixture::new(notification_settings("folder", true, vec![]));

        let (outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(DeliveryWorkflow::Push, DeliveryMode::Execute, None, None),
            vec![selection_outcome(&[101, 102], "")],
            Vec::new(),
        )
        .expect("push execute should write favorites");

        assert_eq!(outcome.status, DeliveryOutcomeState::Completed);
        assert_eq!(outcome.subscribers[0].favorite_writes.len(), 2);
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let dedupe = delivery_dedupe(&fixture, DeliveryWorkflow::Push);
        assert_eq!(dedupe.len(), 2);
        assert!(dedupe
            .iter()
            .any(|row| row.user_id == 1 && row.article_id == 101));
    }

    #[test]
    fn changes_manifest_filters_candidates_and_rejects_wrong_database() {
        let fixture = DeliveryFixture::new(notification_settings("folder", true, vec![]));
        let changes_file = fixture.root.path().join("changes.json");
        fs::write(
            &changes_file,
            r#"{"db_name":"fixture.sqlite","run_id":"manifest-run","changed_issue_keys":["1:11"],"changed_inpress_journal_ids":[],"notifiable_article_ids":[102]}"#,
        )
        .expect("manifest should be written");

        let (outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(
                DeliveryWorkflow::Push,
                DeliveryMode::DryRun,
                Some(changes_file.clone()),
                None,
            ),
            vec![selection_outcome(&[102], "")],
            Vec::new(),
        )
        .expect("manifest run should filter candidates");

        assert_eq!(outcome.candidate_article_ids, vec![102]);
        assert_eq!(outcome.subscribers[0].selected_article_ids, vec![102]);

        fs::write(
            &changes_file,
            r#"{"db_name":"fixture.sqlite","run_id":"article-only","notifiable_article_ids":[101]}"#,
        )
        .expect("article-only manifest should be written");
        let (article_only_outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(
                DeliveryWorkflow::Push,
                DeliveryMode::DryRun,
                Some(changes_file.clone()),
                None,
            ),
            vec![selection_outcome(&[101], "")],
            Vec::new(),
        )
        .expect("article-only manifest run should load candidates");

        assert_eq!(article_only_outcome.candidate_article_ids, vec![101]);
        assert_eq!(
            article_only_outcome.subscribers[0].selected_article_ids,
            vec![101]
        );

        fs::write(
            &changes_file,
            r#"{"db_name":"other.sqlite","changed_issue_keys":["1:11"],"changed_inpress_journal_ids":[],"notifiable_article_ids":[102]}"#,
        )
        .expect("manifest should be replaced");
        let error = run_fixture_delivery(
            &fixture.config(
                DeliveryWorkflow::Push,
                DeliveryMode::DryRun,
                Some(changes_file),
                None,
            ),
            Vec::new(),
            Vec::new(),
        )
        .expect_err("wrong database manifest should be rejected");

        assert!(error.to_string().contains("database mismatch"));
    }

    #[test]
    fn disabled_or_unselected_subscribers_are_skipped() {
        let disabled_fixture = DeliveryFixture::new(notification_settings("folder", false, vec![]));

        let (disabled_outcome, _pushplus_sender) = run_fixture_delivery(
            &disabled_fixture.config(DeliveryWorkflow::Push, DeliveryMode::DryRun, None, None),
            Vec::new(),
            Vec::new(),
        )
        .expect("disabled subscriber run should complete");

        assert_eq!(disabled_outcome.status, DeliveryOutcomeState::Skipped);
        assert!(disabled_outcome.subscribers.is_empty());

        let unselected_fixture = DeliveryFixture::new(notification_settings(
            "folder",
            true,
            vec!["other.sqlite".to_string()],
        ));

        let (unselected_outcome, _pushplus_sender) = run_fixture_delivery(
            &unselected_fixture.config(DeliveryWorkflow::Push, DeliveryMode::DryRun, None, None),
            Vec::new(),
            Vec::new(),
        )
        .expect("unselected database run should complete");

        assert_eq!(unselected_outcome.status, DeliveryOutcomeState::Skipped);
        assert!(unselected_outcome.subscribers.is_empty());
    }

    #[test]
    fn user_scoped_subscriber_loading_isolates_secret_decryption() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));
        let unrelated_user_id = fixture.add_subscriber(
            "unrelated-user",
            notification_settings("pushplus", true, vec![]),
        );
        let disabled_user_id = fixture.add_subscriber(
            "disabled-user",
            notification_settings("pushplus", false, vec![]),
        );
        fixture.corrupt_notification_ai_key(unrelated_user_id);

        let mut ai_selector = FixtureDeliveryAiSelector::new(vec![selection_outcome(&[101], "")]);
        let mut pushplus_sender =
            FixturePushPlusSender::new(vec![Ok("target-message".to_string())]);
        let outcome = run_recommendation_delivery_with_services_for_user(
            &fixture.config(DeliveryWorkflow::Notify, DeliveryMode::Execute, None, None),
            Some(fixture.user_id),
            &mut ai_selector,
            &mut pushplus_sender,
        )
        .expect("healthy target should not decrypt an unrelated subscriber");

        assert_eq!(outcome.status, DeliveryOutcomeState::Completed);
        assert_eq!(outcome.subscribers.len(), 1);
        assert_eq!(
            outcome.subscribers[0].subscriber_id,
            fixture.user_id.value().to_string()
        );
        assert_eq!(
            ai_selector.subscriber_ids,
            vec![fixture.user_id.value().to_string()]
        );
        assert_eq!(pushplus_sender.messages.len(), 1);
        assert_eq!(pushplus_sender.messages[0].token, "token");
        assert_eq!(favorite_count(&fixture.auth_db_path), 1);
        assert_eq!(
            litradar_storage::count_favorites(&fixture.auth_db_path, unrelated_user_id, None)
                .expect("unrelated favorites should be counted"),
            0
        );
        assert!(delivery_dedupe(&fixture, DeliveryWorkflow::Notify)
            .iter()
            .all(|row| row.user_id == fixture.user_id.value()));

        let missing = filtered_subscribers(
            &fixture.auth_db_path,
            &fixture.secret_codec,
            &fixture.db_name,
            DeliveryWorkflow::Notify,
            Some(UserId(i64::MAX)),
        )
        .expect("missing scoped subscriber should not load unrelated rows");
        assert!(missing.is_empty());
        let disabled = filtered_subscribers(
            &fixture.auth_db_path,
            &fixture.secret_codec,
            &fixture.db_name,
            DeliveryWorkflow::Notify,
            Some(disabled_user_id),
        )
        .expect("disabled scoped subscriber should not load unrelated rows");
        assert!(disabled.is_empty());
        assert!(filtered_subscribers(
            &fixture.auth_db_path,
            &fixture.secret_codec,
            &fixture.db_name,
            DeliveryWorkflow::Notify,
            None,
        )
        .is_err());

        let corrupt_manifest = fixture.root.path().join("corrupt-target.changes.json");
        fs::write(
            &corrupt_manifest,
            r#"{"db_name":"fixture.sqlite","run_id":"corrupt-target","notifiable_article_ids":[101]}"#,
        )
        .expect("corrupt-target manifest should be written");
        let corrupt_target_config = fixture.config(
            DeliveryWorkflow::Notify,
            DeliveryMode::Execute,
            Some(corrupt_manifest),
            None,
        );
        let mut corrupt_target_ai_selector =
            FixtureDeliveryAiSelector::new(vec![selection_outcome(&[101], "")]);
        let mut corrupt_target_pushplus_sender =
            FixturePushPlusSender::new(vec![Ok("unexpected-message".to_string())]);
        let target_error = run_recommendation_delivery_with_services_for_user(
            &corrupt_target_config,
            Some(unrelated_user_id),
            &mut corrupt_target_ai_selector,
            &mut corrupt_target_pushplus_sender,
        )
        .expect_err("corrupt target should fail before delivery side effects");

        assert_eq!(
            target_error.to_string(),
            "Stored secret authentication failed"
        );
        assert!(corrupt_target_ai_selector.subscriber_ids.is_empty());
        assert!(corrupt_target_pushplus_sender.messages.is_empty());
        assert_eq!(
            litradar_storage::count_favorites(&fixture.auth_db_path, unrelated_user_id, None)
                .expect("corrupt target favorites should be counted"),
            0
        );
        let connection = litradar_storage::open_sqlite_connection(&fixture.auth_db_path)
            .expect("auth database should open");
        let corrupt_run_status: String = connection
            .query_row(
                "SELECT status FROM delivery_runs
                 WHERE user_id = ?1 AND scope_key = 'fixture.sqlite'
                 ORDER BY id DESC LIMIT 1",
                [unrelated_user_id.value()],
                |row| row.get(0),
            )
            .expect("corrupt target run should persist");
        assert_eq!(corrupt_run_status, "failed");
        assert_eq!(
            delivery_checkpoint(&fixture, DeliveryWorkflow::Notify).status,
            litradar_storage::DeliveryCheckpointStatus::Failed
        );
    }

    #[test]
    fn two_process_delivery_allows_exactly_one_workflow_owner() {
        let fixture = DeliveryFixture::new(notification_settings("folder", true, vec![]));
        let listener = TcpListener::bind("127.0.0.1:0").expect("process listener should bind");
        listener
            .set_nonblocking(true)
            .expect("process listener should be nonblocking");
        let address = listener
            .local_addr()
            .expect("process listener address should resolve");
        let mut owner = spawn_delivery_process(&fixture, "owner", address);
        let mut owner_stream = accept_process_connection(&listener, &mut owner);
        let contender = spawn_delivery_process(&fixture, "contender", address);
        let contender_output = wait_for_delivery_process(contender);
        assert_process_success("contender", &contender_output);
        owner_stream
            .write_all(&[1])
            .expect("owner process should be released");
        let owner_output = wait_for_delivery_process(owner);
        assert_process_success("owner", &owner_output);

        let connection = litradar_storage::open_sqlite_connection(&fixture.auth_db_path)
            .expect("auth database should open");
        let completed_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM delivery_runs WHERE status = 'completed'",
                [],
                |row| row.get(0),
            )
            .expect("completed runs should count");
        let cancelled_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM delivery_runs WHERE status = 'cancelled'",
                [],
                |row| row.get(0),
            )
            .expect("cancelled runs should count");
        assert_eq!(completed_count, 1);
        assert_eq!(cancelled_count, 1);
        assert_eq!(favorite_count(&fixture.auth_db_path), 1);
        assert_eq!(delivery_dedupe(&fixture, DeliveryWorkflow::Push).len(), 1);
        let lease = litradar_storage::load_delivery_lease(
            &fixture.auth_db_path,
            litradar_storage::DeliveryWorkflow::Push,
            &fixture.db_name,
        )
        .expect("workflow lease should load")
        .expect("workflow lease should exist");
        assert!(lease.owner_id.is_none());
        assert!(!fixture
            .root
            .path()
            .join("data/folder_push_state/fixture.json")
            .exists());
    }

    #[test]
    fn crashed_sending_process_recovers_as_unknown_without_replay() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));
        let listener = TcpListener::bind("127.0.0.1:0").expect("process listener should bind");
        listener
            .set_nonblocking(true)
            .expect("process listener should be nonblocking");
        let address = listener
            .local_addr()
            .expect("process listener address should resolve");
        let mut crashing = spawn_delivery_process(&fixture, "crash-notify", address);
        let _sending_stream = accept_process_connection(&listener, &mut crashing);
        crashing
            .kill()
            .expect("sending process should be terminated");
        let crashing_output = wait_for_delivery_process(crashing);
        assert!(!crashing_output.status.success());

        let connection = litradar_storage::open_sqlite_connection(&fixture.auth_db_path)
            .expect("auth database should open");
        connection
            .execute_batch(
                "UPDATE delivery_runs SET lease_expires_at = 0
                 WHERE status IN ('claimed', 'running', 'cancelling');
                 UPDATE delivery_leases SET expires_at = 0 WHERE owner_id IS NOT NULL;",
            )
            .expect("crashed owner leases should expire");
        drop(connection);
        let recovery = spawn_delivery_process(&fixture, "recover-notify", address);
        let recovery_output = wait_for_delivery_process(recovery);
        assert_process_success("recovery", &recovery_output);
        thread::sleep(Duration::from_millis(50));
        assert!(matches!(
            listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));

        let dedupe = delivery_dedupe(&fixture, DeliveryWorkflow::Notify);
        assert_eq!(dedupe.len(), 1);
        assert_eq!(
            dedupe[0].status,
            litradar_storage::DeliveryDedupeStatus::Unknown
        );
        let connection = litradar_storage::open_sqlite_connection(&fixture.auth_db_path)
            .expect("auth database should reopen");
        let unknown_item_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM delivery_run_items
                 WHERE item_kind = 'subscriber' AND status = 'unknown'
                   AND error_code = 'abandoned_sending'",
                [],
                |row| row.get(0),
            )
            .expect("unknown subscriber items should count");
        let unknown_run_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM delivery_runs WHERE status = 'unknown'",
                [],
                |row| row.get(0),
            )
            .expect("unknown runs should count");
        assert_eq!(unknown_item_count, 1);
        assert_eq!(unknown_run_count, 1);
        assert_eq!(favorite_count(&fixture.auth_db_path), 1);
    }

    #[test]
    #[ignore = "subprocess helper invoked by delivery process regressions"]
    fn delivery_process_helper() {
        let role = env::var("LITRADAR_DELIVERY_TEST_ROLE")
            .expect("delivery process role should be provided");
        let auth_db_path = PathBuf::from(
            env::var_os("LITRADAR_DELIVERY_TEST_AUTH")
                .expect("delivery process auth path should be provided"),
        );
        let index_db_path = PathBuf::from(
            env::var_os("LITRADAR_DELIVERY_TEST_INDEX")
                .expect("delivery process index path should be provided"),
        );
        let address = env::var("LITRADAR_DELIVERY_TEST_ADDRESS")
            .expect("delivery process address should be provided");
        let workflow = if role.ends_with("notify") {
            DeliveryWorkflow::Notify
        } else {
            DeliveryWorkflow::Push
        };
        let config = RecommendationRunConfig {
            auth_db_path,
            secret_codec: litradar_storage::SecretCodec::from_key([17_u8; 32]),
            index_db_path,
            db_name: "fixture.sqlite".to_string(),
            changes_file: None,
            ai_model: None,
            max_candidates: None,
            timeout_seconds: 60,
            retry_attempts: 1,
            dedupe_retention_days: 30,
            mode: DeliveryMode::Execute,
            workflow,
            trigger: DeliveryTrigger::Scheduled,
            execution_control: None,
        };
        let mut ai_selector = ProcessDeliveryAiSelector {
            role: role.clone(),
            address: address.clone(),
        };
        let mut pushplus_sender = ProcessPushPlusSender {
            role: role.clone(),
            address,
        };
        let result = run_recommendation_delivery_with_services(
            &config,
            &mut ai_selector,
            &mut pushplus_sender,
        );
        match role.as_str() {
            "owner" => assert_eq!(
                result.expect("owner should complete").status,
                DeliveryOutcomeState::Completed
            ),
            "contender" => assert!(matches!(result, Err(DeliveryError::Busy))),
            "recover-notify" => {
                assert_eq!(
                    result.expect("recovery should complete").status,
                    DeliveryOutcomeState::Unknown
                );
            }
            "crash-notify" => panic!("crash helper should be terminated while sending"),
            _ => panic!("unknown delivery process role"),
        }
    }

    struct ProcessDeliveryAiSelector {
        role: String,
        address: String,
    }

    impl DeliveryAiSelector for ProcessDeliveryAiSelector {
        fn select_for_subscriber(
            &mut self,
            _request: DeliveryAiSelectionRequest<'_>,
        ) -> Result<AiSelectionOutcome, DeliveryError> {
            match self.role.as_str() {
                "owner" => {
                    wait_for_process_release(&self.address);
                    Ok(selection_outcome(&[101], ""))
                }
                "crash-notify" => Ok(selection_outcome(&[101], "")),
                "contender" | "recover-notify" => {
                    panic!("non-owner process must not invoke AI selection")
                }
                _ => panic!("unknown process selector role"),
            }
        }
    }

    struct ProcessPushPlusSender {
        role: String,
        address: String,
    }

    impl DeliveryPushPlusSender for ProcessPushPlusSender {
        fn send(&mut self, _message: &PushPlusMessage) -> Result<String, DeliveryError> {
            match self.role.as_str() {
                "crash-notify" => {
                    wait_for_process_release(&self.address);
                    Ok("unexpected-message".to_string())
                }
                "recover-notify" => panic!("recovery must not replay PushPlus"),
                _ => panic!("folder workflow must not invoke PushPlus"),
            }
        }
    }

    fn wait_for_process_release(address: &str) {
        let mut stream = TcpStream::connect(address).expect("process helper should connect");
        stream
            .write_all(&[1])
            .expect("process helper should announce itself");
        let mut release = [0_u8; 1];
        stream
            .read_exact(&mut release)
            .expect("process helper should receive release");
    }

    fn spawn_delivery_process(
        fixture: &DeliveryFixture,
        role: &str,
        address: std::net::SocketAddr,
    ) -> Child {
        Command::new(env::current_exe().expect("current test executable should resolve"))
            .arg("delivery::orchestration::tests::delivery_process_helper")
            .arg("--exact")
            .arg("--ignored")
            .arg("--nocapture")
            .env("LITRADAR_DELIVERY_TEST_ROLE", role)
            .env("LITRADAR_DELIVERY_TEST_AUTH", &fixture.auth_db_path)
            .env("LITRADAR_DELIVERY_TEST_INDEX", &fixture.index_db_path)
            .env("LITRADAR_DELIVERY_TEST_ADDRESS", address.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("delivery process should spawn")
    }

    fn accept_process_connection(listener: &TcpListener, child: &mut Child) -> TcpStream {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut announcement = [0_u8; 1];
                    stream
                        .read_exact(&mut announcement)
                        .expect("process announcement should be readable");
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("process listener failed: {error}"),
            }
            if Instant::now() >= deadline {
                panic!("delivery process did not reach its synchronization boundary");
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!("delivery process exited before synchronization: {status}");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_delivery_process(mut child: Child) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if child
                .try_wait()
                .expect("delivery process status should be readable")
                .is_some()
            {
                return child
                    .wait_with_output()
                    .expect("delivery process output should be collected");
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .expect("timed out delivery process output should collect");
                panic!(
                    "delivery process timed out: stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_process_success(role: &str, output: &Output) {
        assert!(
            output.status.success(),
            "{role} process failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct FixtureDeliveryAiSelector {
        outcomes: Vec<AiSelectionOutcome>,
        subscriber_ids: Vec<String>,
    }

    impl FixtureDeliveryAiSelector {
        fn new(outcomes: Vec<AiSelectionOutcome>) -> Self {
            Self {
                outcomes: outcomes.into_iter().rev().collect(),
                subscriber_ids: Vec::new(),
            }
        }
    }

    impl DeliveryAiSelector for FixtureDeliveryAiSelector {
        fn select_for_subscriber(
            &mut self,
            request: DeliveryAiSelectionRequest<'_>,
        ) -> Result<AiSelectionOutcome, DeliveryError> {
            self.subscriber_ids
                .push(request.subscriber.subscriber_id.clone());
            self.outcomes
                .pop()
                .ok_or_else(|| DeliveryError::Ai("missing fixture AI selection".into()))
        }
    }

    #[derive(Debug)]
    struct FixturePushPlusSender {
        responses: Vec<Result<String, DeliveryError>>,
        messages: Vec<PushPlusMessage>,
    }

    impl FixturePushPlusSender {
        fn new(responses: Vec<Result<String, DeliveryError>>) -> Self {
            Self {
                responses: responses.into_iter().rev().collect(),
                messages: Vec::new(),
            }
        }
    }

    impl DeliveryPushPlusSender for FixturePushPlusSender {
        fn send(&mut self, message: &PushPlusMessage) -> Result<String, DeliveryError> {
            self.messages.push(message.clone());
            self.responses
                .pop()
                .unwrap_or_else(|| Err(DeliveryError::PushPlus("missing PushPlus fixture".into())))
        }
    }
    fn run_fixture_delivery(
        config: &RecommendationRunConfig,
        outcomes: Vec<AiSelectionOutcome>,
        pushplus_responses: Vec<Result<String, DeliveryError>>,
    ) -> Result<(RecommendationRunOutcome, FixturePushPlusSender), DeliveryError> {
        let mut ai_selector = FixtureDeliveryAiSelector::new(outcomes);
        let mut pushplus_sender = FixturePushPlusSender::new(pushplus_responses);
        let outcome = run_recommendation_delivery_with_services(
            config,
            &mut ai_selector,
            &mut pushplus_sender,
        )?;
        Ok((outcome, pushplus_sender))
    }

    fn selection_outcome(article_ids: &[i64], summary: &str) -> AiSelectionOutcome {
        AiSelectionOutcome {
            accepted: article_ids
                .iter()
                .enumerate()
                .map(|(index, article_id)| RankedSelectionInfo {
                    article_id: *article_id,
                    score: 100.0 - index as f64,
                })
                .collect(),
            summary: summary.to_string(),
            skip_reason: None,
        }
    }
    struct DeliveryFixture {
        root: TempDir,
        auth_db_path: PathBuf,
        secret_codec: litradar_storage::SecretCodec,
        user_id: UserId,
        index_db_path: PathBuf,
        db_name: String,
    }

    impl DeliveryFixture {
        fn new(settings: NotificationSettingsUpdate) -> Self {
            let root = tempdir().expect("temp dir should be created");
            let auth_db_path = root.path().join("auth.sqlite");
            let secret_codec = litradar_storage::SecretCodec::from_key([17_u8; 32]);
            litradar_storage::initialize_auth_database(&auth_db_path)
                .expect("auth database should initialize");
            litradar_storage::upsert_runtime_settings(
                &auth_db_path,
                &secret_codec,
                &HashMap::from([(
                    "ai_allowed_base_urls".to_string(),
                    Some("https://api.siliconflow.cn/v1/".to_string()),
                )]),
                &HashMap::new(),
            )
            .expect("AI endpoint catalog should persist");
            let user =
                litradar_storage::bootstrap_admin(&auth_db_path, "alice", "hash", "salt", 1.0)
                    .expect("fixture administrator should be bootstrapped");
            litradar_storage::create_folder(&auth_db_path, user.id, "Tracking", true)
                .expect("tracking folder should be created");
            litradar_storage::upsert_notification_settings(
                &auth_db_path,
                &secret_codec,
                user.id,
                &settings,
            )
            .expect("notification settings should be saved");
            let index_db_path = root.path().join("fixture.sqlite");
            create_index_database(&index_db_path);
            Self {
                root,
                auth_db_path,
                secret_codec,
                user_id: user.id,
                index_db_path,
                db_name: "fixture.sqlite".to_string(),
            }
        }

        fn config(
            &self,
            workflow: DeliveryWorkflow,
            mode: DeliveryMode,
            changes_file: Option<PathBuf>,
            max_candidates: Option<usize>,
        ) -> RecommendationRunConfig {
            RecommendationRunConfig {
                auth_db_path: self.auth_db_path.clone(),
                secret_codec: self.secret_codec.clone(),
                index_db_path: self.index_db_path.clone(),
                db_name: self.db_name.clone(),
                changes_file,
                ai_model: None,
                max_candidates,
                timeout_seconds: 60,
                retry_attempts: 3,
                dedupe_retention_days: 30,
                mode,
                workflow,
                trigger: DeliveryTrigger::Scheduled,
                execution_control: None,
            }
        }

        fn add_subscriber(&self, username: &str, settings: NotificationSettingsUpdate) -> UserId {
            let connection = litradar_storage::open_sqlite_connection(&self.auth_db_path)
                .expect("auth database should open");
            connection
                .execute(
                    "INSERT INTO users \
                     (username, password_hash, salt, is_admin, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, 0, ?4, ?4)",
                    (username, "hash", "salt", 2.0_f64),
                )
                .expect("subscriber user should be inserted");
            let user_id = UserId(connection.last_insert_rowid());
            drop(connection);
            litradar_storage::create_folder(&self.auth_db_path, user_id, "Tracking", true)
                .expect("subscriber tracking folder should be created");
            litradar_storage::upsert_notification_settings(
                &self.auth_db_path,
                &self.secret_codec,
                user_id,
                &settings,
            )
            .expect("subscriber settings should be saved");
            user_id
        }

        fn corrupt_notification_ai_key(&self, user_id: UserId) {
            let connection = litradar_storage::open_sqlite_connection(&self.auth_db_path)
                .expect("auth database should open");
            connection
                .execute(
                    "UPDATE notification_settings SET ai_api_key = 'litradarenc:v1:bad' \
                     WHERE user_id = ?1",
                    [user_id.value()],
                )
                .expect("subscriber ciphertext should be corrupted");
        }
    }

    fn notification_settings(
        delivery_method: &str,
        enabled: bool,
        selected_databases: Vec<String>,
    ) -> NotificationSettingsUpdate {
        NotificationSettingsUpdate {
            keywords: vec!["rust".to_string()],
            directions: vec!["systems".to_string()],
            selected_databases,
            delivery_method: delivery_method.to_string(),
            pushplus_token: if delivery_method == "pushplus" {
                Some(Some("token".to_string()))
            } else {
                None
            },
            pushplus_template: "markdown".to_string(),
            pushplus_topic: String::new(),
            pushplus_channel: "wechat".to_string(),
            sync_to_tracking_folder: true,
            ai_base_url: String::new(),
            ai_api_key: Some(Some("key".to_string())),
            ai_model: "model".to_string(),
            ai_system_prompt: String::new(),
            ai_backup_base_url: String::new(),
            ai_backup_api_key: None,
            ai_backup_model: String::new(),
            ai_backup_system_prompt: String::new(),
            ai_retry_attempts: 1,
            enabled,
        }
    }

    fn create_index_database(path: &Path) {
        litradar_storage::migrate_index_database(path, None)
            .expect("index database should migrate");
        let connection =
            litradar_storage::open_sqlite_connection(path).expect("index database should open");
        connection
            .execute_batch(
                r#"
                INSERT INTO journals (
                    journal_id, catalog_id, title, title_aliases_json, issns_json,
                    issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating,
                    fms_rank, fms_rating, fmscn_rank, fmscn_rating
                ) VALUES (
                    1, 'fixture-journal', 'Fixture Journal', '[]',
                    '["1234-5679"]', '1234-5679', NULL, 'Systems',
                    NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL
                );

                INSERT INTO issues (
                    issue_id, journal_id, publication_year, title, volume, number, date
                ) VALUES (11, 1, 2026, 'Fixture Issue', '1', '1', '2026-07-01');

                INSERT INTO articles (
                    article_id, journal_id, issue_id, title, publication_year, date,
                    authors_json, start_page, end_page, abstract_text, doi, pmid,
                    open_access, in_press
                ) VALUES
                    (101, 1, 11, 'Rust systems', 2026, '2026-07-01', '["Alice"]',
                     NULL, NULL, 'rust systems', '10.0000/101', NULL, 1, 0),
                    (102, 1, 11, 'Rust migration', 2026, '2026-07-01', '["Bob"]',
                     NULL, NULL, 'rust migration', '10.0000/102', NULL, 1, 0);
                "#,
            )
            .expect("index fixture data should be created");
    }

    fn delivery_checkpoint(
        fixture: &DeliveryFixture,
        workflow: DeliveryWorkflow,
    ) -> litradar_storage::DeliveryCheckpointRecord {
        litradar_storage::load_delivery_checkpoint(
            &fixture.auth_db_path,
            storage_workflow(workflow),
            &fixture.db_name,
        )
        .expect("delivery checkpoint should load")
        .expect("delivery checkpoint should exist")
    }

    fn delivery_dedupe(
        fixture: &DeliveryFixture,
        workflow: DeliveryWorkflow,
    ) -> Vec<litradar_storage::DeliveryDedupeRecord> {
        litradar_storage::list_delivery_dedupe_for_scope(
            &fixture.auth_db_path,
            storage_workflow(workflow),
            &fixture.db_name,
        )
        .expect("delivery dedupe should load")
    }

    fn favorite_count(auth_db_path: &Path) -> i64 {
        litradar_storage::count_favorites(auth_db_path, UserId(1), None)
            .expect("favorites should be counted")
    }
}
