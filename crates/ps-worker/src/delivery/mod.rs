//! Notification and tracking delivery worker orchestration.

use std::collections::{BTreeMap, BTreeSet};
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
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod candidates;
mod folder;
mod manifests;
mod notify;
mod orchestration;
mod state;

pub use orchestration::{
    run_manual_weekly_push, run_recommendation_delivery, run_recommendation_delivery_for_user,
};

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
    /// Deployment secret codec.
    pub secret_codec: ps_storage::SecretCodec,
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
    /// Deployment secret codec.
    pub secret_codec: ps_storage::SecretCodec,
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
