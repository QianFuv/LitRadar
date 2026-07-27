//! Notification and tracking delivery worker orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ai::{AiClientError, AiCompletionClient, ReqwestAiTransport};
use crate::pushplus::{PushPlusClient, PushPlusError, PushPlusMessage, ReqwestPushPlusTransport};
use litradar_domain::{
    ArticleCandidateInfo, FavoriteAdd, NotificationSubscriberInfo, RankedSelectionInfo,
    SelectionResultInfo, UserId,
};
use litradar_recommend::{
    apply_selection_rules, build_markdown_content, build_message_title,
    compute_changed_inpress_keys, compute_changed_issue_keys, deduplicate_candidates,
    has_selection_preferences, is_database_selected, load_change_manifest,
    resolve_ai_runtime_configs, utc_now_iso, AiRuntimeConfig, NotificationDefaults,
    NotificationGlobalConfig, RecommendationSnapshot, DEFAULT_OPENAI_BASE_URL,
    DEFAULT_OPENAI_MODEL, MAX_ARTICLES_PER_PUSH, PUSHPLUS_CHANNEL,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod candidates;
mod folder;
mod manifests;
mod manual_job;
mod notify;
mod orchestration;
mod state;

pub use manual_job::run_manual_delivery_job;
pub use orchestration::{
    run_manual_weekly_push, run_recommendation_delivery, run_recommendation_delivery_for_user,
};

/// Total AI HTTP attempts available to one durable manual delivery job.
pub const MANUAL_DELIVERY_AI_REQUEST_BUDGET: usize = 8;

/// Default absolute wall-clock budget for an admitted manual delivery job.
pub const MANUAL_DELIVERY_JOB_DEADLINE_SECONDS: u64 = 10 * 60;

const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Fixed execution-stop reason shared by delivery clients and orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryExecutionControlError {
    /// Durable cancellation was requested.
    Cancelled,
    /// The persisted total job deadline elapsed.
    TimedOut,
    /// The durable cancellation state could not be loaded safely.
    StateUnavailable,
    /// The job exhausted its total AI HTTP request budget.
    AiRequestBudgetExhausted,
}

impl DeliveryExecutionControlError {
    /// Return the stable terminal classification.
    ///
    /// # Returns
    ///
    /// Fixed ASCII error code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::TimedOut => "deadline_exceeded",
            Self::StateUnavailable => "cancellation_state_unavailable",
            Self::AiRequestBudgetExhausted => "ai_request_budget_exhausted",
        }
    }
}

impl fmt::Display for DeliveryExecutionControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "Delivery job was cancelled",
            Self::TimedOut => "Delivery job deadline was exceeded",
            Self::StateUnavailable => "Delivery job cancellation state is unavailable",
            Self::AiRequestBudgetExhausted => "Delivery AI request budget was exhausted",
        })
    }
}

impl Error for DeliveryExecutionControlError {}

/// Cloneable deadline, cancellation, and AI-request budget shared by one manual job.
#[derive(Clone)]
pub struct DeliveryExecutionControl {
    deadline_at: f64,
    remaining_ai_requests: Arc<AtomicUsize>,
    cancellation_probe: Arc<dyn Fn() -> Result<bool, ()> + Send + Sync>,
}

impl fmt::Debug for DeliveryExecutionControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeliveryExecutionControl")
            .field("deadline_at", &self.deadline_at)
            .field(
                "remaining_ai_requests",
                &self.remaining_ai_requests.load(Ordering::Relaxed),
            )
            .field("cancellation_probe", &"[CONFIGURED]")
            .finish()
    }
}

