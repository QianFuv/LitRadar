//! Shared recommendation pipeline logic for notifications and tracking delivery.

use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ps_domain::{
    ArticleCandidateInfo, NotificationSubscriberInfo, RankedSelectionInfo, SelectionResultInfo,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default OpenAI-compatible base URL used by Python notification settings.
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.siliconflow.cn/v1";
/// Default OpenAI-compatible model used by Python notification settings.
pub const DEFAULT_OPENAI_MODEL: &str = "deepseek-ai/DeepSeek-V3";
/// Default PushPlus channel.
pub const PUSHPLUS_CHANNEL: &str = "wechat";
/// Maximum articles per PushPlus message.
pub const MAX_ARTICLES_PER_PUSH: usize = 20;
/// Maximum PushPlus content length.
pub const MAX_PUSH_CONTENT_LENGTH: usize = 18_000;

/// Recommendation logic errors.
#[derive(Debug)]
pub enum RecommendationError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// JSON parsing or encoding failed.
    Json(serde_json::Error),
    /// State file belongs to a different database.
    StateDatabaseMismatch,
    /// Change manifest is invalid.
    InvalidManifest(String),
    /// AI payload cannot be normalized.
    InvalidAiPayload(String),
}

impl fmt::Display for RecommendationError {
    /// Format the recommendation error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::StateDatabaseMismatch => {
                formatter.write_str("State file does not match selected database")
            }
            Self::InvalidManifest(message) => formatter.write_str(message),
            Self::InvalidAiPayload(message) => formatter.write_str(message),
        }
    }
}

impl Error for RecommendationError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for RecommendationError {
    /// Convert IO errors into recommendation errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for RecommendationError {
    /// Convert JSON errors into recommendation errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Snapshot stored in a notification state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationSnapshot {
    /// Article counts grouped by `journal_id:issue_id`.
    #[serde(default)]
    pub issue_article_counts: BTreeMap<String, i64>,
    /// In-press article counts grouped by journal id.
    #[serde(default)]
    pub inpress_article_counts: BTreeMap<String, i64>,
}

impl Default for RecommendationSnapshot {
    /// Build an empty snapshot.
    fn default() -> Self {
        Self {
            issue_article_counts: BTreeMap::new(),
            inpress_article_counts: BTreeMap::new(),
        }
    }
}

/// Persisted notification state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationState {
    /// Source database file name.
    pub db_name: String,
    /// Current state status.
    #[serde(default = "default_idle_status")]
    pub status: String,
    /// Last completed run timestamp.
    #[serde(default)]
    pub last_completed_run_at: Option<String>,
    /// Last known article-count snapshot.
    #[serde(default)]
    pub snapshot: RecommendationSnapshot,
    /// Current run state.
    #[serde(default)]
    pub run: Option<RecommendationRunState>,
    /// Per-subscriber delivery dedupe map.
    #[serde(default)]
    pub delivery_dedupe: BTreeMap<String, String>,
    /// Last state update timestamp.
    #[serde(default)]
    pub updated_at: String,
}

/// Current notification run state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationRunState {
    /// Stable run identifier.
    pub run_id: String,
    /// Run status.
    pub status: String,
    /// Start timestamp.
    pub started_at: String,
    /// Completion timestamp.
    pub completed_at: Option<String>,
    /// Last update timestamp.
    pub updated_at: String,
    /// Issue keys still pending.
    pub pending_issue_keys: Vec<String>,
    /// Completed issue keys.
    pub done_issue_keys: Vec<String>,
    /// In-press journal keys still pending.
    pub pending_inpress_keys: Vec<String>,
    /// Completed in-press journal keys.
    pub done_inpress_keys: Vec<String>,
    /// Candidate article ids considered by the run.
    pub delivered_article_ids: Vec<i64>,
    /// Run-level error messages.
    pub errors: Vec<String>,
    /// Per-subscriber delivery results.
    pub user_results: Vec<RecommendationUserResult>,
}

/// Per-subscriber run result stored in the state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationUserResult {
    /// Subscriber identifier.
    pub subscriber_id: String,
    /// Number of selected articles.
    pub selected_count: usize,
    /// Number of pushed articles.
    pub pushed_count: usize,
    /// Tracking-folder sync count when the workflow records it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_synced_count: Option<usize>,
    /// PushPlus message id.
    pub message_id: Option<String>,
    /// Result status.
    pub status: String,
    /// Error or skip reason.
    pub error: Option<String>,
}

/// Parsed change manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeManifest {
    /// Pending issue keys.
    pub pending_issue_keys: Vec<String>,
    /// Pending in-press journal keys.
    pub pending_inpress_keys: Vec<String>,
    /// Pending article ids.
    pub pending_article_ids: Vec<i64>,
    /// Optional run identifier.
    pub run_id: Option<String>,
}

/// Global notification runtime configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationGlobalConfig {
    /// Default OpenAI-compatible API base URL.
    pub ai_base_url: String,
    /// Default OpenAI-compatible API key.
    pub ai_api_key: String,
    /// Default PushPlus channel.
    pub pushplus_channel: String,
    /// Default PushPlus template.
    pub pushplus_template: String,
    /// Default PushPlus topic.
    pub pushplus_topic: Option<String>,
    /// Default PushPlus option value.
    pub pushplus_option: Option<String>,
    /// Default AI system prompt.
    pub ai_system_prompt: Option<String>,
}

