//! Business API request and response models for migrated auth database routes.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{ArticleId, JournalId, UserId};

/// Create-folder request payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FolderCreate {
    /// Folder display name.
    pub name: String,
    /// Whether the folder should become the user's tracking folder.
    #[serde(default)]
    pub is_tracking: bool,
}

/// Rename-folder request payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FolderRename {
    /// Replacement folder display name.
    pub name: String,
}

/// Folder response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FolderResponse {
    /// Folder row identifier.
    pub id: i64,
    /// Folder display name.
    pub name: String,
    /// Whether the folder receives tracking pushes.
    pub is_tracking: bool,
    /// Number of favorite rows in the folder.
    pub article_count: i64,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Favorite creation request payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteAdd {
    /// Article identifier.
    pub article_id: ArticleId,
    /// Source index database name.
    #[serde(default)]
    pub db_name: String,
    /// User note text.
    #[serde(default)]
    pub note: String,
}

/// Favorite article reference used by bulk operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteArticleRef {
    /// Article identifier.
    pub article_id: ArticleId,
    /// Source index database name.
    #[serde(default)]
    pub db_name: String,
}

/// Favorite row response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteResponse {
    /// Favorite row identifier.
    pub id: i64,
    /// Folder row identifier.
    pub folder_id: i64,
    /// Article identifier.
    pub article_id: ArticleId,
    /// Source index database name.
    pub db_name: String,
    /// User note text.
    pub note: String,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Favorite row enriched with optional article metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteArticleResponse {
    /// Favorite row identifier.
    pub id: i64,
    /// Folder row identifier.
    pub folder_id: i64,
    /// Article identifier.
    pub article_id: ArticleId,
    /// Source index database name.
    pub db_name: String,
    /// User note text.
    pub note: String,
    /// Favorite creation timestamp.
    pub created_at: f64,
    /// Journal identifier from the index database.
    pub journal_id: Option<JournalId>,
    /// Issue identifier from the index database.
    pub issue_id: Option<i64>,
    /// Article title from the index database.
    pub title: Option<String>,
    /// Article publication year.
    pub publication_year: Option<i64>,
    /// Article publication date.
    pub date: Option<String>,
    /// Ordered article author names.
    pub authors: Option<Vec<String>>,
    /// Article abstract text.
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    /// Article DOI.
    pub doi: Option<String>,
    /// Journal title.
    pub journal_title: Option<String>,
    /// Open-access flag from the index database.
    pub open_access: Option<bool>,
    /// In-press flag from the index database.
    pub in_press: Option<bool>,
    /// Issue volume.
    pub volume: Option<String>,
    /// Issue number.
    pub number: Option<String>,
    /// Journal ISSN.
    pub issn: Option<String>,
    /// Journal electronic ISSN.
    pub eissn: Option<String>,
}

impl From<FavoriteResponse> for FavoriteArticleResponse {
    /// Convert a favorite row into a metadata-empty article response.
    fn from(favorite: FavoriteResponse) -> Self {
        Self {
            id: favorite.id,
            folder_id: favorite.folder_id,
            article_id: favorite.article_id,
            db_name: favorite.db_name,
            note: favorite.note,
            created_at: favorite.created_at,
            journal_id: None,
            issue_id: None,
            title: None,
            publication_year: None,
            date: None,
            authors: None,
            abstract_text: None,
            doi: None,
            journal_title: None,
            open_access: None,
            in_press: None,
            volume: None,
            number: None,
            issn: None,
            eissn: None,
        }
    }
}

/// Favorite membership row returned by check endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteCheckResponse {
    /// Folder row identifier.
    pub folder_id: i64,
    /// Folder display name.
    pub folder_name: String,
}

/// Batch favorite check request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBatchCheckRequest {
    /// Article identifiers to check.
    pub article_ids: Vec<ArticleId>,
    /// Source index database name.
    #[serde(default)]
    pub db_name: String,
}

/// Batch favorite check response item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBatchCheckResponse {
    /// Article identifier that was checked.
    pub article_id: ArticleId,
    /// Folder memberships for the article.
    pub folders: Vec<FavoriteCheckResponse>,
}

/// Bulk favorite add request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBulkAdd {
    /// Favorite articles to add.
    pub articles: Vec<FavoriteAdd>,
}

