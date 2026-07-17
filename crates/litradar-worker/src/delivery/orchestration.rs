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
    let mut ai_selector = DefaultDeliveryAiSelector::live(timeout_seconds, config.retry_attempts);
    let mut pushplus_sender =
        LiveDeliveryPushPlusSender::new(timeout_seconds, config.retry_attempts)?;
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
    let mut ai_selector = DefaultDeliveryAiSelector::live(timeout_seconds, config.retry_attempts);
    let mut pushplus_sender =
        LiveDeliveryPushPlusSender::new(timeout_seconds, config.retry_attempts)?;
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
    let settings = litradar_storage::get_notification_settings(
        config.storage_config.auth_db_path(),
        &config.secret_codec,
        config.user_id,
    )?;
    let Some(settings) = settings.filter(|item| item.enabled) else {
        return Ok(manual_outcome(
            "completed",
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
            "completed",
            message,
            folder.as_ref().map(|item| item.id),
            folder.as_ref().map(|item| item.name.clone()),
        ));
    }

    if settings.keywords.is_empty() && settings.directions.is_empty() {
        return Ok(manual_outcome(
            "completed",
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
    let state_dir = manual_delivery_state_dir(config.storage_config.project_root(), workflow);
    let mut outcomes = Vec::new();
    for manifest in manifests {
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
                state_dir: state_dir.clone(),
                changes_file: Some(manifest.path),
                ai_model: config.ai_model.clone(),
                max_candidates: config.max_candidates,
                timeout_seconds: config.timeout_seconds,
                retry_attempts: config.retry_attempts,
                dedupe_retention_days: config.dedupe_retention_days,
                mode: DeliveryMode::Execute,
                workflow,
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
    let now = utc_now_iso();
    let state_path = state_path(&config.state_dir, &config.db_name);
    let mut state = load_state(&state_path, &config.db_name, &now)?;
    let current_issue_counts =
        litradar_storage::collect_issue_article_counts(&config.index_db_path)?;
    let current_inpress_counts =
        litradar_storage::collect_inpress_article_counts(&config.index_db_path)?;

    let (pending_issue_keys, pending_inpress_keys, pending_article_ids, manifest_run_id) =
        if let Some(changes_file) = &config.changes_file {
            let manifest = load_change_manifest(changes_file, &config.db_name)?;
            (
                manifest.pending_issue_keys,
                manifest.pending_inpress_keys,
                manifest.pending_article_ids,
                manifest.run_id,
            )
        } else {
            (
                compute_changed_issue_keys(
                    &state.snapshot.issue_article_counts,
                    &current_issue_counts,
                ),
                compute_changed_inpress_keys(
                    &state.snapshot.inpress_article_counts,
                    &current_inpress_counts,
                ),
                Vec::new(),
                None,
            )
        };

    if pending_issue_keys.is_empty()
        && pending_inpress_keys.is_empty()
        && pending_article_ids.is_empty()
    {
        state.status = "idle".to_string();
        state.run = None;
        state.updated_at = now.clone();
        save_state_atomic(&state_path, &state)?;
        return Ok(outcome(config, state_path, "idle", Vec::new(), Vec::new()));
    }

    let run_id = manifest_run_id.unwrap_or_else(|| now.clone());
    let mut run_state = create_run_state(
        &run_id,
        pending_issue_keys.clone(),
        pending_inpress_keys.clone(),
        &now,
    );
    state.status = "running".to_string();
    state.run = Some(run_state.clone());
    state.updated_at = now.clone();
    save_state_atomic(&state_path, &state)?;

    let candidates = if pending_issue_keys.is_empty() && pending_inpress_keys.is_empty() {
        litradar_storage::fetch_candidates_for_article_ids(
            &config.index_db_path,
            &pending_article_ids,
        )?
    } else {
        let mut candidates = litradar_storage::fetch_candidates_for_issue_keys(
            &config.index_db_path,
            &pending_issue_keys,
        )?;
        candidates.extend(litradar_storage::fetch_candidates_for_inpress_keys(
            &config.index_db_path,
            &pending_inpress_keys,
        )?);
        candidates
    };
    let mut candidates = deduplicate_candidates(candidates);
    if config.changes_file.is_some() {
        let pending_article_ids = pending_article_ids.into_iter().collect::<BTreeSet<_>>();
        candidates.retain(|candidate| pending_article_ids.contains(&candidate.article_id));
    }

    if candidates.is_empty() {
        complete_without_candidates(
            &mut state,
            &mut run_state,
            &current_issue_counts,
            &current_inpress_counts,
            &pending_issue_keys,
            &pending_inpress_keys,
            &now,
        );
        save_state_atomic(&state_path, &state)?;
        return Ok(outcome(
            config,
            state_path,
            "completed",
            Vec::new(),
            Vec::new(),
        ));
    }

    run_state.delivered_article_ids = candidates
        .iter()
        .map(|candidate| candidate.article_id)
        .collect();
    run_state.updated_at = now.clone();
    state.run = Some(run_state.clone());
    state.updated_at = now.clone();
    save_state_atomic(&state_path, &state)?;

    let subscribers = filtered_subscribers(
        &config.auth_db_path,
        &config.secret_codec,
        &config.db_name,
        config.workflow,
        subscriber_user_id,
    )?;
    if subscribers.is_empty() {
        run_state.status = "skipped".to_string();
        run_state.updated_at = now.clone();
        state.status = "skipped".to_string();
        state.run = Some(run_state);
        state.updated_at = now.clone();
        save_state_atomic(&state_path, &state)?;
        return Ok(outcome(
            config,
            state_path,
            "skipped",
            candidates
                .iter()
                .map(|candidate| candidate.article_id)
                .collect(),
            Vec::new(),
        ));
    }

    let global_config = load_global_config();
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
    let candidate_article_ids = candidates
        .iter()
        .map(|candidate| candidate.article_id)
        .collect::<Vec<_>>();
    let mut delivery_dedupe = state.delivery_dedupe.clone();
    let mut plans = Vec::new();
    let mut errors = Vec::new();

    for subscriber in subscribers {
        match build_subscriber_plan(
            SubscriberPlanRequest {
                config,
                subscriber: &subscriber,
                global_config: &global_config,
                defaults: &defaults,
                run_id: &run_id,
                candidates_for_model: &candidates_for_model,
                candidates_by_id: &candidates_by_id,
                delivery_dedupe: &mut delivery_dedupe,
            },
            ai_selector,
            pushplus_sender,
        ) {
            Ok(plan) => {
                run_state
                    .user_results
                    .push(user_result_from_plan(config.workflow, &plan));
                plans.push(plan);
            }
            Err(error) => {
                let error_message = format!("{}: {error}", subscriber.subscriber_id);
                errors.push(error_message);
                run_state.user_results.push(RecommendationUserResult {
                    subscriber_id: subscriber.subscriber_id,
                    selected_count: 0,
                    pushed_count: 0,
                    folder_synced_count: Some(0),
                    message_id: None,
                    status: "error".to_string(),
                    error: Some(error.to_string()),
                });
            }
        }
        run_state.updated_at = utc_now_iso();
        state.updated_at = run_state.updated_at.clone();
        state.run = Some(run_state.clone());
        if should_save_subscriber_progress(config) {
            save_state_atomic(&state_path, &state)?;
        }
    }

    if errors.is_empty() {
        let completed_at = utc_now_iso();
        state.delivery_dedupe = prune_delivery_dedupe(
            &delivery_dedupe,
            config.dedupe_retention_days,
            SystemTime::now(),
        );
        complete_successfully(
            &mut state,
            &mut run_state,
            &current_issue_counts,
            &current_inpress_counts,
            &pending_issue_keys,
            &pending_inpress_keys,
            &completed_at,
        );
    } else {
        run_state.status = "failed".to_string();
        run_state.errors = errors;
        run_state.updated_at = utc_now_iso();
        state.status = "failed".to_string();
        state.updated_at = run_state.updated_at.clone();
        state.run = Some(run_state);
    }
    save_state_atomic(&state_path, &state)?;

    Ok(outcome(
        config,
        state_path,
        state.status.as_str(),
        candidate_article_ids,
        plans,
    ))
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
                .filter(|subscriber| subscriber.status == "error")
                .count();
            if outcome.status == "failed" {
                tracing::warn!(
                    event = "delivery.workflow.failed",
                    component = "delivery",
                    outcome = "failure",
                    status = "failed",
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
                    status = delivery_status_kind(&outcome.status),
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
        Ok(outcome) if outcome.status == "failed" => tracing::warn!(
            event = "delivery.manual.failed",
            component = "delivery",
            outcome = "failure",
            status = "failed",
            selected_count = outcome.selected,
            delivered_count = outcome.pushed,
            candidate_count = outcome.total_candidates.unwrap_or(0),
            duration_ms = elapsed_millis(started_at),
        ),
        Ok(outcome) => tracing::info!(
            event = "delivery.manual.completed",
            component = "delivery",
            outcome = "success",
            status = delivery_status_kind(&outcome.status),
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
        DeliveryError::Recommendation(_) => "recommendation",
        DeliveryError::Ai(_) => "ai",
        DeliveryError::PushPlus(_) => "pushplus",
        DeliveryError::Manual(_) => "manual_validation",
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

fn delivery_status_kind(status: &str) -> &'static str {
    match status {
        "completed" => "completed",
        "idle" => "idle",
        "skipped" => "skipped",
        "failed" => "failed",
        _ => "unknown",
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn manual_outcome(
    status: &str,
    message: &str,
    folder_id: Option<i64>,
    folder_name: Option<String>,
) -> ManualWeeklyPushOutcome {
    ManualWeeklyPushOutcome {
        status: status.to_string(),
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
    let mut skip_messages = Vec::new();

    for outcome in outcomes {
        total_candidates += outcome.candidate_article_ids.len() as i64;
        if outcome.status == "failed" {
            errors.push(format!("{} delivery failed", outcome.db_name));
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
            status: "failed".to_string(),
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
        status: "completed".to_string(),
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

struct SubscriberPlanRequest<'a> {
    config: &'a RecommendationRunConfig,
    subscriber: &'a NotificationSubscriberInfo,
    global_config: &'a NotificationGlobalConfig,
    defaults: &'a NotificationDefaults,
    run_id: &'a str,
    candidates_for_model: &'a [ArticleCandidateInfo],
    candidates_by_id: &'a BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &'a mut BTreeMap<String, String>,
}

fn build_subscriber_plan(
    request: SubscriberPlanRequest<'_>,
    ai_selector: &mut impl DeliveryAiSelector,
    pushplus_sender: &mut impl DeliveryPushPlusSender,
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
    let mut message_id = None;
    if config.mode == DeliveryMode::Execute {
        execute_favorite_writes(config, &favorite_writes)?;
        if config.workflow == DeliveryWorkflow::Notify {
            let title = message_title
                .as_deref()
                .ok_or_else(|| DeliveryError::PushPlus("PushPlus title is unavailable".into()))?;
            let content = message_content
                .as_deref()
                .ok_or_else(|| DeliveryError::PushPlus("PushPlus content is unavailable".into()))?;
            message_id = Some(pushplus_sender.send(&pushplus_message(
                subscriber,
                global_config,
                title,
                content,
            ))?);
        }
        for article_id in &selected_article_ids {
            delivery_dedupe.insert(
                format!("{}:{article_id}", subscriber.subscriber_id),
                utc_now_iso(),
            );
        }
    }
    Ok(SubscriberDeliveryPlan {
        subscriber_id: subscriber.subscriber_id.clone(),
        delivery_method: subscriber.delivery_method.clone(),
        status: "ok".to_string(),
        error: None,
        selected_article_ids,
        message_title,
        message_content,
        message_id,
        folder_synced_count: favorite_writes.len(),
        favorite_writes,
        would_send_pushplus,
    })
}

fn skipped_plan(subscriber: &NotificationSubscriberInfo, reason: &str) -> SubscriberDeliveryPlan {
    SubscriberDeliveryPlan {
        subscriber_id: subscriber.subscriber_id.clone(),
        delivery_method: subscriber.delivery_method.clone(),
        status: "skipped".to_string(),
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
    state_path: PathBuf,
    status: &str,
    candidate_article_ids: Vec<i64>,
    subscribers: Vec<SubscriberDeliveryPlan>,
) -> RecommendationRunOutcome {
    RecommendationRunOutcome {
        db_name: config.db_name.clone(),
        workflow: config.workflow,
        mode: config.mode,
        status: status.to_string(),
        state_path,
        candidate_article_ids,
        subscribers,
    }
}

fn load_global_config() -> NotificationGlobalConfig {
    NotificationGlobalConfig {
        ai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
        ai_api_key: String::new(),
        pushplus_channel: PUSHPLUS_CHANNEL.to_string(),
        pushplus_template: "markdown".to_string(),
        pushplus_topic: None,
        pushplus_option: None,
        ai_system_prompt: None,
    }
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
    use std::fs;
    use std::path::{Path, PathBuf};

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

        assert_eq!(outcome.status, "completed");
        assert_eq!(outcome.candidate_article_ids, vec![102, 101]);
        assert_eq!(outcome.subscribers.len(), 1);
        let plan = &outcome.subscribers[0];
        assert_eq!(plan.status, "ok");
        assert_eq!(plan.selected_article_ids, vec![101, 102]);
        assert_eq!(plan.folder_synced_count, 2);
        assert_eq!(plan.favorite_writes.len(), 2);
        assert!(!plan.would_send_pushplus);
        assert_eq!(favorite_count(&fixture.auth_db_path), 0);
        let state =
            litradar_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
                .expect("state should be written");
        assert_eq!(state.status, "completed");
        assert!(state.delivery_dedupe.is_empty());
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

        assert_eq!(outcome.status, "completed");
        assert_eq!(outcome.subscribers.len(), 1);
        let plan = &outcome.subscribers[0];
        assert_eq!(plan.status, "ok");
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

        assert_eq!(outcome.status, "completed");
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

        assert_eq!(outcome.status, "completed");
        assert_eq!(outcome.subscribers[0].message_id.as_deref(), Some("msg-1"));
        assert_eq!(pushplus_sender.messages.len(), 1);
        assert_eq!(pushplus_sender.messages[0].token, "token");
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let state =
            litradar_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
                .expect("state should be written");
        let run = state.run.expect("run state should be recorded");
        assert_eq!(run.user_results[0].message_id.as_deref(), Some("msg-1"));
        assert_eq!(state.delivery_dedupe.len(), 2);
    }

    #[test]
    fn execute_notify_pushplus_failure_does_not_update_dedupe() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));

        let (outcome, _pushplus_sender) = run_fixture_delivery(
            &fixture.config(DeliveryWorkflow::Notify, DeliveryMode::Execute, None, None),
            vec![selection_outcome(&[101, 102], "")],
            vec![Err(DeliveryError::PushPlus("send failed".to_string()))],
        )
        .expect("notify execute should record PushPlus failure");

        assert_eq!(outcome.status, "failed");
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let state =
            litradar_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
                .expect("state should be written");
        assert!(state.delivery_dedupe.is_empty());
        assert!(state
            .run
            .expect("run state should be recorded")
            .errors
            .iter()
            .any(|error| error.contains("send failed")));
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

        assert_eq!(outcome.status, "completed");
        assert_eq!(outcome.subscribers[0].favorite_writes.len(), 2);
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let state =
            litradar_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
                .expect("state should be written");
        assert_eq!(state.delivery_dedupe.len(), 2);
        assert!(state.delivery_dedupe.contains_key("1:101"));
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

        assert_eq!(disabled_outcome.status, "skipped");
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

        assert_eq!(unselected_outcome.status, "skipped");
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

        assert_eq!(outcome.status, "completed");
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
        let state =
            litradar_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
                .expect("completed state should load");
        assert!(state
            .delivery_dedupe
            .keys()
            .all(|key| key.starts_with(&format!("{}:", fixture.user_id.value()))));

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

        let mut corrupt_target_config =
            fixture.config(DeliveryWorkflow::Notify, DeliveryMode::Execute, None, None);
        corrupt_target_config.state_dir = fixture.root.path().join("corrupt-target-state");
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
        let corrupt_target_state = litradar_recommend::load_state(
            &state_path(
                &corrupt_target_config.state_dir,
                &corrupt_target_config.db_name,
            ),
            &corrupt_target_config.db_name,
            "ignored",
        )
        .expect("corrupt target state should load");
        assert_eq!(corrupt_target_state.status, "running");
        assert!(corrupt_target_state.delivery_dedupe.is_empty());
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
        state_dir: PathBuf,
        db_name: String,
    }

    impl DeliveryFixture {
        fn new(settings: NotificationSettingsUpdate) -> Self {
            let root = tempdir().expect("temp dir should be created");
            let auth_db_path = root.path().join("auth.sqlite");
            let secret_codec = litradar_storage::SecretCodec::from_key([17_u8; 32]);
            litradar_storage::initialize_auth_database(&auth_db_path)
                .expect("auth database should initialize");
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
            let state_dir = root.path().join("state");
            Self {
                root,
                auth_db_path,
                secret_codec,
                user_id: user.id,
                index_db_path,
                state_dir,
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
                state_dir: self.state_dir.clone(),
                changes_file,
                ai_model: None,
                max_candidates,
                timeout_seconds: 60,
                retry_attempts: 3,
                dedupe_retention_days: 30,
                mode,
                workflow,
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
        let connection =
            litradar_storage::open_sqlite_connection(path).expect("index database should open");
        connection
            .execute_batch(
                "
                CREATE TABLE journals (
                    journal_id INTEGER PRIMARY KEY,
                    title TEXT NOT NULL
                );
                CREATE TABLE articles (
                    article_id INTEGER PRIMARY KEY,
                    journal_id INTEGER NOT NULL,
                    issue_id INTEGER,
                    title TEXT NOT NULL,
                    abstract TEXT,
                    date TEXT,
                    open_access INTEGER,
                    in_press INTEGER,
                    within_library_holdings INTEGER,
                    doi TEXT,
                    full_text_file TEXT,
                    permalink TEXT,
                    suppressed INTEGER
                );
                ",
            )
            .expect("index schema should be created");
        connection
            .execute(
                "INSERT INTO journals (journal_id, title) VALUES (?1, ?2)",
                (1_i64, "Fixture Journal"),
            )
            .expect("journal should be inserted");
        for (article_id, issue_id, title, abstract_text) in [
            (101, Some(11), "Rust systems", "rust systems"),
            (102, Some(11), "Rust migration", "rust migration"),
            (103, None, "Suppressed Rust", "rust hidden"),
        ] {
            connection
                .execute(
                    "INSERT INTO articles (
                    article_id, journal_id, issue_id, title, abstract, date, open_access,
                    in_press, within_library_holdings, doi, full_text_file, permalink, suppressed
                ) VALUES (?1, 1, ?2, ?3, ?4, '2026-07-01', 1, ?5, 1, ?6, '', ?7, ?8)",
                    (
                        article_id,
                        issue_id,
                        title,
                        abstract_text,
                        if issue_id.is_none() { 1_i64 } else { 0_i64 },
                        format!("10.0000/{article_id}"),
                        format!("https://example.test/{article_id}"),
                        if article_id == 103 { 1_i64 } else { 0_i64 },
                    ),
                )
                .expect("article should be inserted");
        }
    }

    fn favorite_count(auth_db_path: &Path) -> i64 {
        litradar_storage::count_favorites(auth_db_path, UserId(1), None)
            .expect("favorites should be counted")
    }
}