/// Default selection runtime settings.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationDefaults {
    /// Maximum candidates sent to model.
    pub max_candidates: usize,
    /// Default OpenAI-compatible model.
    pub ai_model: String,
    /// Model temperature.
    pub temperature: f64,
}

/// Resolved AI endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRuntimeConfig {
    /// OpenAI-compatible base URL.
    pub base_url: String,
    /// OpenAI-compatible API key.
    pub api_key: String,
    /// OpenAI-compatible model.
    pub model: String,
    /// System prompt.
    pub system_prompt: String,
}

/// Expected AI payload category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPayloadKind {
    /// Article selection payload.
    Selection,
    /// Summary payload.
    Summary,
}

/// Build the default recommendation state.
///
/// # Arguments
///
/// * `db_name` - Database filename.
/// * `now` - Current timestamp.
///
/// # Returns
///
/// Initial state payload.
pub fn build_default_state(db_name: &str, now: &str) -> RecommendationState {
    RecommendationState {
        db_name: db_name.to_string(),
        status: "idle".to_string(),
        last_completed_run_at: None,
        snapshot: RecommendationSnapshot::default(),
        run: None,
        delivery_dedupe: BTreeMap::new(),
        updated_at: now.to_string(),
    }
}

/// Load and normalize a recommendation state file.
///
/// # Arguments
///
/// * `path` - State JSON path.
/// * `db_name` - Selected database filename.
/// * `now` - Current timestamp.
///
/// # Returns
///
/// Loaded or default state.
pub fn load_state(
    path: &Path,
    db_name: &str,
    now: &str,
) -> Result<RecommendationState, RecommendationError> {
    if !path.exists() {
        return Ok(build_default_state(db_name, now));
    }
    let mut state: RecommendationState = serde_json::from_str(&fs::read_to_string(path)?)?;
    if state.db_name != db_name {
        return Err(RecommendationError::StateDatabaseMismatch);
    }
    if state.status.trim().is_empty() {
        state.status = "idle".to_string();
    }
    if state.updated_at.trim().is_empty() {
        state.updated_at = now.to_string();
    }
    if let Some(run) = state.run.as_mut() {
        run.delivered_article_ids
            .retain(|article_id| *article_id > 0);
    }
    Ok(state)
}

/// Save a recommendation state file atomically.
///
/// # Arguments
///
/// * `path` - Destination state path.
/// * `state` - State payload.
///
/// # Returns
///
/// Empty result on success.
pub fn save_state_atomic(
    path: &Path,
    state: &RecommendationState,
) -> Result<(), RecommendationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_file_name(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("state.json")
    ));
    fs::write(&temp_path, serde_json::to_string_pretty(state)?)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

/// Build a run state for pending issue and in-press keys.
///
/// # Arguments
///
/// * `run_id` - Stable run identifier.
/// * `pending_issue_keys` - Pending issue keys.
/// * `pending_inpress_keys` - Pending in-press keys.
/// * `now` - Current timestamp.
///
/// # Returns
///
/// New run state.
pub fn create_run_state(
    run_id: &str,
    pending_issue_keys: Vec<String>,
    pending_inpress_keys: Vec<String>,
    now: &str,
) -> RecommendationRunState {
    RecommendationRunState {
        run_id: run_id.to_string(),
        status: "running".to_string(),
        started_at: now.to_string(),
        completed_at: None,
        updated_at: now.to_string(),
        pending_issue_keys,
        done_issue_keys: Vec::new(),
        pending_inpress_keys,
        done_inpress_keys: Vec::new(),
        delivered_article_ids: Vec::new(),
        errors: Vec::new(),
        user_results: Vec::new(),
    }
}

/// Build an ISO-like UTC timestamp without external time dependencies.
///
/// # Returns
///
/// Current timestamp string.
pub fn utc_now_iso() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs() as i64;
    format_unix_seconds(seconds)
}

/// Compute issue keys whose article counts changed.
///
/// # Arguments
///
/// * `previous_counts` - Previous snapshot.
/// * `current_counts` - Current snapshot.
///
/// # Returns
///
/// Sorted changed issue keys.
pub fn compute_changed_issue_keys(
    previous_counts: &BTreeMap<String, i64>,
    current_counts: &BTreeMap<String, i64>,
) -> Vec<String> {
    let mut changed = current_counts
        .iter()
        .filter(|(key, count)| previous_counts.get(*key) != Some(*count))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    changed.sort_by_key(|key| parse_issue_key(key).unwrap_or((0, 0)));
    changed
}

/// Compute in-press journal keys whose article counts changed.
///
/// # Arguments
///
/// * `previous_counts` - Previous snapshot.
/// * `current_counts` - Current snapshot.
///
/// # Returns
///
/// Sorted changed in-press keys.
pub fn compute_changed_inpress_keys(
    previous_counts: &BTreeMap<String, i64>,
    current_counts: &BTreeMap<String, i64>,
) -> Vec<String> {
    let mut changed = current_counts
        .iter()
        .filter(|(key, count)| previous_counts.get(*key) != Some(*count))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    changed.sort_by_key(|key| key.parse::<i64>().unwrap_or(0));
    changed
}