/// Bulk favorite remove request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBulkRemove {
    /// Favorite articles to remove.
    pub articles: Vec<FavoriteArticleRef>,
}

/// Bulk favorite move request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBulkMove {
    /// Target folder row identifier.
    pub target_folder_id: i64,
    /// Favorite articles to move.
    pub articles: Vec<FavoriteArticleRef>,
}

/// Bulk operation count response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBulkResult {
    /// Number of affected favorite rows.
    pub count: i64,
}

/// Bulk add response preserving the existing `added` key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteBulkAddResult {
    /// Number of inserted favorite rows.
    pub added: i64,
}

/// Set-tracking-folder request payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TrackingSetRequest {
    /// Folder row identifier to mark as tracking.
    pub folder_id: i64,
}

/// Current favorite tracking folder payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FavoriteTrackingResponse {
    /// Tracking folder identifier, or null when none is configured.
    pub folder_id: Option<i64>,
    /// Tracking folder display name, or null when none is configured.
    pub folder_name: Option<String>,
}

/// Minimum delivery retry count accepted from command-line callers.
pub const DELIVERY_RETRY_ATTEMPTS_MIN: usize = 0;

/// Maximum delivery retry count accepted by executable delivery paths.
pub const DELIVERY_RETRY_ATTEMPTS_MAX: usize = 10;

/// Minimum AI retry count accepted for persisted notification settings.
pub const NOTIFICATION_AI_RETRY_ATTEMPTS_MIN: i64 = 1;

/// Maximum AI retry count accepted for persisted notification settings.
pub const NOTIFICATION_AI_RETRY_ATTEMPTS_MAX: i64 = DELIVERY_RETRY_ATTEMPTS_MAX as i64;

/// Notification settings update payload.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct NotificationSettingsUpdate {
    /// Keyword preferences.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Research direction preferences.
    #[serde(default)]
    pub directions: Vec<String>,
    /// Selected index database names.
    #[serde(default)]
    pub selected_databases: Vec<String>,
    /// Delivery method.
    #[serde(default = "default_delivery_method")]
    pub delivery_method: String,
    /// PushPlus token update: omitted preserves, null clears, and non-empty replaces.
    #[serde(
        default,
        deserialize_with = "deserialize_present_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub pushplus_token: Option<Option<String>>,
    /// PushPlus template.
    #[serde(default = "default_pushplus_template")]
    pub pushplus_template: String,
    /// PushPlus topic.
    #[serde(default)]
    pub pushplus_topic: String,
    /// PushPlus channel.
    #[serde(default = "default_pushplus_channel")]
    pub pushplus_channel: String,
    /// Whether PushPlus delivery also syncs to the tracking folder.
    #[serde(default)]
    pub sync_to_tracking_folder: bool,
    /// Primary AI endpoint base URL.
    #[serde(default)]
    pub ai_base_url: String,
    /// Primary AI API key update: omitted preserves, null clears, and non-empty replaces.
    #[serde(
        default,
        deserialize_with = "deserialize_present_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub ai_api_key: Option<Option<String>>,
    /// Primary AI model.
    #[serde(default)]
    pub ai_model: String,
    /// Primary AI system prompt.
    #[serde(default)]
    pub ai_system_prompt: String,
    /// Backup AI endpoint base URL.
    #[serde(default)]
    pub ai_backup_base_url: String,
    /// Backup AI API key update: omitted preserves, null clears, and non-empty replaces.
    #[serde(
        default,
        deserialize_with = "deserialize_present_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub ai_backup_api_key: Option<Option<String>>,
    /// Backup AI model.
    #[serde(default)]
    pub ai_backup_model: String,
    /// Backup AI system prompt.
    #[serde(default)]
    pub ai_backup_system_prompt: String,
    /// Retry attempts per AI endpoint.
    #[serde(default = "default_ai_retry_attempts")]
    #[schema(minimum = 1, maximum = 10)]
    pub ai_retry_attempts: i64,
    /// Whether recommendations are enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl std::fmt::Debug for NotificationSettingsUpdate {
    /// Format an update without exposing submitted credential values.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NotificationSettingsUpdate([REDACTED])")
    }
}

