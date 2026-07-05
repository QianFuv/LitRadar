//! Notification and tracking delivery worker orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ps_domain::{
    ArticleCandidateInfo, FavoriteAdd, NotificationSubscriberInfo, SelectionResultInfo, UserId,
};
use ps_recommend::{
    apply_selection_rules, build_markdown_content, build_message_title,
    compute_changed_inpress_keys, compute_changed_issue_keys, create_run_state,
    deduplicate_candidates, has_selection_preferences, is_database_selected, load_change_manifest,
    load_state, prune_delivery_dedupe, resolve_ai_runtime_configs, save_state_atomic, utc_now_iso,
    NotificationDefaults, NotificationGlobalConfig, RecommendationState, RecommendationUserResult,
    DEFAULT_OPENAI_BASE_URL, DEFAULT_OPENAI_MODEL, PUSHPLUS_CHANNEL,
};
use serde::Serialize;

/// Delivery worker errors.
#[derive(Debug)]
pub enum DeliveryError {
    /// Index storage operation failed.
    Index(ps_storage::IndexRepositoryError),
    /// Auth database storage operation failed.
    Business(ps_storage::BusinessRepositoryError),
    /// Recommendation logic failed.
    Recommendation(ps_recommend::RecommendationError),
    /// PushPlus execution is intentionally unavailable during dry-run parity.
    PushPlusExecutionUnavailable,
}

impl fmt::Display for DeliveryError {
    /// Format the delivery error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(error) => write!(formatter, "{error}"),
            Self::Business(error) => write!(formatter, "{error}"),
            Self::Recommendation(error) => write!(formatter, "{error}"),
            Self::PushPlusExecutionUnavailable => {
                formatter.write_str("PushPlus execution is unavailable in Rust dry-run parity mode")
            }
        }
    }
}

impl Error for DeliveryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Index(error) => Some(error),
            Self::Business(error) => Some(error),
            Self::Recommendation(error) => Some(error),
            Self::PushPlusExecutionUnavailable => None,
        }
    }
}

impl From<ps_storage::IndexRepositoryError> for DeliveryError {
    /// Convert index repository errors into delivery errors.
    fn from(error: ps_storage::IndexRepositoryError) -> Self {
        Self::Index(error)
    }
}

impl From<ps_storage::BusinessRepositoryError> for DeliveryError {
    /// Convert business repository errors into delivery errors.
    fn from(error: ps_storage::BusinessRepositoryError) -> Self {
        Self::Business(error)
    }
}

impl From<ps_recommend::RecommendationError> for DeliveryError {
    /// Convert recommendation errors into delivery errors.
    fn from(error: ps_recommend::RecommendationError) -> Self {
        Self::Recommendation(error)
    }
}

/// Recommendation delivery workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryWorkflow {
    /// PushPlus notification workflow.
    Notify,
    /// Tracking-folder push workflow.
    Push,
}

/// Worker delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Plan delivery without side effects.
    DryRun,
    /// Shadow delivery without side effects.
    Shadow,
    /// Execute side effects.
    Execute,
}

/// Recommendation worker run configuration.
#[derive(Debug, Clone)]
pub struct RecommendationRunConfig {
    /// Path to `auth.sqlite`.
    pub auth_db_path: PathBuf,
    /// Path to selected index SQLite database.
    pub index_db_path: PathBuf,
    /// Selected database filename.
    pub db_name: String,
    /// State directory.
    pub state_dir: PathBuf,
    /// Optional change manifest path.
    pub changes_file: Option<PathBuf>,
    /// Optional model override.
    pub ai_model: Option<String>,
    /// Optional max-candidates override.
    pub max_candidates: Option<usize>,
    /// Dedupe retention days.
    pub dedupe_retention_days: i64,
    /// Delivery mode.
    pub mode: DeliveryMode,
    /// Delivery workflow.
    pub workflow: DeliveryWorkflow,
}

/// Planned favorite write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FavoriteWritePlan {
    /// User identifier.
    pub user_id: i64,
    /// Tracking folder identifier.
    pub folder_id: i64,
    /// Article identifier.
    pub article_id: i64,
    /// Source database filename.
    pub db_name: String,
}