/// Deduplicate candidates by article id while preserving order.
///
/// # Arguments
///
/// * `candidates` - Candidate list.
///
/// # Returns
///
/// Deduplicated candidate list.
pub fn deduplicate_candidates(candidates: Vec<ArticleCandidateInfo>) -> Vec<ArticleCandidateInfo> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for candidate in candidates {
        if seen.insert(candidate.article_id) {
            deduped.push(candidate);
        }
    }
    deduped
}

/// Load a Python-compatible change manifest.
///
/// # Arguments
///
/// * `path` - Manifest JSON path.
/// * `db_name` - Selected database name.
///
/// # Returns
///
/// Parsed manifest fields used by notification delivery.
pub fn load_change_manifest(
    path: &Path,
    db_name: &str,
) -> Result<ChangeManifest, RecommendationError> {
    let payload: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let object = payload.as_object().ok_or_else(|| {
        RecommendationError::InvalidManifest("Invalid change manifest file".into())
    })?;
    let manifest_db = object
        .get("db_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if !manifest_db.is_empty() && manifest_db != db_name {
        return Err(RecommendationError::InvalidManifest(format!(
            "Change manifest database mismatch: expected {db_name}, got {manifest_db}"
        )));
    }
    let mut pending_issue_keys = object
        .get("changed_issue_keys")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| parse_issue_key(item).is_some())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pending_issue_keys.sort_by_key(|key| parse_issue_key(key).unwrap_or((0, 0)));
    pending_issue_keys.dedup();

    let mut pending_inpress_keys = object
        .get("changed_inpress_journal_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(json_i64)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pending_inpress_keys.sort_by_key(|key| key.parse::<i64>().unwrap_or(0));
    pending_inpress_keys.dedup();

    let article_values = object
        .get("notifiable_article_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            RecommendationError::InvalidManifest(
                "Change manifest missing notifiable_article_ids".into(),
            )
        })?;
    let mut seen_articles = HashSet::new();
    let pending_article_ids = article_values
        .iter()
        .filter_map(json_i64)
        .filter(|article_id| seen_articles.insert(*article_id))
        .collect::<Vec<_>>();
    let run_id = object
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(ChangeManifest {
        pending_issue_keys,
        pending_inpress_keys,
        pending_article_ids,
        run_id,
    })
}

/// Prune old delivery dedupe entries.
///
/// # Arguments
///
/// * `delivery_dedupe` - Dedupe map.
/// * `retention_days` - Retention days.
/// * `now` - Current timestamp.
///
/// # Returns
///
/// Pruned dedupe map.
pub fn prune_delivery_dedupe(
    delivery_dedupe: &BTreeMap<String, String>,
    retention_days: i64,
    now: SystemTime,
) -> BTreeMap<String, String> {
    if retention_days <= 0 {
        return BTreeMap::new();
    }
    let cutoff = now
        .checked_sub(Duration::from_secs((retention_days as u64) * 86_400))
        .unwrap_or(UNIX_EPOCH);
    delivery_dedupe
        .iter()
        .filter_map(|(key, value)| {
            parse_iso_utc_seconds(value)
                .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64)))
                .filter(|timestamp| *timestamp >= cutoff)
                .map(|_| (key.clone(), value.clone()))
        })
        .collect()
}

/// Check whether a subscriber selected a database.
///
/// # Arguments
///
/// * `selected_databases` - Subscriber database selections.
/// * `db_name` - Target database filename.
///
/// # Returns
///
/// True when the subscriber should receive the database.
pub fn is_database_selected(selected_databases: &[String], db_name: &str) -> bool {
    let normalized = normalize_db_name(db_name);
    match normalized {
        Some(target) => {
            selected_databases.is_empty()
                || selected_databases
                    .iter()
                    .filter_map(|value| normalize_db_name(value))
                    .any(|value| value == target)
        }
        None => false,
    }
}

/// Compute keyword and direction match count for a candidate.
///
/// # Arguments
///
/// * `candidate` - Candidate article.
/// * `subscriber` - Subscriber preferences.
///
/// # Returns
///
/// Number of matching preference phrases.
pub fn candidate_match_score(
    candidate: &ArticleCandidateInfo,
    subscriber: &NotificationSubscriberInfo,
) -> i64 {
    let source_text = format!("{} {}", candidate.title, candidate.abstract_text).to_lowercase();
    subscriber
        .keywords
        .iter()
        .chain(subscriber.directions.iter())
        .filter(|phrase| {
            let phrase = phrase.trim().to_lowercase();
            !phrase.is_empty() && source_text.contains(&phrase)
        })
        .count() as i64
}

/// Check whether a subscriber has any selection preferences.
///
/// # Arguments
///
/// * `subscriber` - Subscriber preferences.
///
/// # Returns
///
/// True when keywords or directions contain a non-empty value.
pub fn has_selection_preferences(subscriber: &NotificationSubscriberInfo) -> bool {
    subscriber
        .keywords
        .iter()
        .any(|item| !item.trim().is_empty())
        || subscriber
            .directions
            .iter()
            .any(|item| !item.trim().is_empty())
}