/// Notification settings response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NotificationSettingsResponse {
    /// Settings row identifier.
    pub id: i64,
    /// User row identifier.
    pub user_id: UserId,
    /// Keyword preferences.
    pub keywords: Vec<String>,
    /// Research direction preferences.
    pub directions: Vec<String>,
    /// Selected index database names.
    pub selected_databases: Vec<String>,
    /// Delivery method.
    pub delivery_method: String,
    /// Whether a PushPlus token is configured.
    pub has_pushplus_token: bool,
    /// Fixed non-secret mask when a PushPlus token is configured.
    pub pushplus_token_mask: String,
    /// PushPlus template.
    pub pushplus_template: String,
    /// PushPlus topic.
    pub pushplus_topic: String,
    /// PushPlus channel.
    pub pushplus_channel: String,
    /// Whether PushPlus delivery also syncs to the tracking folder.
    pub sync_to_tracking_folder: bool,
    /// Primary AI endpoint base URL.
    pub ai_base_url: String,
    /// Whether a primary AI API key is configured.
    pub has_ai_api_key: bool,
    /// Fixed non-secret mask when a primary AI API key is configured.
    pub ai_api_key_mask: String,
    /// Primary AI model.
    pub ai_model: String,
    /// Primary AI system prompt.
    pub ai_system_prompt: String,
    /// Backup AI endpoint base URL.
    pub ai_backup_base_url: String,
    /// Whether a backup AI API key is configured.
    pub has_ai_backup_api_key: bool,
    /// Fixed non-secret mask when a backup AI API key is configured.
    pub ai_backup_api_key_mask: String,
    /// Backup AI model.
    pub ai_backup_model: String,
    /// Backup AI system prompt.
    pub ai_backup_system_prompt: String,
    /// Retry attempts per AI endpoint.
    pub ai_retry_attempts: i64,
    /// Whether recommendations are enabled.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: f64,
    /// Last update timestamp.
    pub updated_at: f64,
}

/// Internal notification settings with decrypted credentials.
#[derive(Clone, PartialEq)]
pub struct NotificationSettings {
    /// Settings row identifier.
    pub id: i64,
    /// User row identifier.
    pub user_id: UserId,
    /// Keyword preferences.
    pub keywords: Vec<String>,
    /// Research direction preferences.
    pub directions: Vec<String>,
    /// Selected index database names.
    pub selected_databases: Vec<String>,
    /// Delivery method.
    pub delivery_method: String,
    /// Decrypted PushPlus token.
    pub pushplus_token: String,
    /// PushPlus template.
    pub pushplus_template: String,
    /// PushPlus topic.
    pub pushplus_topic: String,
    /// PushPlus channel.
    pub pushplus_channel: String,
    /// Whether PushPlus delivery also syncs to the tracking folder.
    pub sync_to_tracking_folder: bool,
    /// Primary AI endpoint base URL.
    pub ai_base_url: String,
    /// Decrypted primary AI endpoint API key.
    pub ai_api_key: String,
    /// Primary AI model.
    pub ai_model: String,
    /// Primary AI system prompt.
    pub ai_system_prompt: String,
    /// Backup AI endpoint base URL.
    pub ai_backup_base_url: String,
    /// Decrypted backup AI endpoint API key.
    pub ai_backup_api_key: String,
    /// Backup AI model.
    pub ai_backup_model: String,
    /// Backup AI system prompt.
    pub ai_backup_system_prompt: String,
    /// Retry attempts per AI endpoint.
    pub ai_retry_attempts: i64,
    /// Whether recommendations are enabled.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: f64,
    /// Last update timestamp.
    pub updated_at: f64,
}

impl std::fmt::Debug for NotificationSettings {
    /// Format internal settings without exposing credential values.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NotificationSettings([REDACTED])")
    }
}