/// Per-subscriber delivery plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriberDeliveryPlan {
    /// Subscriber identifier.
    pub subscriber_id: String,
    /// Delivery method.
    pub delivery_method: String,
    /// Result status.
    pub status: String,
    /// Skip or error reason.
    pub error: Option<String>,
    /// Accepted article ids.
    pub selected_article_ids: Vec<i64>,
    /// Planned PushPlus title.
    pub message_title: Option<String>,
    /// Planned PushPlus content.
    pub message_content: Option<String>,
    /// Planned tracking favorite writes.
    pub favorite_writes: Vec<FavoriteWritePlan>,
    /// Folder sync count.
    pub folder_synced_count: usize,
    /// Whether PushPlus would be called in execute mode.
    pub would_send_pushplus: bool,
}

/// Recommendation worker outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecommendationRunOutcome {
    /// Selected database filename.
    pub db_name: String,
    /// Workflow name.
    pub workflow: DeliveryWorkflow,
    /// Delivery mode.
    pub mode: DeliveryMode,
    /// Final run status.
    pub status: String,
    /// State file path.
    pub state_path: PathBuf,
    /// Candidate article ids considered by the run.
    pub candidate_article_ids: Vec<i64>,
    /// Per-subscriber delivery plans.
    pub subscribers: Vec<SubscriberDeliveryPlan>,
}