/// Apply Python-compatible local selection rules.
///
/// # Arguments
///
/// * `selection_result` - Model output.
/// * `subscriber` - Subscriber preferences.
/// * `candidates_by_id` - Candidate lookup.
/// * `delivery_dedupe` - Delivery dedupe map.
///
/// # Returns
///
/// Accepted selections.
pub fn apply_selection_rules(
    selection_result: &SelectionResultInfo,
    subscriber: &NotificationSubscriberInfo,
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &BTreeMap<String, String>,
) -> Vec<RankedSelectionInfo> {
    let mut eligible = Vec::new();
    let mut selected_ids = HashSet::new();
    for selection in &selection_result.selections {
        let Some(candidate) = candidates_by_id.get(&selection.article_id) else {
            continue;
        };
        let dedupe_key = delivery_key(subscriber, candidate.article_id);
        if delivery_dedupe.contains_key(&dedupe_key) {
            continue;
        }
        eligible.push(*selection);
        selected_ids.insert(selection.article_id);
    }

    let mut supplemental = Vec::new();
    if eligible.len() < MAX_ARTICLES_PER_PUSH {
        for candidate in candidates_by_id.values() {
            if selected_ids.contains(&candidate.article_id) {
                continue;
            }
            let dedupe_key = delivery_key(subscriber, candidate.article_id);
            if delivery_dedupe.contains_key(&dedupe_key) {
                continue;
            }
            if candidate_match_score(candidate, subscriber) <= 0 {
                continue;
            }
            supplemental.push(RankedSelectionInfo {
                article_id: candidate.article_id,
                score: 0.0,
            });
        }
        supplemental.sort_by(|left, right| {
            let left_candidate = candidates_by_id
                .get(&left.article_id)
                .expect("supplemental candidate should exist");
            let right_candidate = candidates_by_id
                .get(&right.article_id)
                .expect("supplemental candidate should exist");
            (
                candidate_match_score(right_candidate, subscriber),
                right_candidate.article_id,
            )
                .cmp(&(
                    candidate_match_score(left_candidate, subscriber),
                    left_candidate.article_id,
                ))
        });
    }

    let mut merged = eligible;
    merged.extend(supplemental);
    if merged.is_empty() {
        return Vec::new();
    }
    merged.sort_by(|left, right| {
        let left_candidate = candidates_by_id
            .get(&left.article_id)
            .expect("selected candidate should exist");
        let right_candidate = candidates_by_id
            .get(&right.article_id)
            .expect("selected candidate should exist");
        let left_key = (
            candidate_match_score(left_candidate, subscriber),
            ordered_score(left.score),
        );
        let right_key = (
            candidate_match_score(right_candidate, subscriber),
            ordered_score(right.score),
        );
        right_key.cmp(&left_key)
    });
    merged.truncate(MAX_ARTICLES_PER_PUSH);
    merged
}

/// Resolve primary and backup AI runtime configs.
///
/// # Arguments
///
/// * `subscriber` - Subscriber settings.
/// * `global_config` - Global runtime config.
/// * `defaults` - Default model settings.
/// * `override_model` - Optional CLI model override.
///
/// # Returns
///
/// Distinct effective AI endpoint configs.
pub fn resolve_ai_runtime_configs(
    subscriber: &NotificationSubscriberInfo,
    global_config: &NotificationGlobalConfig,
    defaults: &NotificationDefaults,
    override_model: Option<&str>,
) -> Vec<AiRuntimeConfig> {
    let mut configs = Vec::new();
    if let Some(config) = resolve_ai_runtime_config(
        subscriber.ai_base_url.as_deref(),
        subscriber.ai_api_key.as_deref(),
        subscriber.ai_model.as_deref(),
        subscriber.ai_system_prompt.as_deref(),
        global_config,
        defaults,
        override_model,
    ) {
        configs.push(config);
    }
    let has_backup_override = [
        subscriber.ai_backup_base_url.as_deref(),
        subscriber.ai_backup_api_key.as_deref(),
        subscriber.ai_backup_model.as_deref(),
        subscriber.ai_backup_system_prompt.as_deref(),
    ]
    .iter()
    .flatten()
    .any(|value| !value.trim().is_empty());
    if !has_backup_override {
        return configs;
    }
    if let Some(config) = resolve_ai_runtime_config(
        subscriber.ai_backup_base_url.as_deref(),
        subscriber.ai_backup_api_key.as_deref(),
        subscriber.ai_backup_model.as_deref(),
        subscriber.ai_backup_system_prompt.as_deref(),
        global_config,
        defaults,
        override_model,
    ) {
        if !configs.contains(&config) {
            configs.push(config);
        }
    }
    configs
}

/// Build a PushPlus title.
///
/// # Arguments
///
/// * `db_name` - Database filename.
/// * `run_id` - Run identifier.
///
/// # Returns
///
/// Message title.
pub fn build_message_title(db_name: &str, run_id: &str) -> String {
    let prefix = run_id.chars().take(10).collect::<String>();
    format!("Paper Scanner Weekly Update [{db_name}] {prefix}")
}