impl From<&NotificationSettings> for NotificationSettingsResponse {
    /// Build a safe public response with fixed credential masks.
    fn from(settings: &NotificationSettings) -> Self {
        let has_pushplus_token = !settings.pushplus_token.is_empty();
        let has_ai_api_key = !settings.ai_api_key.is_empty();
        let has_ai_backup_api_key = !settings.ai_backup_api_key.is_empty();
        Self {
            id: settings.id,
            user_id: settings.user_id,
            keywords: settings.keywords.clone(),
            directions: settings.directions.clone(),
            selected_databases: settings.selected_databases.clone(),
            delivery_method: settings.delivery_method.clone(),
            has_pushplus_token,
            pushplus_token_mask: fixed_secret_mask(has_pushplus_token),
            pushplus_template: settings.pushplus_template.clone(),
            pushplus_topic: settings.pushplus_topic.clone(),
            pushplus_channel: settings.pushplus_channel.clone(),
            sync_to_tracking_folder: settings.sync_to_tracking_folder,
            ai_base_url: settings.ai_base_url.clone(),
            has_ai_api_key,
            ai_api_key_mask: fixed_secret_mask(has_ai_api_key),
            ai_model: settings.ai_model.clone(),
            ai_system_prompt: settings.ai_system_prompt.clone(),
            ai_backup_base_url: settings.ai_backup_base_url.clone(),
            has_ai_backup_api_key,
            ai_backup_api_key_mask: fixed_secret_mask(has_ai_backup_api_key),
            ai_backup_model: settings.ai_backup_model.clone(),
            ai_backup_system_prompt: settings.ai_backup_system_prompt.clone(),
            ai_retry_attempts: settings.ai_retry_attempts,
            enabled: settings.enabled,
            created_at: settings.created_at,
            updated_at: settings.updated_at,
        }
    }
}

fn fixed_secret_mask(is_configured: bool) -> String {
    if is_configured {
        "••••".to_string()
    } else {
        String::new()
    }
}

/// Arguments accepted by an index scheduled job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduledIndexJob {
    /// Optional metadata CSV basename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(
        max_length = 128,
        pattern = r"^[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*\.csv$"
    )]
    pub metadata_file: Option<String>,
    /// Whether notification delivery runs after indexing succeeds.
    #[serde(default)]
    pub notify: bool,
    /// Whether push delivery runs after indexing succeeds.
    #[serde(default)]
    pub push: bool,
}

/// Arguments accepted by a notification or push scheduled job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduledDeliveryJob {
    /// Optional index database basename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(
        max_length = 128,
        pattern = r"^[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*\.sqlite$"
    )]
    pub database: Option<String>,
    /// Optional upper bound for recommendation candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1, maximum = 1000)]
    pub max_candidates: Option<usize>,
}

/// Strictly typed scheduled job specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduledJobSpec {
    /// Refresh the index and optionally run delivery workflows.
    Index(ScheduledIndexJob),
    /// Run notification delivery.
    Notify(ScheduledDeliveryJob),
    /// Run push delivery.
    Push(ScheduledDeliveryJob),
}

/// Scheduled job validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledJobValidationError {
    message: String,
}