/// Run notification or tracking delivery.
///
/// # Arguments
///
/// * `config` - Worker run configuration.
///
/// # Returns
///
/// Dry-run, shadow, or execution outcome.
pub fn run_recommendation_delivery(
    config: &RecommendationRunConfig,
) -> Result<RecommendationRunOutcome, DeliveryError> {
    let now = utc_now_iso();
    let state_path = state_path(&config.state_dir, &config.db_name);
    let mut state = load_state(&state_path, &config.db_name, &now)?;
    let current_issue_counts = ps_storage::collect_issue_article_counts(&config.index_db_path)?;
    let current_inpress_counts = ps_storage::collect_inpress_article_counts(&config.index_db_path)?;

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

    if pending_issue_keys.is_empty() && pending_inpress_keys.is_empty() {
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

    let mut candidates =
        ps_storage::fetch_candidates_for_issue_keys(&config.index_db_path, &pending_issue_keys)?;
    candidates.extend(ps_storage::fetch_candidates_for_inpress_keys(
        &config.index_db_path,
        &pending_inpress_keys,
    )?);
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

    let subscribers = filtered_subscribers(&config.auth_db_path, &config.db_name, config.workflow)?;
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
    let candidates = candidates
        .into_iter()
        .take(defaults.max_candidates)
        .collect::<Vec<_>>();
    let candidates_by_id = candidates_by_id(&candidates);
    let mut delivery_dedupe = state.delivery_dedupe.clone();
    let mut plans = Vec::new();
    let mut errors = Vec::new();

    for subscriber in subscribers {
        match build_subscriber_plan(
            config,
            &subscriber,
            &global_config,
            &defaults,
            &run_id,
            &candidates_by_id,
            &mut delivery_dedupe,
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
        state
            .run
            .as_ref()
            .map(|run| run.delivered_article_ids.clone())
            .unwrap_or_default(),
        plans,
    ))
}

fn should_save_subscriber_progress(config: &RecommendationRunConfig) -> bool {
    config.mode == DeliveryMode::Execute
}

fn filtered_subscribers(
    auth_db_path: &Path,
    db_name: &str,
    workflow: DeliveryWorkflow,
) -> Result<Vec<NotificationSubscriberInfo>, DeliveryError> {
    Ok(ps_storage::list_notification_subscribers(auth_db_path)?
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

fn build_subscriber_plan(
    config: &RecommendationRunConfig,
    subscriber: &NotificationSubscriberInfo,
    global_config: &NotificationGlobalConfig,
    defaults: &NotificationDefaults,
    run_id: &str,
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &mut BTreeMap<String, String>,
) -> Result<SubscriberDeliveryPlan, DeliveryError> {
    if !has_selection_preferences(subscriber) {
        return Ok(skipped_plan(
            subscriber,
            "No keywords or directions configured",
        ));
    }
    let ai_configs = resolve_ai_runtime_configs(
        subscriber,
        global_config,
        defaults,
        config.ai_model.as_deref(),
    );
    if ai_configs.is_empty() {
        return Ok(skipped_plan(subscriber, "AI configuration is unavailable"));
    }
    let accepted = apply_selection_rules(
        &SelectionResultInfo {
            summary: String::new(),
            selections: Vec::new(),
        },
        subscriber,
        candidates_by_id,
        delivery_dedupe,
    );
    if accepted.is_empty() {
        return Ok(skipped_plan(
            subscriber,
            "AI selection found no matching articles",
        ));
    }
    if config.workflow == DeliveryWorkflow::Notify && config.mode == DeliveryMode::Execute {
        return Err(DeliveryError::PushPlusExecutionUnavailable);
    }
    let selected_article_ids = accepted
        .iter()
        .map(|selection| selection.article_id)
        .collect::<Vec<_>>();
    let favorite_writes = favorite_writes(config, subscriber, &selected_article_ids);
    if config.mode == DeliveryMode::Execute {
        execute_favorite_writes(config, &favorite_writes)?;
        for article_id in &selected_article_ids {
            delivery_dedupe.insert(
                format!("{}:{article_id}", subscriber.subscriber_id),
                utc_now_iso(),
            );
        }
    }
    let (message_title, message_content, would_send_pushplus) =
        if config.workflow == DeliveryWorkflow::Notify {
            (
                Some(build_message_title(&config.db_name, run_id)),
                Some(build_markdown_content(
                    &config.db_name,
                    run_id,
                    subscriber,
                    "",
                    &accepted,
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
        status: "ok".to_string(),
        error: None,
        selected_article_ids,
        message_title,
        message_content,
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
        favorite_writes: Vec::new(),
        folder_synced_count: 0,
        would_send_pushplus: false,
    }
}

fn favorite_writes(
    config: &RecommendationRunConfig,
    subscriber: &NotificationSubscriberInfo,
    selected_article_ids: &[i64],
) -> Vec<FavoriteWritePlan> {
    let should_write = match config.workflow {
        DeliveryWorkflow::Notify => subscriber.sync_to_tracking_folder,
        DeliveryWorkflow::Push => true,
    };
    if !should_write {
        return Vec::new();
    }
    let Some(folder_id) = subscriber.tracking_folder_id else {
        return Vec::new();
    };
    selected_article_ids
        .iter()
        .map(|article_id| FavoriteWritePlan {
            user_id: subscriber.user_id,
            folder_id,
            article_id: *article_id,
            db_name: config.db_name.clone(),
        })
        .collect()
}

fn execute_favorite_writes(
    config: &RecommendationRunConfig,
    favorite_writes: &[FavoriteWritePlan],
) -> Result<(), DeliveryError> {
    let mut grouped: BTreeMap<(i64, i64), Vec<FavoriteAdd>> = BTreeMap::new();
    for write in favorite_writes {
        grouped
            .entry((write.user_id, write.folder_id))
            .or_default()
            .push(FavoriteAdd {
                article_id: ps_domain::ArticleId(write.article_id),
                db_name: write.db_name.clone(),
                note: String::new(),
            });
    }
    for ((user_id, folder_id), articles) in grouped {
        ps_storage::bulk_add_favorites(
            &config.auth_db_path,
            UserId(user_id),
            folder_id,
            &articles,
        )?;
    }
    Ok(())
}

fn user_result_from_plan(
    workflow: DeliveryWorkflow,
    plan: &SubscriberDeliveryPlan,
) -> RecommendationUserResult {
    RecommendationUserResult {
        subscriber_id: plan.subscriber_id.clone(),
        selected_count: plan.selected_article_ids.len(),
        pushed_count: plan.selected_article_ids.len(),
        folder_synced_count: if workflow == DeliveryWorkflow::Notify && plan.status == "ok" {
            Some(plan.folder_synced_count)
        } else {
            None
        },
        message_id: None,
        status: plan.status.clone(),
        error: plan.error.clone(),
    }
}

fn complete_without_candidates(
    state: &mut RecommendationState,
    run_state: &mut ps_recommend::RecommendationRunState,
    current_issue_counts: &BTreeMap<String, i64>,
    current_inpress_counts: &BTreeMap<String, i64>,
    pending_issue_keys: &[String],
    pending_inpress_keys: &[String],
    now: &str,
) {
    complete_successfully(
        state,
        run_state,
        current_issue_counts,
        current_inpress_counts,
        pending_issue_keys,
        pending_inpress_keys,
        now,
    );
}

fn complete_successfully(
    state: &mut RecommendationState,
    run_state: &mut ps_recommend::RecommendationRunState,
    current_issue_counts: &BTreeMap<String, i64>,
    current_inpress_counts: &BTreeMap<String, i64>,
    pending_issue_keys: &[String],
    pending_inpress_keys: &[String],
    completed_at: &str,
) {
    run_state.status = "completed".to_string();
    run_state.completed_at = Some(completed_at.to_string());
    run_state.updated_at = completed_at.to_string();
    run_state.done_issue_keys = pending_issue_keys.to_vec();
    run_state.done_inpress_keys = pending_inpress_keys.to_vec();
    run_state.pending_issue_keys = Vec::new();
    run_state.pending_inpress_keys = Vec::new();
    state.status = "completed".to_string();
    state.last_completed_run_at = Some(completed_at.to_string());
    state.snapshot.issue_article_counts = current_issue_counts.clone();
    state.snapshot.inpress_article_counts = current_inpress_counts.clone();
    state.updated_at = completed_at.to_string();
    state.run = Some(run_state.clone());
}

fn candidates_by_id(candidates: &[ArticleCandidateInfo]) -> BTreeMap<i64, ArticleCandidateInfo> {
    candidates
        .iter()
        .map(|candidate| (candidate.article_id, candidate.clone()))
        .collect()
}

fn state_path(state_dir: &Path, db_name: &str) -> PathBuf {
    let stem = db_name
        .strip_suffix(".sqlite")
        .unwrap_or(db_name)
        .to_string();
    state_dir.join(format!("{stem}.json"))
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
        ai_base_url: read_env("NOTIFY_AI_BASE_URL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string()),
        ai_api_key: read_env("NOTIFY_AI_API_KEY").unwrap_or_default(),
        pushplus_channel: read_env("NOTIFY_PUSHPLUS_CHANNEL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| PUSHPLUS_CHANNEL.to_string()),
        pushplus_template: read_env("NOTIFY_PUSHPLUS_TEMPLATE")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "markdown".to_string()),
        pushplus_topic: read_env("NOTIFY_PUSHPLUS_TOPIC").filter(|value| !value.is_empty()),
        pushplus_option: read_env("NOTIFY_PUSHPLUS_OPTION").filter(|value| !value.is_empty()),
        ai_system_prompt: read_env("NOTIFY_AI_SYSTEM_PROMPT").filter(|value| !value.is_empty()),
    }
}

fn load_defaults() -> NotificationDefaults {
    NotificationDefaults {
        max_candidates: read_env("NOTIFY_MAX_CANDIDATES")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(120)
            .max(1),
        ai_model: read_env("NOTIFY_AI_MODEL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string()),
        temperature: read_env("NOTIFY_TEMPERATURE")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.2)
            .clamp(0.0, 1.0),
    }
}

fn read_env(name: &str) -> Option<String> {
    env::var(name).ok().map(|value| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use ps_domain::{NotificationSettingsUpdate, UserId};
    use tempfile::{tempdir, TempDir};

    use super::{
        run_recommendation_delivery, DeliveryMode, DeliveryWorkflow, RecommendationRunConfig,
    };

    #[test]
    fn dry_run_push_plans_folder_writes_without_side_effects() {
        let fixture = DeliveryFixture::new(notification_settings("folder", true, vec![]));

        let outcome = run_recommendation_delivery(&fixture.config(
            DeliveryWorkflow::Push,
            DeliveryMode::DryRun,
            None,
            None,
        ))
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
        let state = ps_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
            .expect("state should be written");
        assert_eq!(state.status, "completed");
        assert!(state.delivery_dedupe.is_empty());
    }

    #[test]
    fn shadow_notify_plans_pushplus_without_sending() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));

        let outcome = run_recommendation_delivery(&fixture.config(
            DeliveryWorkflow::Notify,
            DeliveryMode::Shadow,
            None,
            Some(1),
        ))
        .expect("notify shadow run should build a PushPlus plan");

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
        assert_eq!(favorite_count(&fixture.auth_db_path), 0);
    }

    #[test]
    fn execute_notify_fails_before_pushplus_side_effects() {
        let fixture = DeliveryFixture::new(notification_settings("pushplus", true, vec![]));

        let outcome = run_recommendation_delivery(&fixture.config(
            DeliveryWorkflow::Notify,
            DeliveryMode::Execute,
            None,
            None,
        ))
        .expect("notify execute should record PushPlus as a failed subscriber result");

        assert_eq!(outcome.status, "failed");
        assert!(outcome.subscribers.is_empty());
        assert_eq!(favorite_count(&fixture.auth_db_path), 0);
        let state = ps_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
            .expect("state should be written");
        assert!(state
            .run
            .expect("run state should be recorded")
            .errors
            .iter()
            .any(|error| error.contains("PushPlus execution is unavailable")));
    }

    #[test]
    fn execute_push_writes_folder_state_and_dedupe() {
        let fixture = DeliveryFixture::new(notification_settings("folder", true, vec![]));

        let outcome = run_recommendation_delivery(&fixture.config(
            DeliveryWorkflow::Push,
            DeliveryMode::Execute,
            None,
            None,
        ))
        .expect("push execute should write favorites");

        assert_eq!(outcome.status, "completed");
        assert_eq!(outcome.subscribers[0].favorite_writes.len(), 2);
        assert_eq!(favorite_count(&fixture.auth_db_path), 2);
        let state = ps_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
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

        let outcome = run_recommendation_delivery(&fixture.config(
            DeliveryWorkflow::Push,
            DeliveryMode::DryRun,
            Some(changes_file.clone()),
            None,
        ))
        .expect("manifest run should filter candidates");

        assert_eq!(outcome.candidate_article_ids, vec![102]);
        assert_eq!(outcome.subscribers[0].selected_article_ids, vec![102]);

        fs::write(
            &changes_file,
            r#"{"db_name":"other.sqlite","changed_issue_keys":["1:11"],"changed_inpress_journal_ids":[],"notifiable_article_ids":[102]}"#,
        )
        .expect("manifest should be replaced");
        let error = run_recommendation_delivery(&fixture.config(
            DeliveryWorkflow::Push,
            DeliveryMode::DryRun,
            Some(changes_file),
            None,
        ))
        .expect_err("wrong database manifest should be rejected");

        assert!(error.to_string().contains("database mismatch"));
    }

    #[test]
    fn disabled_or_unselected_subscribers_are_skipped() {
        let disabled_fixture = DeliveryFixture::new(notification_settings("folder", false, vec![]));

        let disabled_outcome = run_recommendation_delivery(&disabled_fixture.config(
            DeliveryWorkflow::Push,
            DeliveryMode::DryRun,
            None,
            None,
        ))
        .expect("disabled subscriber run should complete");

        assert_eq!(disabled_outcome.status, "skipped");
        assert!(disabled_outcome.subscribers.is_empty());

        let unselected_fixture = DeliveryFixture::new(notification_settings(
            "folder",
            true,
            vec!["other.sqlite".to_string()],
        ));

        let unselected_outcome = run_recommendation_delivery(&unselected_fixture.config(
            DeliveryWorkflow::Push,
            DeliveryMode::DryRun,
            None,
            None,
        ))
        .expect("unselected database run should complete");

        assert_eq!(unselected_outcome.status, "skipped");
        assert!(unselected_outcome.subscribers.is_empty());
    }

    struct DeliveryFixture {
        root: TempDir,
        auth_db_path: PathBuf,
        index_db_path: PathBuf,
        state_dir: PathBuf,
        db_name: String,
    }

    impl DeliveryFixture {
        fn new(settings: NotificationSettingsUpdate) -> Self {
            let root = tempdir().expect("temp dir should be created");
            let auth_db_path = root.path().join("auth.sqlite");
            ps_storage::initialize_auth_database(&auth_db_path)
                .expect("auth database should initialize");
            let user = ps_storage::register_user_with_invite(
                &auth_db_path,
                "alice",
                "hash",
                "salt",
                None,
                1.0,
            )
            .expect("user should be registered");
            ps_storage::create_folder(&auth_db_path, user.id, "Tracking", true)
                .expect("tracking folder should be created");
            ps_storage::upsert_notification_settings(&auth_db_path, user.id, &settings)
                .expect("notification settings should be saved");
            let index_db_path = root.path().join("fixture.sqlite");
            create_index_database(&index_db_path);
            let state_dir = root.path().join("state");
            Self {
                root,
                auth_db_path,
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
                index_db_path: self.index_db_path.clone(),
                db_name: self.db_name.clone(),
                state_dir: self.state_dir.clone(),
                changes_file,
                ai_model: None,
                max_candidates,
                dedupe_retention_days: 30,
                mode,
                workflow,
            }
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
                "token".to_string()
            } else {
                String::new()
            },
            pushplus_template: "markdown".to_string(),
            pushplus_topic: String::new(),
            pushplus_channel: "wechat".to_string(),
            sync_to_tracking_folder: true,
            ai_base_url: String::new(),
            ai_api_key: "key".to_string(),
            ai_model: "model".to_string(),
            ai_system_prompt: String::new(),
            ai_backup_base_url: String::new(),
            ai_backup_api_key: String::new(),
            ai_backup_model: String::new(),
            ai_backup_system_prompt: String::new(),
            ai_retry_attempts: 1,
            enabled,
        }
    }

    fn create_index_database(path: &Path) {
        let connection =
            ps_storage::open_sqlite_connection(path).expect("index database should open");
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
        ps_storage::count_favorites(auth_db_path, UserId(1), None)
            .expect("favorites should be counted")
    }
}