/// Build markdown push content.
///
/// # Arguments
///
/// * `db_name` - Database filename.
/// * `run_id` - Run identifier.
/// * `subscriber` - Subscriber settings.
/// * `summary` - Selection summary.
/// * `selections` - Accepted selections.
/// * `candidates_by_id` - Candidate lookup.
///
/// # Returns
///
/// Markdown message content.
pub fn build_markdown_content(
    db_name: &str,
    run_id: &str,
    subscriber: &NotificationSubscriberInfo,
    summary: &str,
    selections: &[RankedSelectionInfo],
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
) -> String {
    let mut base_lines = vec![
        format!("## Weekly Digest for {}", subscriber.name),
        String::new(),
        format!("- Database: `{db_name}`"),
        format!("- Run ID: `{run_id}`"),
    ];
    if !summary.trim().is_empty() {
        base_lines.push(String::new());
        base_lines.push(summary.trim().to_string());
    }

    let mut sections = Vec::new();
    for selection in selections.iter().take(MAX_ARTICLES_PER_PUSH) {
        let Some(candidate) = candidates_by_id.get(&selection.article_id) else {
            continue;
        };
        let display_doi = candidate.doi.as_deref().unwrap_or("N/A");
        let date = candidate.date.as_deref().unwrap_or("Unknown");
        let abstract_text = if candidate.abstract_text.trim().is_empty() {
            "N/A"
        } else {
            candidate.abstract_text.trim()
        };
        sections.push(format!(
            "### {}. {}\n- Journal: {}\n- Date: {date}\n- DOI: {display_doi}\n- Abstract: {abstract_text}",
            sections.len() + 1,
            candidate.title,
            candidate.journal_title,
        ));
    }

    let mut kept = Vec::new();
    for section in sections {
        let mut trial = kept.clone();
        trial.push(section.clone());
        if render_content(&base_lines, &trial).len() <= MAX_PUSH_CONTENT_LENGTH {
            kept.push(section);
        }
    }
    let content = render_content(&base_lines, &kept);
    if content.len() <= MAX_PUSH_CONTENT_LENGTH {
        content
    } else {
        truncate_text(&render_content(&base_lines, &[]), MAX_PUSH_CONTENT_LENGTH)
    }
}

/// Extract and normalize an OpenAI-compatible response payload.
///
/// # Arguments
///
/// * `response_json` - OpenAI-compatible response JSON.
/// * `payload_kind` - Expected payload category.
///
/// # Returns
///
/// Normalized payload object.
pub fn extract_response_payload(
    response_json: &Value,
    payload_kind: AiPayloadKind,
) -> Result<Value, RecommendationError> {
    let choices = response_json
        .get("choices")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            RecommendationError::InvalidAiPayload("AI response missing choices".into())
        })?;
    let first_choice = choices.first().and_then(Value::as_object).ok_or_else(|| {
        RecommendationError::InvalidAiPayload("AI response has invalid choice item".into())
    })?;
    let message = first_choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            RecommendationError::InvalidAiPayload("AI response missing message".into())
        })?;
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str) {
        if !refusal.trim().is_empty() {
            return Err(RecommendationError::InvalidAiPayload(format!(
                "AI model refused structured output: {}",
                refusal.trim()
            )));
        }
    }
    if let Some(parsed) = message.get("parsed") {
        return normalize_payload(parsed, payload_kind);
    }
    if let Some(content) = message.get("content") {
        if content.is_object() {
            return normalize_payload(content, payload_kind);
        }
        if let Some(content_array) = content.as_array() {
            let joined = content_array
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<String>();
            return normalize_content_text(&joined, payload_kind);
        }
        if let Some(text) = content.as_str() {
            return normalize_content_text(text, payload_kind);
        }
    }
    Err(RecommendationError::InvalidAiPayload(
        "AI message content is invalid".into(),
    ))
}

fn default_idle_status() -> String {
    "idle".to_string()
}

fn parse_issue_key(key: &str) -> Option<(i64, i64)> {
    let (journal_id, issue_id) = key.split_once(':')?;
    Some((journal_id.parse().ok()?, issue_id.parse().ok()?))
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

fn normalize_db_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        let filename = Path::new(trimmed).file_name()?.to_str()?;
        if filename.ends_with(".sqlite") {
            Some(filename.to_string())
        } else {
            Some(format!("{filename}.sqlite"))
        }
    }
}

fn delivery_key(subscriber: &NotificationSubscriberInfo, article_id: i64) -> String {
    format!("{}:{article_id}", subscriber.subscriber_id)
}

fn ordered_score(score: f64) -> i64 {
    (score * 1_000_000.0).round() as i64
}

fn resolve_ai_runtime_config(
    base_url: Option<&str>,
    api_key: Option<&str>,
    model: Option<&str>,
    system_prompt: Option<&str>,
    global_config: &NotificationGlobalConfig,
    defaults: &NotificationDefaults,
    override_model: Option<&str>,
) -> Option<AiRuntimeConfig> {
    let resolved_api_key = api_key
        .unwrap_or(global_config.ai_api_key.as_str())
        .trim()
        .to_string();
    let resolved_model = override_model
        .or(model)
        .unwrap_or(defaults.ai_model.as_str())
        .trim()
        .to_string();
    if resolved_api_key.is_empty() || resolved_model.is_empty() {
        return None;
    }
    Some(AiRuntimeConfig {
        base_url: base_url
            .unwrap_or(global_config.ai_base_url.as_str())
            .trim()
            .to_string(),
        api_key: resolved_api_key,
        model: resolved_model,
        system_prompt: system_prompt
            .unwrap_or(
                global_config
                    .ai_system_prompt
                    .as_deref()
                    .unwrap_or_default(),
            )
            .trim()
            .to_string(),
    })
}