impl fmt::Display for ScheduledJobValidationError {
    /// Format the validation error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ScheduledJobValidationError {}

impl ScheduledJobSpec {
    /// Validate every argument against the scheduler allowlist.
    ///
    /// # Returns
    ///
    /// Empty result when every argument is safe and within range.
    pub fn validate(&self) -> Result<(), ScheduledJobValidationError> {
        match self {
            Self::Index(job) => {
                if let Some(metadata_file) = job.metadata_file.as_deref() {
                    validate_scheduled_filename(metadata_file, ".csv", "metadata file")?;
                }
            }
            Self::Notify(job) | Self::Push(job) => {
                if let Some(database) = job.database.as_deref() {
                    validate_scheduled_filename(database, ".sqlite", "database")?;
                }
                if let Some(max_candidates) = job.max_candidates {
                    if !(1..=1_000).contains(&max_candidates) {
                        return Err(scheduled_job_error(
                            "max_candidates must be between 1 and 1000",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_scheduled_filename(
    value: &str,
    extension: &str,
    label: &str,
) -> Result<(), ScheduledJobValidationError> {
    let is_allowed = !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && !value.contains("..")
        && value.ends_with(extension)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        });
    if !is_allowed {
        return Err(scheduled_job_error(&format!(
            "{label} must be a safe {extension} basename"
        )));
    }
    Ok(())
}

fn scheduled_job_error(message: &str) -> ScheduledJobValidationError {
    ScheduledJobValidationError {
        message: message.to_string(),
    }
}

/// Scheduled task timing validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledTaskValidationError {
    message: String,
}

impl fmt::Display for ScheduledTaskValidationError {
    /// Format the validation error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ScheduledTaskValidationError {}

/// Validate scheduler time zone and timeout settings.
///
/// # Arguments
///
/// * `timezone` - Explicit IANA time zone name.
/// * `timeout_seconds` - Maximum task runtime in seconds.
///
/// # Returns
///
/// Empty result when the timing configuration is valid.
pub fn validate_scheduled_task_timing(
    timezone: &str,
    timeout_seconds: u64,
) -> Result<(), ScheduledTaskValidationError> {
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(scheduled_task_error("timezone must be a valid IANA name"));
    }
    if !(1..=86_400).contains(&timeout_seconds) {
        return Err(scheduled_task_error(
            "timeout_seconds must be between 1 and 86400",
        ));
    }
    Ok(())
}

fn scheduled_task_error(message: &str) -> ScheduledTaskValidationError {
    ScheduledTaskValidationError {
        message: message.to_string(),
    }
}

/// Scheduled task response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ScheduledTaskInfo {
    /// Scheduled task row identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Validated job specification, absent only for a migrated legacy row.
    pub job: Option<ScheduledJobSpec>,
    /// Read-only command text retained from a legacy row for administrator review.
    pub legacy_command: Option<String>,
    /// Five-field cron expression.
    pub cron: String,
    /// Explicit IANA time zone used for cron evaluation.
    pub timezone: String,
    /// Maximum execution time in seconds.
    pub timeout_seconds: u64,
    /// Whether missed slots are collapsed to the latest slot.
    pub coalesce: bool,
    /// Whether the task is enabled.
    pub enabled: bool,
    /// Last run timestamp.
    pub last_run_at: Option<f64>,
    /// Last run status.
    pub last_status: String,
    /// Creation timestamp.
    pub created_at: f64,
    /// Last update timestamp.
    pub updated_at: f64,
}

/// Scheduled task creation payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduledTaskCreate {
    /// Display name.
    pub name: String,
    /// Validated job specification.
    pub job: ScheduledJobSpec,
    /// Five-field cron expression.
    pub cron: String,
    /// Explicit IANA time zone used for cron evaluation.
    #[serde(default = "default_scheduler_timezone")]
    pub timezone: String,
    /// Maximum execution time in seconds.
    #[serde(default = "default_scheduler_timeout_seconds")]
    #[schema(minimum = 1, maximum = 86400)]
    pub timeout_seconds: u64,
    /// Whether missed slots are collapsed to the latest slot.
    #[serde(default = "default_enabled")]
    pub coalesce: bool,
    /// Whether the task is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Scheduled task update payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduledTaskUpdate {
    /// Optional replacement display name.
    pub name: Option<String>,
    /// Optional replacement job specification.
    pub job: Option<ScheduledJobSpec>,
    /// Optional replacement cron expression.
    pub cron: Option<String>,
    /// Optional replacement IANA time zone.
    pub timezone: Option<String>,
    /// Optional replacement timeout in seconds.
    #[schema(minimum = 1, maximum = 86400)]
    pub timeout_seconds: Option<u64>,
    /// Optional coalescing flag.
    pub coalesce: Option<bool>,
    /// Optional enabled flag.
    pub enabled: Option<bool>,
}

/// Persisted scheduled task run visible to administrators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ScheduledTaskRunInfo {
    /// Run row identifier.
    pub id: i64,
    /// Scheduled task row identifier.
    pub task_id: i64,
    /// Task name captured when the slot was queued.
    pub task_name: String,
    /// Scheduled UTC Unix timestamp aligned to a minute.
    pub scheduled_for: i64,
    /// Durable run status.
    pub status: String,
    /// Worker identifier currently owning the run.
    pub worker_id: Option<String>,
    /// Claim timestamp.
    pub claimed_at: Option<f64>,
    /// Execution start timestamp.
    pub started_at: Option<f64>,
    /// Terminal or unknown-state timestamp.
    pub finished_at: Option<f64>,
}

/// Persisted scheduler worker heartbeat visible to administrators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SchedulerWorkerInfo {
    /// Stable worker process identifier.
    pub worker_id: String,
    /// Worker start timestamp.
    pub started_at: f64,
    /// Most recent heartbeat timestamp.
    pub heartbeat_at: f64,
    /// Whether the heartbeat is within the health threshold.
    pub is_healthy: bool,
}

/// Administrator scheduler status response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SchedulerStatusResponse {
    /// Last completed scheduler wall-clock check.
    pub last_checked_at: Option<f64>,
    /// Known worker heartbeats ordered from newest to oldest.
    pub workers: Vec<SchedulerWorkerInfo>,
    /// Recent durable task runs ordered from newest to oldest.
    pub recent_runs: Vec<ScheduledTaskRunInfo>,
}

/// Runtime setting response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSettingInfo {
    /// API field name.
    pub field: String,
    /// Human-readable label.
    pub label: String,
    /// Human-readable description.
    pub description: String,
    /// Frontend input type.
    pub input_type: String,
    /// Whether the value contains credentials.
    pub is_secret: bool,
    /// Effective setting value.
    pub value: String,
    /// Whether a credential value is configured.
    pub has_value: bool,
    /// Fixed non-secret mask when a credential value is configured.
    pub masked_value: String,
    /// Individually manageable masked entries for a secret pool.
    #[serde(default)]
    pub secret_items: Vec<RuntimeSecretItemInfo>,
    /// Effective setting source.
    pub source: String,
    /// Database update timestamp.
    pub updated_at: Option<f64>,
}