impl DeliveryExecutionControl {
    /// Build one shared durable-job execution boundary.
    ///
    /// # Arguments
    ///
    /// * `deadline_at` - Absolute Unix deadline persisted with the delivery run.
    /// * `ai_request_budget` - Total AI HTTP attempts across endpoints, formats, and rounds.
    /// * `cancellation_probe` - Fail-closed durable cancellation lookup.
    ///
    /// # Returns
    ///
    /// Shared execution control.
    pub fn new(
        deadline_at: f64,
        ai_request_budget: usize,
        cancellation_probe: impl Fn() -> Result<bool, ()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            deadline_at,
            remaining_ai_requests: Arc::new(AtomicUsize::new(ai_request_budget)),
            cancellation_probe: Arc::new(cancellation_probe),
        }
    }

    /// Check durable cancellation and the absolute deadline.
    ///
    /// # Returns
    ///
    /// Empty result while work may continue.
    pub fn check(&self) -> Result<(), DeliveryExecutionControlError> {
        match (self.cancellation_probe)() {
            Ok(true) => return Err(DeliveryExecutionControlError::Cancelled),
            Ok(false) => {}
            Err(()) => return Err(DeliveryExecutionControlError::StateUnavailable),
        }
        if unix_time() >= self.deadline_at {
            return Err(DeliveryExecutionControlError::TimedOut);
        }
        Ok(())
    }

    /// Return the persisted absolute Unix deadline.
    ///
    /// # Returns
    ///
    /// Absolute deadline shared by every child request and database run.
    pub const fn deadline_at(&self) -> f64 {
        self.deadline_at
    }

    /// Reserve one AI request and return its remaining-time-capped timeout.
    ///
    /// # Arguments
    ///
    /// * `default_timeout` - Normal per-request timeout before the job deadline cap.
    ///
    /// # Returns
    ///
    /// Positive request timeout bounded by the remaining total deadline.
    pub fn begin_ai_request(
        &self,
        default_timeout: Duration,
    ) -> Result<Duration, DeliveryExecutionControlError> {
        self.check()?;
        self.remaining_ai_requests
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .map_err(|_| DeliveryExecutionControlError::AiRequestBudgetExhausted)?;
        self.remaining_timeout(default_timeout)
    }

    /// Return a remaining-time-capped timeout for a non-AI external request.
    ///
    /// # Arguments
    ///
    /// * `default_timeout` - Normal per-request timeout before the job deadline cap.
    ///
    /// # Returns
    ///
    /// Positive request timeout bounded by the remaining total deadline.
    pub fn begin_external_request(
        &self,
        default_timeout: Duration,
    ) -> Result<Duration, DeliveryExecutionControlError> {
        self.check()?;
        self.remaining_timeout(default_timeout)
    }

    /// Sleep through a retry delay while polling cancellation and deadline state.
    ///
    /// # Arguments
    ///
    /// * `delay` - Requested bounded retry delay.
    ///
    /// # Returns
    ///
    /// Empty result when the whole delay completed before cancellation or deadline.
    pub fn wait(&self, delay: Duration) -> Result<(), DeliveryExecutionControlError> {
        let started_at = std::time::Instant::now();
        while started_at.elapsed() < delay {
            self.check()?;
            let remaining = delay.saturating_sub(started_at.elapsed());
            thread::sleep(CONTROL_POLL_INTERVAL.min(remaining));
        }
        self.check()
    }

    fn remaining_timeout(
        &self,
        default_timeout: Duration,
    ) -> Result<Duration, DeliveryExecutionControlError> {
        let remaining_seconds = self.deadline_at - unix_time();
        if !remaining_seconds.is_finite() || remaining_seconds <= 0.0 {
            return Err(DeliveryExecutionControlError::TimedOut);
        }
        Ok(default_timeout
            .min(Duration::from_secs_f64(remaining_seconds))
            .max(Duration::from_millis(1)))
    }
}

/// Delivery worker errors.
#[derive(Debug)]
pub enum DeliveryError {
    /// Index storage operation failed.
    Index(litradar_storage::IndexRepositoryError),
    /// Auth database storage operation failed.
    Business(litradar_storage::BusinessRepositoryError),
    /// Durable delivery state operation failed.
    Durable(litradar_storage::DeliveryRepositoryError),
    /// Authentication storage utility failed.
    Auth(litradar_storage::AuthRepositoryError),
    /// Recommendation logic failed.
    Recommendation(litradar_recommend::RecommendationError),
    /// AI selection client failed unexpectedly.
    Ai(String),
    /// PushPlus delivery failed.
    PushPlus(String),
    /// Manual delivery validation failed.
    Manual(String),
    /// Another process owns this workflow/database delivery lease.
    Busy,
    /// Durable job execution was cancelled, timed out, unavailable, or exhausted its budget.
    Control(DeliveryExecutionControlError),
}

