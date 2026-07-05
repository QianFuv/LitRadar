//! Notification and tracking delivery worker orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ai::{live_ai_client, AiClientError, AiCompletionClient, ReqwestAiTransport};
use crate::pushplus::{
    live_pushplus_client, PushPlusClient, PushPlusError, PushPlusMessage, ReqwestPushPlusTransport,
};
use ps_domain::{
    ArticleCandidateInfo, FavoriteAdd, NotificationSubscriberInfo, RankedSelectionInfo,
    SelectionResultInfo, UserId,
};
use ps_recommend::{
    apply_selection_rules, build_markdown_content, build_message_title,
    compute_changed_inpress_keys, compute_changed_issue_keys, create_run_state,
    deduplicate_candidates, has_selection_preferences, is_database_selected, load_change_manifest,
    load_state, prune_delivery_dedupe, resolve_ai_runtime_configs, save_state_atomic, utc_now_iso,
    AiRuntimeConfig, NotificationDefaults, NotificationGlobalConfig, RecommendationState,
    RecommendationUserResult, DEFAULT_OPENAI_BASE_URL, DEFAULT_OPENAI_MODEL, MAX_ARTICLES_PER_PUSH,
    PUSHPLUS_CHANNEL,
};
use serde::Serialize;
use serde_json::Value;

const MAX_AI_SELECTION_ROUNDS: usize = 5;

/// Delivery worker errors.
#[derive(Debug)]
pub enum DeliveryError {
    /// Index storage operation failed.
    Index(ps_storage::IndexRepositoryError),
    /// Auth database storage operation failed.
    Business(ps_storage::BusinessRepositoryError),
    /// Recommendation logic failed.
    Recommendation(ps_recommend::RecommendationError),
    /// AI selection client failed unexpectedly.
    Ai(String),
    /// PushPlus delivery failed.
    PushPlus(String),
    /// Manual delivery validation failed.
    Manual(String),
}