/// Ordered online Provider configuration with optional catalog overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderOrderConfiguration {
    /// Default Provider order used when a catalog has no explicit override.
    pub default: Vec<String>,
    /// Complete per-catalog replacement orders keyed by canonical catalog stem.
    pub catalogs: BTreeMap<String, Vec<String>>,
}

/// Aggregated capabilities for one logical Provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderCapabilityInfo {
    /// Stable lowercase runtime Provider name.
    pub name: String,
    /// Whether the Provider can build canonical index content.
    pub index_content: bool,
    /// Whether the Provider can resolve an online abstract page.
    pub article_abstract: bool,
    /// Whether the Provider can resolve online full text.
    pub article_full_text: bool,
}

/// Safe catalog file metadata visible to administrators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderCatalogInfo {
    /// Canonical catalog stem shared by CSV and SQLite files.
    pub stem: String,
    /// Metadata CSV filename when present.
    pub csv_filename: Option<String>,
    /// Content SQLite filename when present.
    pub database_filename: Option<String>,
}

/// Administrator Provider and catalog capability response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderCatalogResponse {
    /// Logical Providers and their aggregate capabilities.
    pub providers: Vec<ProviderCapabilityInfo>,
    /// Discovered metadata and content catalogs.
    pub catalogs: Vec<ProviderCatalogInfo>,
}

/// Individually manageable secret-pool item metadata.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSecretItemInfo {
    /// Opaque authenticated reference accepted by a pool removal update.
    pub reference: String,
    /// Human-readable prefix mask that never contains the complete secret.
    pub masked_value: String,
}

impl std::fmt::Debug for RuntimeSecretItemInfo {
    /// Format item metadata without exposing its encrypted reference.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeSecretItemInfo")
            .field("reference", &"[REDACTED]")
            .field("masked_value", &self.masked_value)
            .finish()
    }
}

/// Additions and removals applied to one secret runtime pool.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSecretPoolUpdate {
    /// New plaintext entries to normalize and append.
    #[serde(default)]
    pub add: Vec<String>,
    /// Opaque item references to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

impl std::fmt::Debug for RuntimeSecretPoolUpdate {
    /// Format pool mutations without exposing additions or references.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuntimeSecretPoolUpdate([REDACTED])")
    }
}

/// Runtime settings update payload.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSettingsUpdate {
    /// Values keyed by API field name; null clears and a blank secret preserves.
    #[serde(default)]
    pub values: HashMap<String, Option<String>>,
    /// Incremental secret-pool mutations keyed by API field name.
    #[serde(default)]
    pub secret_pool_updates: HashMap<String, RuntimeSecretPoolUpdate>,
}

impl std::fmt::Debug for RuntimeSettingsUpdate {
    /// Format an update without exposing submitted credential values.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuntimeSettingsUpdate([REDACTED])")
    }
}

/// Internal runtime setting value, including decrypted credentials.
#[derive(Clone, PartialEq)]
pub struct RuntimeSettingValue {
    /// API field name.
    pub field: String,
    /// Effective decrypted or non-secret value.
    pub value: String,
    /// Effective setting source.
    pub source: String,
    /// Database update timestamp.
    pub updated_at: Option<f64>,
}