impl fmt::Display for DeliveryError {
    /// Format the delivery error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(error) => write!(formatter, "{error}"),
            Self::Business(error) => write!(formatter, "{error}"),
            Self::Durable(error) => write!(formatter, "{error}"),
            Self::Auth(error) => write!(formatter, "{error}"),
            Self::Recommendation(error) => write!(formatter, "{error}"),
            Self::Ai(message) => formatter.write_str(message),
            Self::PushPlus(message) => formatter.write_str(message),
            Self::Manual(message) => formatter.write_str(message),
            Self::Busy => formatter.write_str("Delivery workflow is already running"),
            Self::Control(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for DeliveryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Index(error) => Some(error),
            Self::Business(error) => Some(error),
            Self::Durable(error) => Some(error),
            Self::Auth(error) => Some(error),
            Self::Recommendation(error) => Some(error),
            Self::Control(error) => Some(error),
            Self::Ai(_) | Self::PushPlus(_) | Self::Manual(_) | Self::Busy => None,
        }
    }
}

impl From<litradar_storage::IndexRepositoryError> for DeliveryError {
    /// Convert index repository errors into delivery errors.
    fn from(error: litradar_storage::IndexRepositoryError) -> Self {
        Self::Index(error)
    }
}

impl From<litradar_storage::BusinessRepositoryError> for DeliveryError {
    /// Convert business repository errors into delivery errors.
    fn from(error: litradar_storage::BusinessRepositoryError) -> Self {
        Self::Business(error)
    }
}

impl From<litradar_storage::DeliveryRepositoryError> for DeliveryError {
    /// Convert durable delivery repository errors into delivery errors.
    fn from(error: litradar_storage::DeliveryRepositoryError) -> Self {
        Self::Durable(error)
    }
}

impl From<litradar_storage::AuthRepositoryError> for DeliveryError {
    /// Convert authentication repository errors into delivery errors.
    fn from(error: litradar_storage::AuthRepositoryError) -> Self {
        Self::Auth(error)
    }
}

impl From<litradar_recommend::RecommendationError> for DeliveryError {
    /// Convert recommendation errors into delivery errors.
    fn from(error: litradar_recommend::RecommendationError) -> Self {
        Self::Recommendation(error)
    }
}

impl From<PushPlusError> for DeliveryError {
    /// Convert PushPlus client errors into delivery errors.
    fn from(error: PushPlusError) -> Self {
        Self::PushPlus(error.to_string())
    }
}

impl From<DeliveryExecutionControlError> for DeliveryError {
    /// Convert execution-control stops into delivery errors.
    fn from(error: DeliveryExecutionControlError) -> Self {
        Self::Control(error)
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

/// Durable admission source for one database-scoped delivery run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryTrigger {
    /// Scheduler, CLI, or a durable parent job owns admission.
    Scheduled,
    /// Legacy direct authenticated manual execution owns admission.
    Manual,
}

/// Recommendation worker run configuration.
#[derive(Debug, Clone)]
pub struct RecommendationRunConfig {
    /// Path to `auth.sqlite`.
    pub auth_db_path: PathBuf,
    /// Deployment secret codec.
    pub secret_codec: litradar_storage::SecretCodec,
    /// Path to selected index SQLite database.
    pub index_db_path: PathBuf,
    /// Selected database filename.
    pub db_name: String,
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
    /// Durable database-run admission source.
    pub trigger: DeliveryTrigger,
    /// Optional shared total deadline, cancellation, and request budget.
    pub execution_control: Option<DeliveryExecutionControl>,
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
    /// Durable SQLite delivery run identifier.
    pub delivery_run_id: i64,
    /// Candidate article ids considered by the run.
    pub candidate_article_ids: Vec<i64>,
    /// Per-subscriber delivery plans.
    pub subscribers: Vec<SubscriberDeliveryPlan>,
}

/// Manual weekly push run configuration.
#[derive(Debug, Clone)]
pub struct ManualWeeklyPushConfig {
    /// Storage path configuration.
    pub storage_config: litradar_storage::StorageConfig,
    /// Deployment secret codec.
    pub secret_codec: litradar_storage::SecretCodec,
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
    /// Optional shared total deadline, cancellation, and request budget.
    pub execution_control: Option<DeliveryExecutionControl>,
}

/// Manual weekly push delivery result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

fn unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