fn render_content(base_lines: &[String], sections: &[String]) -> String {
    let mut header_lines = base_lines.to_vec();
    header_lines.push(format!("- Selected Articles: {}", sections.len()));
    let mut parts = vec![header_lines.join("\n").trim().to_string()];
    parts.extend(sections.iter().cloned());
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn truncate_text(value: &str, max_length: usize) -> String {
    if value.len() <= max_length {
        return value.to_string();
    }
    value.chars().take(max_length).collect()
}

fn normalize_content_text(
    content: &str,
    payload_kind: AiPayloadKind,
) -> Result<Value, RecommendationError> {
    let mut normalized = content.trim().to_string();
    if normalized.starts_with("```") {
        let mut lines = normalized.lines().collect::<Vec<_>>();
        if lines.first().is_some_and(|line| line.starts_with("```")) {
            lines.remove(0);
        }
        if lines.last().is_some_and(|line| line.starts_with("```")) {
            lines.pop();
        }
        normalized = lines.join("\n").trim().to_string();
    }
    match serde_json::from_str::<Value>(&normalized) {
        Ok(value) => normalize_payload(&value, payload_kind),
        Err(_) => normalize_payload(&Value::String(normalized), payload_kind),
    }
}

fn normalize_payload(
    value: &Value,
    payload_kind: AiPayloadKind,
) -> Result<Value, RecommendationError> {
    match payload_kind {
        AiPayloadKind::Selection => normalize_selection_payload(value),
        AiPayloadKind::Summary => normalize_summary_payload(value),
    }
}

fn normalize_selection_payload(value: &Value) -> Result<Value, RecommendationError> {
    if let Some(items) = value.as_array() {
        return Ok(serde_json::json!({
            "summary": "",
            "selected": coerce_selected_items(items),
        }));
    }
    let Some(object) = value.as_object() else {
        return Err(RecommendationError::InvalidAiPayload(
            "Structured response is not a JSON object".into(),
        ));
    };
    if let Some(selected) = object.get("selected") {
        return Ok(serde_json::json!({
            "summary": extract_summary_value(object),
            "selected": coerce_selected_value(selected),
        }));
    }
    for key in ["items", "results", "recommendations", "articles"] {
        if let Some(selected) = object.get(key) {
            return Ok(serde_json::json!({
                "summary": extract_summary_value(object),
                "selected": coerce_selected_value(selected),
            }));
        }
    }
    if let Some(article_id) = object.get("article_id").and_then(json_i64) {
        let mut item = serde_json::Map::new();
        item.insert("article_id".into(), Value::from(article_id));
        if let Some(score) = object.get("score").and_then(json_f64) {
            item.insert("score".into(), Value::from(score));
        }
        return Ok(serde_json::json!({
            "summary": extract_summary_value(object),
            "selected": [Value::Object(item)],
        }));
    }
    Err(RecommendationError::InvalidAiPayload(
        "Structured response is not a JSON object".into(),
    ))
}

fn normalize_summary_payload(value: &Value) -> Result<Value, RecommendationError> {
    if let Some(text) = value.as_str() {
        if !text.trim().is_empty() {
            return Ok(serde_json::json!({"summary": text.trim()}));
        }
        return Err(RecommendationError::InvalidAiPayload(
            "Structured response is not a JSON object".into(),
        ));
    }
    if let Some(items) = value.as_array() {
        let text_items = items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        if !text_items.is_empty() {
            return Ok(serde_json::json!({"summary": text_items.join("\n")}));
        }
        return Err(RecommendationError::InvalidAiPayload(
            "Structured response is not a JSON object".into(),
        ));
    }
    let Some(object) = value.as_object() else {
        return Err(RecommendationError::InvalidAiPayload(
            "Structured response is not a JSON object".into(),
        ));
    };
    let summary = extract_summary_value(object);
    if !summary.is_empty() {
        return Ok(serde_json::json!({"summary": summary}));
    }
    if object.len() == 1 {
        if let Some(text) = object.values().next().and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Ok(serde_json::json!({"summary": text.trim()}));
            }
        }
    }
    Err(RecommendationError::InvalidAiPayload(
        "Structured response is not a JSON object".into(),
    ))
}

fn extract_summary_value(object: &serde_json::Map<String, Value>) -> String {
    ["summary", "message", "text", "analysis", "reason"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn coerce_selected_value(value: &Value) -> Vec<Value> {
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .filter_map(|(key, item_value)| {
                let article_id = key.parse::<i64>().ok()?;
                let mut item = serde_json::Map::new();
                item.insert("article_id".into(), Value::from(article_id));
                if let Some(score) = json_f64(item_value) {
                    item.insert("score".into(), Value::from(score));
                }
                Some(Value::Object(item))
            })
            .collect();
    }
    value
        .as_array()
        .map_or_else(Vec::new, |items| coerce_selected_items(items))
}

fn coerce_selected_items(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|item| {
            if item.is_object() {
                return Some(item.clone());
            }
            if let Some(article_id) = json_i64(item) {
                return Some(serde_json::json!({"article_id": article_id, "score": 0}));
            }
            if let Some(values) = item.as_array() {
                if values.len() >= 2 {
                    let article_id = values.first().and_then(json_i64)?;
                    let score = values.get(1).and_then(json_f64)?;
                    return Some(serde_json::json!({
                        "article_id": article_id,
                        "score": score,
                    }));
                }
            }
            None
        })
        .collect()
}