impl fmt::Display for DeliveryError {
    /// Format the delivery error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(error) => write!(formatter, "{error}"),
            Self::Business(error) => write!(formatter, "{error}"),
            Self::Recommendation(error) => write!(formatter, "{error}"),
            Self::Ai(message) => formatter.write_str(message),
            Self::PushPlus(message) => formatter.write_str(message),
            Self::Manual(message) => formatter.write_str(message),
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
            Self::Ai(_) => None,
            Self::PushPlus(_) => None,
            Self::Manual(_) => None,
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

impl From<PushPlusError> for DeliveryError {
    /// Convert PushPlus client errors into delivery errors.
    fn from(error: PushPlusError) -> Self {
        Self::PushPlus(error.to_string())
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
    /// HTTP timeout in seconds for AI and PushPlus requests.
    pub timeout_seconds: u64,
    /// CLI retry attempts for AI and PushPlus requests.
    pub retry_attempts: usize,
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
    /// PushPlus message id returned by execute mode.
    pub message_id: Option<String>,
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

/// Manual weekly push run configuration.
#[derive(Debug, Clone)]
pub struct ManualWeeklyPushConfig {
    /// Storage path configuration.
    pub storage_config: ps_storage::StorageConfig,
    /// User that requested the manual push.
    pub user_id: UserId,
    /// Optional model override.
    pub ai_model: Option<String>,
    /// Optional max-candidates override.
    pub max_candidates: Option<usize>,
    /// HTTP timeout in seconds for AI and PushPlus requests.
    pub timeout_seconds: u64,
    /// Retry attempts for AI and PushPlus requests.
    pub retry_attempts: usize,
    /// Dedupe retention days.
    pub dedupe_retention_days: i64,
}

/// Manual weekly push delivery result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManualWeeklyPushOutcome {
    /// Final run status.
    pub status: String,
    /// Human-readable status message.
    pub message: String,
    /// Number of pushed or tracking-folder-synced articles.
    pub pushed: i64,
    /// Number of selected articles.
    pub selected: i64,
    /// Number of candidate articles considered by AI selection.
    pub total_candidates: Option<i64>,
    /// AI-generated summary text when available.
    pub summary: String,
    /// Tracking folder identifier when applicable.
    pub folder_id: Option<i64>,
    /// Tracking folder name when applicable.
    pub folder_name: Option<String>,
}

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
    let settings = ps_storage::get_notification_settings(
        config.storage_config.auth_db_path(),
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
    let folder =
        ps_storage::get_tracking_folder(config.storage_config.auth_db_path(), config.user_id)?;
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
            .map_err(ps_storage::IndexRepositoryError::from)?;
        outcomes.push(run_recommendation_delivery_for_user(
            &RecommendationRunConfig {
                auth_db_path: config.storage_config.auth_db_path().to_path_buf(),
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
        ps_storage::fetch_candidates_for_article_ids(&config.index_db_path, &pending_article_ids)?
    } else {
        let mut candidates = ps_storage::fetch_candidates_for_issue_keys(
            &config.index_db_path,
            &pending_issue_keys,
        )?;
        candidates.extend(ps_storage::fetch_candidates_for_inpress_keys(
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

fn should_save_subscriber_progress(config: &RecommendationRunConfig) -> bool {
    config.mode == DeliveryMode::Execute
}

#[derive(Debug, Clone, PartialEq)]
struct AiSelectionOutcome {
    accepted: Vec<RankedSelectionInfo>,
    summary: String,
    skip_reason: Option<String>,
}

trait DeliveryAiSelector {
    fn select_for_subscriber(
        &mut self,
        request: DeliveryAiSelectionRequest<'_>,
    ) -> Result<AiSelectionOutcome, DeliveryError>;
}

struct DeliveryAiSelectionRequest<'a> {
    subscriber: &'a NotificationSubscriberInfo,
    global_config: &'a NotificationGlobalConfig,
    defaults: &'a NotificationDefaults,
    override_model: Option<&'a str>,
    candidates_for_model: &'a [ArticleCandidateInfo],
    candidates_by_id: &'a BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &'a BTreeMap<String, String>,
}

trait AiSelectionClient {
    fn select_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        defaults: &NotificationDefaults,
        candidates: &[ArticleCandidateInfo],
    ) -> Result<SelectionResultInfo, String>;

    fn summarize_selected_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        selected_candidates: &[ArticleCandidateInfo],
    ) -> Result<String, String>;
}

trait AiSelectionClientFactory {
    fn build_client(
        &mut self,
        config: &AiRuntimeConfig,
        retry_attempts: usize,
        temperature: f64,
    ) -> Result<Box<dyn AiSelectionClient>, String>;
}

struct DefaultDeliveryAiSelector<F: AiSelectionClientFactory> {
    factory: F,
    retry_attempts: usize,
    max_rounds: usize,
}

impl DefaultDeliveryAiSelector<LiveAiSelectionClientFactory> {
    fn live(timeout_seconds: u64, retry_attempts: usize) -> Self {
        Self {
            factory: LiveAiSelectionClientFactory { timeout_seconds },
            retry_attempts,
            max_rounds: MAX_AI_SELECTION_ROUNDS,
        }
    }
}

impl<F: AiSelectionClientFactory> DefaultDeliveryAiSelector<F> {
    #[cfg(test)]
    fn new(factory: F, retry_attempts: usize, max_rounds: usize) -> Self {
        Self {
            factory,
            retry_attempts,
            max_rounds,
        }
    }
}

impl<F: AiSelectionClientFactory> DeliveryAiSelector for DefaultDeliveryAiSelector<F> {
    fn select_for_subscriber(
        &mut self,
        request: DeliveryAiSelectionRequest<'_>,
    ) -> Result<AiSelectionOutcome, DeliveryError> {
        let DeliveryAiSelectionRequest {
            subscriber,
            global_config,
            defaults,
            override_model,
            candidates_for_model,
            candidates_by_id,
            delivery_dedupe,
        } = request;
        if !has_selection_preferences(subscriber) {
            return Ok(skipped_ai_selection("No keywords or directions configured"));
        }
        let ai_configs =
            resolve_ai_runtime_configs(subscriber, global_config, defaults, override_model);
        if ai_configs.is_empty() {
            return Ok(skipped_ai_selection("AI configuration is unavailable"));
        }
        let effective_retries = self
            .retry_attempts
            .max(usize::try_from(subscriber.ai_retry_attempts.max(0)).unwrap_or(0));
        let mut last_error = String::new();
        for ai_config in ai_configs {
            let mut client =
                match self
                    .factory
                    .build_client(&ai_config, effective_retries, defaults.temperature)
                {
                    Ok(client) => client,
                    Err(error) => {
                        last_error = error;
                        continue;
                    }
                };
            match select_articles_with_retries(
                client.as_mut(),
                subscriber,
                defaults,
                candidates_for_model,
                candidates_by_id,
                delivery_dedupe,
                self.max_rounds,
            ) {
                Ok(selection_result) => {
                    let accepted = apply_selection_rules(
                        &selection_result,
                        subscriber,
                        candidates_by_id,
                        delivery_dedupe,
                    );
                    let mut final_summary = selection_result.summary;
                    if !accepted.is_empty() {
                        let selected_candidates = selected_candidates(&accepted, candidates_by_id);
                        if !selected_candidates.is_empty() {
                            if let Ok(summary) =
                                client.summarize_selected_articles(subscriber, &selected_candidates)
                            {
                                if !summary.trim().is_empty() {
                                    final_summary = summary;
                                }
                            }
                        }
                    }
                    return Ok(AiSelectionOutcome {
                        accepted,
                        summary: final_summary,
                        skip_reason: None,
                    });
                }
                Err(error) => {
                    last_error = error;
                }
            }
        }
        Ok(skipped_ai_selection(&format!(
            "AI selection failed across configured endpoints: {last_error}"
        )))
    }
}

struct LiveAiSelectionClientFactory {
    timeout_seconds: u64,
}

impl AiSelectionClientFactory for LiveAiSelectionClientFactory {
    fn build_client(
        &mut self,
        config: &AiRuntimeConfig,
        retry_attempts: usize,
        temperature: f64,
    ) -> Result<Box<dyn AiSelectionClient>, String> {
        let client = live_ai_client(self.timeout_seconds, retry_attempts, temperature)
            .map_err(|error| error.to_string())?;
        Ok(Box::new(LiveAiSelectionClient {
            config: config.clone(),
            client,
        }))
    }
}

struct LiveAiSelectionClient {
    config: AiRuntimeConfig,
    client: AiCompletionClient<ReqwestAiTransport>,
}

impl AiSelectionClient for LiveAiSelectionClient {
    fn select_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        defaults: &NotificationDefaults,
        candidates: &[ArticleCandidateInfo],
    ) -> Result<SelectionResultInfo, String> {
        self.client
            .select_articles(&self.config, subscriber, defaults, candidates)
            .map_err(ai_client_error)
    }

    fn summarize_selected_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        selected_candidates: &[ArticleCandidateInfo],
    ) -> Result<String, String> {
        self.client
            .summarize_selected_articles(&self.config, subscriber, selected_candidates)
            .map_err(ai_client_error)
    }
}

fn ai_client_error(error: AiClientError) -> String {
    error.to_string()
}

fn skipped_ai_selection(reason: &str) -> AiSelectionOutcome {
    AiSelectionOutcome {
        accepted: Vec::new(),
        summary: String::new(),
        skip_reason: Some(reason.to_string()),
    }
}

fn select_articles_with_retries(
    client: &mut dyn AiSelectionClient,
    subscriber: &NotificationSubscriberInfo,
    defaults: &NotificationDefaults,
    candidates_for_model: &[ArticleCandidateInfo],
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &BTreeMap<String, String>,
    max_rounds: usize,
) -> Result<SelectionResultInfo, String> {
    let mut remaining_candidates = candidates_for_model.to_vec();
    let mut aggregated = BTreeMap::<i64, RankedSelectionInfo>::new();
    let mut summary = String::new();
    for _ in 0..max_rounds.max(1) {
        if remaining_candidates.is_empty() {
            break;
        }
        let round_result = client.select_articles(subscriber, defaults, &remaining_candidates)?;
        if summary.is_empty() && !round_result.summary.trim().is_empty() {
            summary = round_result.summary;
        }
        for selection in round_result.selections {
            match aggregated.get(&selection.article_id) {
                Some(existing) if existing.score >= selection.score => {}
                _ => {
                    aggregated.insert(selection.article_id, selection);
                }
            }
        }
        let merged = merged_selection_result(&summary, &aggregated);
        let accepted =
            apply_selection_rules(&merged, subscriber, candidates_by_id, delivery_dedupe);
        if accepted.len() >= MAX_ARTICLES_PER_PUSH {
            return Ok(merged);
        }
        let selected_ids = aggregated.keys().copied().collect::<BTreeSet<_>>();
        remaining_candidates.retain(|candidate| !selected_ids.contains(&candidate.article_id));
    }
    Ok(merged_selection_result(&summary, &aggregated))
}

fn merged_selection_result(
    summary: &str,
    aggregated: &BTreeMap<i64, RankedSelectionInfo>,
) -> SelectionResultInfo {
    let mut selections = aggregated.values().copied().collect::<Vec<_>>();
    selections.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    SelectionResultInfo {
        summary: summary.to_string(),
        selections,
    }
}

fn selected_candidates(
    accepted: &[RankedSelectionInfo],
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
) -> Vec<ArticleCandidateInfo> {
    accepted
        .iter()
        .filter_map(|selection| candidates_by_id.get(&selection.article_id).cloned())
        .collect()
}

trait DeliveryPushPlusSender {
    fn send(&mut self, message: &PushPlusMessage) -> Result<String, DeliveryError>;
}

struct LiveDeliveryPushPlusSender {
    client: PushPlusClient<ReqwestPushPlusTransport>,
}

impl LiveDeliveryPushPlusSender {
    fn new(timeout_seconds: u64, retry_attempts: usize) -> Result<Self, DeliveryError> {
        Ok(Self {
            client: live_pushplus_client(timeout_seconds, retry_attempts)?,
        })
    }
}

impl DeliveryPushPlusSender for LiveDeliveryPushPlusSender {
    fn send(&mut self, message: &PushPlusMessage) -> Result<String, DeliveryError> {
        self.client.send(message).map_err(DeliveryError::from)
    }
}

fn filtered_subscribers(
    auth_db_path: &Path,
    db_name: &str,
    workflow: DeliveryWorkflow,
    subscriber_user_id: Option<UserId>,
) -> Result<Vec<NotificationSubscriberInfo>, DeliveryError> {
    Ok(ps_storage::list_notification_subscribers(auth_db_path)?
        .into_iter()
        .filter(|subscriber| {
            subscriber_user_id
                .map(|user_id| subscriber.user_id == user_id.value())
                .unwrap_or(true)
        })
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManualWeeklyManifest {
    db_name: String,
    path: PathBuf,
}

fn manual_weekly_manifests(
    project_root: &Path,
    selected_databases: &[String],
) -> Result<Vec<ManualWeeklyManifest>, DeliveryError> {
    let push_state_dir = project_root.join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in
        fs::read_dir(push_state_dir).map_err(|error| DeliveryError::Manual(error.to_string()))?
    {
        let path = entry
            .map_err(|error| DeliveryError::Manual(error.to_string()))?
            .path();
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".changes.json"))
        {
            continue;
        }
        let payload = fs::read_to_string(&path)
            .map_err(|error| DeliveryError::Manual(error.to_string()))
            .and_then(|text| {
                serde_json::from_str::<Value>(&text)
                    .map_err(|error| DeliveryError::Manual(error.to_string()))
            })?;
        let Some(db_name) = manual_manifest_db_name(&payload) else {
            continue;
        };
        if !is_database_selected(selected_databases, &db_name) {
            continue;
        }
        if !manual_manifest_has_notifiable_articles(&payload) {
            continue;
        }
        manifests.push(ManualWeeklyManifest { db_name, path });
    }
    manifests.sort_by(|left, right| {
        left.db_name
            .cmp(&right.db_name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(manifests)
}

fn manual_manifest_db_name(payload: &Value) -> Option<String> {
    let value = payload
        .get("db_name")
        .and_then(Value::as_str)
        .or_else(|| payload.get("db_path").and_then(Value::as_str))?;
    normalize_db_name(value)
}

fn manual_manifest_has_notifiable_articles(payload: &Value) -> bool {
    payload
        .get("notifiable_article_ids")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_i64().is_some()))
}

fn normalize_db_name(value: &str) -> Option<String> {
    let filename = Path::new(value.trim()).file_name()?.to_str()?;
    if filename.is_empty() {
        None
    } else if filename.ends_with(".sqlite") {
        Some(filename.to_string())
    } else {
        Some(format!("{filename}.sqlite"))
    }
}

fn manual_delivery_state_dir(project_root: &Path, workflow: DeliveryWorkflow) -> PathBuf {
    match workflow {
        DeliveryWorkflow::Notify => project_root.join("data").join("push_state"),
        DeliveryWorkflow::Push => project_root.join("data").join("folder_push_state"),
    }
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

fn pushplus_message(
    subscriber: &NotificationSubscriberInfo,
    global_config: &NotificationGlobalConfig,
    title: &str,
    content: &str,
) -> PushPlusMessage {
    PushPlusMessage {
        token: subscriber.pushplus_token.clone(),
        title: title.to_string(),
        content: content.to_string(),
        channel: subscriber
            .channel
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(global_config.pushplus_channel.as_str())
            .to_string(),
        template: subscriber
            .template
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(global_config.pushplus_template.as_str())
            .to_string(),
        topic: subscriber
            .topic
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| global_config.pushplus_topic.clone()),
        option: global_config.pushplus_option.clone(),
        to: None,
    }
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
        message_id: plan.message_id.clone(),
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use ps_domain::{NotificationSettingsUpdate, UserId};
    use tempfile::{tempdir, TempDir};

    use super::*;

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
        let state = ps_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
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
        let state = ps_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
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
        let state = ps_recommend::load_state(&outcome.state_path, &fixture.db_name, "ignored")
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
    fn default_ai_selector_falls_back_to_backup_endpoint() {
        let builds = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let factory = ScriptedAiFactory::new(
            vec![
                ScriptedAiClient::new(
                    vec![Err("primary unavailable".to_string())],
                    Vec::new(),
                    None,
                ),
                ScriptedAiClient::new(
                    vec![Ok(selection_result(&[102], "backup"))],
                    vec![Ok("backup summary".to_string())],
                    None,
                ),
            ],
            builds.clone(),
        );
        let mut selector = DefaultDeliveryAiSelector::new(factory, 1, 5);
        let subscriber = subscriber_info_with_backup();
        let candidates = vec![candidate_info(102)];
        let candidates_by_id = candidates_by_id(&candidates);
        let outcome = selector
            .select_for_subscriber(DeliveryAiSelectionRequest {
                subscriber: &subscriber,
                global_config: &global_config(),
                defaults: &defaults(),
                override_model: None,
                candidates_for_model: &candidates,
                candidates_by_id: &candidates_by_id,
                delivery_dedupe: &BTreeMap::new(),
            })
            .expect("AI selection should succeed through backup");

        assert_eq!(outcome.accepted[0].article_id, 102);
        assert_eq!(outcome.summary, "backup summary");
        assert_eq!(
            builds
                .borrow()
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            vec!["https://primary.test/v1", "https://backup.test/v1"]
        );
    }

    #[test]
    fn default_ai_selector_queries_remaining_candidates_across_rounds() {
        let batch_sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let factory = ScriptedAiFactory::new(
            vec![ScriptedAiClient::new(
                vec![
                    Ok(selection_result(&[101], "round one")),
                    Ok(selection_result(&[102], "")),
                ],
                Vec::new(),
                Some(batch_sizes.clone()),
            )],
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        );
        let mut selector = DefaultDeliveryAiSelector::new(factory, 1, 5);
        let subscriber = subscriber_info();
        let candidates = vec![candidate_info(101), candidate_info(102)];
        let candidates_by_id = candidates_by_id(&candidates);
        let outcome = selector
            .select_for_subscriber(DeliveryAiSelectionRequest {
                subscriber: &subscriber,
                global_config: &global_config(),
                defaults: &defaults(),
                override_model: None,
                candidates_for_model: &candidates,
                candidates_by_id: &candidates_by_id,
                delivery_dedupe: &BTreeMap::new(),
            })
            .expect("AI selection should aggregate rounds");

        assert_eq!(
            outcome
                .accepted
                .iter()
                .map(|selection| selection.article_id)
                .collect::<Vec<_>>(),
            vec![101, 102]
        );
        assert_eq!(*batch_sizes.borrow(), vec![2, 1]);
    }

    struct FixtureDeliveryAiSelector {
        outcomes: Vec<AiSelectionOutcome>,
    }

    impl FixtureDeliveryAiSelector {
        fn new(outcomes: Vec<AiSelectionOutcome>) -> Self {
            Self {
                outcomes: outcomes.into_iter().rev().collect(),
            }
        }
    }

    impl DeliveryAiSelector for FixtureDeliveryAiSelector {
        fn select_for_subscriber(
            &mut self,
            _request: DeliveryAiSelectionRequest<'_>,
        ) -> Result<AiSelectionOutcome, DeliveryError> {
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

    struct ScriptedAiFactory {
        clients: Vec<ScriptedAiClient>,
        builds: std::rc::Rc<std::cell::RefCell<Vec<(String, usize)>>>,
    }

    impl ScriptedAiFactory {
        fn new(
            clients: Vec<ScriptedAiClient>,
            builds: std::rc::Rc<std::cell::RefCell<Vec<(String, usize)>>>,
        ) -> Self {
            Self {
                clients: clients.into_iter().rev().collect(),
                builds,
            }
        }
    }

    impl AiSelectionClientFactory for ScriptedAiFactory {
        fn build_client(
            &mut self,
            config: &AiRuntimeConfig,
            retry_attempts: usize,
            _temperature: f64,
        ) -> Result<Box<dyn AiSelectionClient>, String> {
            self.builds
                .borrow_mut()
                .push((config.base_url.clone(), retry_attempts));
            self.clients
                .pop()
                .map(|client| Box::new(client) as Box<dyn AiSelectionClient>)
                .ok_or_else(|| "missing scripted AI client".to_string())
        }
    }

    struct ScriptedAiClient {
        selections: Vec<Result<SelectionResultInfo, String>>,
        summaries: Vec<Result<String, String>>,
        batch_sizes: Option<std::rc::Rc<std::cell::RefCell<Vec<usize>>>>,
    }

    impl ScriptedAiClient {
        fn new(
            selections: Vec<Result<SelectionResultInfo, String>>,
            summaries: Vec<Result<String, String>>,
            batch_sizes: Option<std::rc::Rc<std::cell::RefCell<Vec<usize>>>>,
        ) -> Self {
            Self {
                selections: selections.into_iter().rev().collect(),
                summaries: summaries.into_iter().rev().collect(),
                batch_sizes,
            }
        }
    }

    impl AiSelectionClient for ScriptedAiClient {
        fn select_articles(
            &mut self,
            _subscriber: &NotificationSubscriberInfo,
            _defaults: &NotificationDefaults,
            candidates: &[ArticleCandidateInfo],
        ) -> Result<SelectionResultInfo, String> {
            if let Some(batch_sizes) = &self.batch_sizes {
                batch_sizes.borrow_mut().push(candidates.len());
            }
            self.selections
                .pop()
                .unwrap_or_else(|| Err("missing scripted selection".to_string()))
        }

        fn summarize_selected_articles(
            &mut self,
            _subscriber: &NotificationSubscriberInfo,
            _selected_candidates: &[ArticleCandidateInfo],
        ) -> Result<String, String> {
            self.summaries.pop().unwrap_or_else(|| Ok(String::new()))
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

    fn selection_result(article_ids: &[i64], summary: &str) -> SelectionResultInfo {
        SelectionResultInfo {
            summary: summary.to_string(),
            selections: article_ids
                .iter()
                .enumerate()
                .map(|(index, article_id)| RankedSelectionInfo {
                    article_id: *article_id,
                    score: 100.0 - index as f64,
                })
                .collect(),
        }
    }

    fn global_config() -> NotificationGlobalConfig {
        NotificationGlobalConfig {
            ai_base_url: "https://primary.test/v1".to_string(),
            ai_api_key: "global-key".to_string(),
            pushplus_channel: "wechat".to_string(),
            pushplus_template: "markdown".to_string(),
            pushplus_topic: None,
            pushplus_option: None,
            ai_system_prompt: None,
        }
    }

    fn defaults() -> NotificationDefaults {
        NotificationDefaults {
            max_candidates: 120,
            ai_model: "model".to_string(),
            temperature: 0.2,
        }
    }

    fn subscriber_info() -> NotificationSubscriberInfo {
        NotificationSubscriberInfo {
            subscriber_id: "1".to_string(),
            user_id: 1,
            name: "Alice".to_string(),
            pushplus_token: "token".to_string(),
            channel: Some("wechat".to_string()),
            keywords: vec!["rust".to_string()],
            directions: vec!["systems".to_string()],
            selected_databases: Vec::new(),
            topic: None,
            template: Some("markdown".to_string()),
            delivery_method: "pushplus".to_string(),
            tracking_folder_id: Some(1),
            sync_to_tracking_folder: true,
            ai_base_url: Some("https://primary.test/v1".to_string()),
            ai_api_key: Some("primary-key".to_string()),
            ai_model: Some("model".to_string()),
            ai_system_prompt: None,
            ai_backup_base_url: None,
            ai_backup_api_key: None,
            ai_backup_model: None,
            ai_backup_system_prompt: None,
            ai_retry_attempts: 1,
        }
    }

    fn subscriber_info_with_backup() -> NotificationSubscriberInfo {
        NotificationSubscriberInfo {
            ai_backup_base_url: Some("https://backup.test/v1".to_string()),
            ai_backup_api_key: Some("backup-key".to_string()),
            ..subscriber_info()
        }
    }

    fn candidate_info(article_id: i64) -> ArticleCandidateInfo {
        ArticleCandidateInfo {
            article_id,
            journal_id: 1,
            issue_id: Some(11),
            title: format!("Rust systems {article_id}"),
            abstract_text: "rust systems".to_string(),
            date: Some("2026-07-01".to_string()),
            journal_title: "Fixture Journal".to_string(),
            doi: Some(format!("10.0000/{article_id}")),
            full_text_file: None,
            permalink: Some(format!("https://example.test/{article_id}")),
            open_access: true,
            in_press: false,
            within_library_holdings: true,
        }
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
                timeout_seconds: 60,
                retry_attempts: 3,
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