impl std::fmt::Debug for RuntimeSettingValue {
    /// Format internal settings without exposing credential values.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeSettingValue")
            .field("field", &self.field)
            .field("value", &"[REDACTED]")
            .field("source", &self.source)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Admin user response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AdminUserInfo {
    /// User row identifier.
    pub id: UserId,
    /// Username.
    pub username: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
    /// Creation timestamp.
    pub created_at: f64,
    /// Last update timestamp.
    pub updated_at: f64,
    /// Number of folders owned by the user.
    pub folder_count: i64,
    /// Number of favorites owned by the user.
    pub favorite_count: i64,
    /// Whether enabled notification settings exist.
    pub notify_enabled: bool,
}

/// Admin grant/revoke request payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdminSetAdmin {
    /// Replacement admin flag.
    pub is_admin: bool,
}

/// Admin password reset request payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdminResetPassword {
    /// Replacement password.
    pub new_password: String,
}

/// Admin invite code response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AdminInviteCodeInfo {
    /// Invite code row identifier.
    pub id: i64,
    /// Raw invite code.
    pub code: String,
    /// Creator user identifier.
    pub created_by: Option<UserId>,
    /// Creator username.
    pub created_by_name: Option<String>,
    /// Consumer user identifier.
    pub used_by: Option<UserId>,
    /// Consumer username.
    pub used_by_name: Option<String>,
    /// Consumption timestamp.
    pub used_at: Option<f64>,
    /// Creation timestamp.
    pub created_at: f64,
}

/// Announcement creation payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AnnouncementCreate {
    /// Announcement title.
    pub title: String,
    /// Announcement message body.
    pub message: String,
    /// Priority label.
    #[serde(default = "default_announcement_priority")]
    pub priority: String,
    /// Whether the announcement is visible.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Announcement update payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AnnouncementUpdate {
    /// Optional replacement title.
    pub title: Option<String>,
    /// Optional replacement message.
    pub message: Option<String>,
    /// Optional replacement priority.
    pub priority: Option<String>,
    /// Optional enabled flag.
    pub enabled: Option<bool>,
}

/// Tracking folder summary for `/api/tracking/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TrackingFolderSummary {
    /// Folder row identifier.
    pub id: i64,
    /// Folder display name.
    pub name: String,
}

/// Tracking status response payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TrackingStatusResponse {
    /// Current tracking folder summary.
    pub tracking_folder: Option<TrackingFolderSummary>,
    /// Total folder count.
    pub total_folders: usize,
    /// Number of weekly article ids currently available.
    pub weekly_articles_available: usize,
    /// Whether notification settings exist for the user.
    pub notification_configured: bool,
}

/// Auth database statistics payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AuthStats {
    /// Total registered users.
    pub total_users: i64,
    /// Users with admin privileges.
    pub admin_count: i64,
    /// Total folders.
    pub total_folders: i64,
    /// Total favorites.
    pub total_favorites: i64,
    /// Total invite codes.
    pub total_invite_codes: i64,
    /// Used invite codes.
    pub used_invite_codes: i64,
    /// Unused invite codes.
    pub unused_invite_codes: i64,
    /// Unexpired access tokens.
    pub active_tokens: i64,
    /// Enabled notification settings.
    pub notification_subscribers: i64,
    /// Scheduled task count.
    pub scheduled_tasks: i64,
    /// Enabled announcement count.
    pub active_announcements: i64,
}

/// Per-index database statistics payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IndexDatabaseStats {
    /// Database file name.
    pub db_name: String,
    /// Article count.
    pub articles: i64,
    /// Journal count.
    pub journals: i64,
    /// Issue count.
    pub issues: i64,
    /// Whether the database failed to read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
}

/// Aggregate index statistics payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IndexStats {
    /// Per-database statistics.
    pub databases: Vec<IndexDatabaseStats>,
    /// Total article count.
    pub total_articles: i64,
    /// Total journal count.
    pub total_journals: i64,
}

/// Push-state statistics payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PushStats {
    /// State file stem.
    pub db_name: String,
    /// Push state status.
    pub status: String,
    /// Last completed run timestamp.
    pub last_completed: Option<String>,
    /// Delivered article count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_count: Option<usize>,
    /// User result count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_results: Option<usize>,
}

/// Admin stats response payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdminStatsResponse {
    /// Auth database statistics.
    pub auth: AuthStats,
    /// Index database statistics.
    pub index: IndexStats,
    /// Push-state statistics.
    pub push: Vec<PushStats>,
}