fn json_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn format_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn parse_iso_utc_seconds(value: &str) -> Option<i64> {
    let text = value
        .trim()
        .strip_suffix('Z')
        .unwrap_or_else(|| value.trim())
        .strip_suffix("+00:00")
        .unwrap_or_else(|| {
            value
                .trim()
                .strip_suffix('Z')
                .unwrap_or_else(|| value.trim())
        });
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<i64>().ok()?;
    let day = date_parts.next()?.parse::<i64>().ok()?;
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i64>().ok()?;
    let minute = time_parts.next()?.parse::<i64>().ok()?;
    let second_text = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let second = second_text
        .split_once('.')
        .map_or(second_text, |(seconds, _)| seconds)
        .parse::<i64>()
        .ok()?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ps_domain::{
        ArticleCandidateInfo, NotificationSubscriberInfo, RankedSelectionInfo, SelectionResultInfo,
    };
    use serde_json::json;

    use super::{
        apply_selection_rules, build_markdown_content, candidate_match_score,
        deduplicate_candidates, extract_response_payload, has_selection_preferences,
        is_database_selected, resolve_ai_runtime_configs, AiPayloadKind, NotificationDefaults,
        NotificationGlobalConfig, MAX_ARTICLES_PER_PUSH, MAX_PUSH_CONTENT_LENGTH,
    };

    #[test]
    fn normalizes_ai_selection_payload_variants() {
        let payload = json!({
            "choices": [{
                "message": {
                    "content": "```json\n{\"summary\":\"ok\",\"recommendations\":{\"42\":88}}\n```"
                }
            }]
        });

        let normalized = extract_response_payload(&payload, AiPayloadKind::Selection)
            .expect("selection payload should normalize");

        assert_eq!(normalized["summary"], "ok");
        assert_eq!(normalized["selected"][0]["article_id"], 42);
        assert_eq!(normalized["selected"][0]["score"], 88.0);
    }

    #[test]
    fn rejects_malformed_selection_text_like_python() {
        let payload = json!({
            "choices": [{
                "message": {"content": "not json"}
            }]
        });

        let error = extract_response_payload(&payload, AiPayloadKind::Selection)
            .expect_err("selection text should not normalize");

        assert!(error.to_string().contains("Structured response"));
    }

    #[test]
    fn applies_keyword_fallback_and_delivery_dedupe() {
        let subscriber = subscriber();
        let candidates = candidates()
            .into_iter()
            .map(|candidate| (candidate.article_id, candidate))
            .collect::<BTreeMap<_, _>>();
        let mut dedupe = BTreeMap::new();
        dedupe.insert("1:3".to_string(), "2026-07-03T00:00:00Z".to_string());

        let accepted = apply_selection_rules(
            &SelectionResultInfo {
                summary: String::new(),
                selections: vec![RankedSelectionInfo {
                    article_id: 2,
                    score: 10.0,
                }],
            },
            &subscriber,
            &candidates,
            &dedupe,
        );

        assert_eq!(
            accepted
                .iter()
                .map(|item| item.article_id)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn filters_missing_deduped_and_over_limit_candidates() {
        let subscriber = subscriber();
        let candidates = (1..=25)
            .map(|article_id| candidate(article_id, &format!("Rust article {article_id}")))
            .map(|candidate| (candidate.article_id, candidate))
            .collect::<BTreeMap<_, _>>();
        let mut dedupe = BTreeMap::new();
        dedupe.insert("1:2".to_string(), "2026-07-03T00:00:00Z".to_string());
        let selections = vec![
            RankedSelectionInfo {
                article_id: 1,
                score: 1.0,
            },
            RankedSelectionInfo {
                article_id: 2,
                score: 99.0,
            },
            RankedSelectionInfo {
                article_id: 999,
                score: 99.0,
            },
        ];

        let accepted = apply_selection_rules(
            &SelectionResultInfo {
                summary: String::new(),
                selections,
            },
            &subscriber,
            &candidates,
            &dedupe,
        );

        assert_eq!(accepted.len(), MAX_ARTICLES_PER_PUSH);
        assert_eq!(accepted[0].article_id, 1);
        assert!(!accepted.iter().any(|item| item.article_id == 2));
        assert!(!accepted.iter().any(|item| item.article_id == 999));
    }

    #[test]
    fn normalizes_summary_payloads_and_rejects_refusals() {
        let summary_payload = json!({
            "choices": [{
                "message": {"content": [{"type": "text", "text": "summary line"}]}
            }]
        });

        let summary = extract_response_payload(&summary_payload, AiPayloadKind::Summary)
            .expect("summary payload should normalize");

        assert_eq!(summary["summary"], "summary line");

        let refusal_payload = json!({
            "choices": [{
                "message": {"refusal": "cannot comply", "content": "{}"}
            }]
        });

        let error = extract_response_payload(&refusal_payload, AiPayloadKind::Selection)
            .expect_err("refusals should be rejected");

        assert!(error.to_string().contains("refused"));
    }

    #[test]
    fn resolves_ai_configs_with_backup_dedupe_and_override() {
        let mut subscriber = subscriber();
        subscriber.ai_base_url = Some("https://primary.test".to_string());
        subscriber.ai_api_key = Some("subscriber-key".to_string());
        subscriber.ai_model = Some("subscriber-model".to_string());
        subscriber.ai_backup_base_url = Some("https://primary.test".to_string());
        subscriber.ai_backup_api_key = Some("subscriber-key".to_string());
        subscriber.ai_backup_model = Some("subscriber-model".to_string());
        let global_config = NotificationGlobalConfig {
            ai_base_url: "https://global.test".to_string(),
            ai_api_key: "global-key".to_string(),
            pushplus_channel: "wechat".to_string(),
            pushplus_template: "markdown".to_string(),
            pushplus_topic: None,
            pushplus_option: None,
            ai_system_prompt: None,
        };
        let defaults = NotificationDefaults {
            ai_model: "default-model".to_string(),
            temperature: 0.2,
            max_candidates: 20,
        };

        let configs =
            resolve_ai_runtime_configs(&subscriber, &global_config, &defaults, Some("override"));

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].base_url, "https://primary.test");
        assert_eq!(configs[0].api_key, "subscriber-key");
        assert_eq!(configs[0].model, "override");
    }

    #[test]
    fn formats_content_with_boundaries_and_missing_candidate_skip() {
        let subscriber = subscriber();
        let mut candidates = BTreeMap::new();
        let mut long_candidate = candidate(1, "Rust systems");
        long_candidate.abstract_text = "x".repeat(MAX_PUSH_CONTENT_LENGTH);
        candidates.insert(1, long_candidate);
        let content = build_markdown_content(
            "fixture.sqlite",
            "run-123",
            &subscriber,
            "summary",
            &[
                RankedSelectionInfo {
                    article_id: 1,
                    score: 1.0,
                },
                RankedSelectionInfo {
                    article_id: 404,
                    score: 1.0,
                },
            ],
            &candidates,
        );

        assert!(content.len() <= MAX_PUSH_CONTENT_LENGTH);
        assert!(content.contains("Selected Articles: 0"));
        assert!(content.contains("summary"));
        assert!(!content.contains("Rust systems"));
    }

    #[test]
    fn preference_and_database_helpers_match_delivery_rules() {
        let mut subscriber = subscriber();
        subscriber.directions = vec!["systems".to_string()];
        let empty_subscriber = NotificationSubscriberInfo {
            keywords: vec![" ".to_string()],
            directions: Vec::new(),
            ..subscriber.clone()
        };

        assert!(has_selection_preferences(&subscriber));
        assert!(!has_selection_preferences(&empty_subscriber));
        assert_eq!(
            candidate_match_score(&candidate(1, "Rust systems"), &subscriber),
            2
        );
        assert!(is_database_selected(&[], "fixture.sqlite"));
        assert!(is_database_selected(
            &["fixture".to_string()],
            "/data/fixture.sqlite"
        ));
        assert!(!is_database_selected(
            &["other.sqlite".to_string()],
            "fixture.sqlite"
        ));
        assert!(!is_database_selected(&["fixture.sqlite".to_string()], ""));
    }

    #[test]
    fn deduplicates_candidates_preserving_first_seen_order() {
        let deduplicated = deduplicate_candidates(vec![
            candidate(2, "Second first"),
            candidate(1, "First"),
            candidate(2, "Second duplicate"),
        ]);

        assert_eq!(
            deduplicated
                .iter()
                .map(|candidate| candidate.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Second first", "First"]
        );
    }

    fn subscriber() -> NotificationSubscriberInfo {
        NotificationSubscriberInfo {
            subscriber_id: "1".to_string(),
            user_id: 1,
            name: "alice".to_string(),
            pushplus_token: String::new(),
            channel: None,
            keywords: vec!["rust".to_string()],
            directions: Vec::new(),
            selected_databases: Vec::new(),
            topic: None,
            template: None,
            delivery_method: "folder".to_string(),
            tracking_folder_id: Some(1),
            sync_to_tracking_folder: false,
            ai_base_url: None,
            ai_api_key: Some("key".to_string()),
            ai_model: Some("model".to_string()),
            ai_system_prompt: None,
            ai_backup_base_url: None,
            ai_backup_api_key: None,
            ai_backup_model: None,
            ai_backup_system_prompt: None,
            ai_retry_attempts: 1,
        }
    }

    fn candidates() -> Vec<ArticleCandidateInfo> {
        vec![
            candidate(1, "Rust systems"),
            candidate(2, "Rust migration"),
            candidate(3, "Rust deduped"),
        ]
    }

    fn candidate(article_id: i64, title: &str) -> ArticleCandidateInfo {
        ArticleCandidateInfo {
            article_id,
            journal_id: 1,
            issue_id: Some(1),
            title: title.to_string(),
            abstract_text: "rust contract".to_string(),
            date: None,
            journal_title: "Journal".to_string(),
            doi: None,
            full_text_file: None,
            permalink: None,
            open_access: false,
            in_press: false,
            within_library_holdings: true,
        }
    }
}