/// Preserve the distinction between a missing secret field and explicit JSON null.
fn deserialize_present_optional<'de, Deserializer>(
    deserializer: Deserializer,
) -> Result<Option<Option<String>>, Deserializer::Error>
where
    Deserializer: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Return the default notification delivery method.
pub fn default_delivery_method() -> String {
    "folder".to_string()
}

/// Return the default PushPlus template.
pub fn default_pushplus_template() -> String {
    "markdown".to_string()
}

/// Return the default PushPlus channel.
pub fn default_pushplus_channel() -> String {
    "wechat".to_string()
}

/// Return the default AI retry attempt count.
pub fn default_ai_retry_attempts() -> i64 {
    3
}

/// Return the default enabled flag.
pub fn default_enabled() -> bool {
    true
}

/// Return the default scheduler time zone.
pub fn default_scheduler_timezone() -> String {
    "UTC".to_string()
}

/// Return the default scheduled task timeout in seconds.
pub fn default_scheduler_timeout_seconds() -> u64 {
    3_600
}

/// Return the default announcement priority.
pub fn default_announcement_priority() -> String {
    "normal".to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        validate_scheduled_task_timing, NotificationSettingsUpdate, ScheduledDeliveryJob,
        ScheduledIndexJob, ScheduledJobSpec,
    };

    #[test]
    fn scheduler_job_spec_accepts_allowlisted_arguments() {
        let index = ScheduledJobSpec::Index(ScheduledIndexJob {
            metadata_file: Some("journals_2026.csv".to_string()),
            notify: true,
            push: true,
        });
        let delivery = ScheduledJobSpec::Notify(ScheduledDeliveryJob {
            database: Some("journals.sqlite".to_string()),
            max_candidates: Some(250),
        });

        index.validate().expect("index arguments should validate");
        delivery
            .validate()
            .expect("delivery arguments should validate");
    }

    #[test]
    fn scheduler_job_spec_rejects_paths_metacharacters_and_invalid_ranges() {
        for metadata_file in [
            "../journals.csv",
            "data/journals.csv",
            "journals.csv && push",
            "journals.txt",
        ] {
            let job = ScheduledJobSpec::Index(ScheduledIndexJob {
                metadata_file: Some(metadata_file.to_string()),
                notify: false,
                push: false,
            });
            assert!(job.validate().is_err(), "{metadata_file} should fail");
        }

        for max_candidates in [0, 1_001] {
            let job = ScheduledJobSpec::Push(ScheduledDeliveryJob {
                database: None,
                max_candidates: Some(max_candidates),
            });
            assert!(job.validate().is_err());
        }
    }

    #[test]
    fn scheduler_job_spec_deserialization_rejects_unknown_kinds_and_fields() {
        assert!(serde_json::from_str::<ScheduledJobSpec>(r#"{"kind":"shell"}"#).is_err());
        assert!(serde_json::from_str::<ScheduledJobSpec>(
            r#"{"kind":"notify","database":"index.sqlite","command":"push"}"#,
        )
        .is_err());
    }

    #[test]
    fn scheduler_timing_requires_iana_timezone_and_bounded_timeout() {
        validate_scheduled_task_timing("Asia/Shanghai", 3_600)
            .expect("valid scheduler timing should pass");

        assert!(validate_scheduled_task_timing("Local", 3_600).is_err());
        assert!(validate_scheduled_task_timing("UTC", 0).is_err());
        assert!(validate_scheduled_task_timing("UTC", 86_401).is_err());
    }

    #[test]
    fn notification_secret_updates_distinguish_missing_null_and_string() {
        let missing = serde_json::from_str::<NotificationSettingsUpdate>("{}")
            .expect("missing secret fields should deserialize");
        let clear = serde_json::from_str::<NotificationSettingsUpdate>(
            r#"{"pushplus_token":null,"ai_api_key":null}"#,
        )
        .expect("null secret fields should deserialize");
        let replace = serde_json::from_str::<NotificationSettingsUpdate>(
            r#"{"pushplus_token":"replacement"}"#,
        )
        .expect("string secret field should deserialize");

        assert_eq!(missing.pushplus_token, None);
        assert_eq!(clear.pushplus_token, Some(None));
        assert_eq!(clear.ai_api_key, Some(None));
        assert_eq!(
            replace.pushplus_token,
            Some(Some("replacement".to_string()))
        );
    }
}
