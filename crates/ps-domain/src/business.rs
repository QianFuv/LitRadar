//! Business API request and response models for migrated auth database routes.

use std::collections::HashMap;

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
    /// Article publication date.
    pub date: Option<String>,
    /// Article authors text.
    pub authors: Option<String>,
    /// Article abstract text.
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    /// Article DOI.
    pub doi: Option<String>,
    /// Source platform identifier.
    pub platform_id: Option<String>,
    /// Source permalink.
    pub permalink: Option<String>,
    /// Journal title.
    pub journal_title: Option<String>,
    /// Open-access flag from the index database.
    pub open_access: Option<i64>,
    /// In-press flag from the index database.
    pub in_press: Option<i64>,
    /// Issue volume.
    pub volume: Option<String>,
    /// Issue number.
    pub number: Option<String>,
    /// Journal ISSN.
    pub issn: Option<String>,
    /// Journal electronic ISSN.
    pub eissn: Option<String>,
    /// Stored full-text file path.
    pub full_text_file: Option<String>,
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
            date: None,
            authors: None,
            abstract_text: None,
            doi: None,
            platform_id: None,
            permalink: None,
            journal_title: None,
            open_access: None,
            in_press: None,
            volume: None,
            number: None,
            issn: None,
            eissn: None,
            full_text_file: None,
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

/// Notification settings update payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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
    /// PushPlus token.
    #[serde(default)]
    pub pushplus_token: String,
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
    /// Primary AI endpoint API key.
    #[serde(default)]
    pub ai_api_key: String,
    /// Primary AI model.
    #[serde(default)]
    pub ai_model: String,
    /// Primary AI system prompt.
    #[serde(default)]
    pub ai_system_prompt: String,
    /// Backup AI endpoint base URL.
    #[serde(default)]
    pub ai_backup_base_url: String,
    /// Backup AI endpoint API key.
    #[serde(default)]
    pub ai_backup_api_key: String,
    /// Backup AI model.
    #[serde(default)]
    pub ai_backup_model: String,
    /// Backup AI system prompt.
    #[serde(default)]
    pub ai_backup_system_prompt: String,
    /// Retry attempts per AI endpoint.
    #[serde(default = "default_ai_retry_attempts")]
    pub ai_retry_attempts: i64,
    /// Whether recommendations are enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
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
    /// PushPlus token.
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
    /// Primary AI endpoint API key.
    pub ai_api_key: String,
    /// Primary AI model.
    pub ai_model: String,
    /// Primary AI system prompt.
    pub ai_system_prompt: String,
    /// Backup AI endpoint base URL.
    pub ai_backup_base_url: String,
    /// Backup AI endpoint API key.
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

/// Scheduled task response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ScheduledTaskInfo {
    /// Scheduled task row identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Shell command.
    pub command: String,
    /// Five-field cron expression.
    pub cron: String,
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
pub struct ScheduledTaskCreate {
    /// Display name.
    pub name: String,
    /// Shell command.
    pub command: String,
    /// Five-field cron expression.
    pub cron: String,
    /// Whether the task is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Scheduled task update payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ScheduledTaskUpdate {
    /// Optional replacement display name.
    pub name: Option<String>,
    /// Optional replacement shell command.
    pub command: Option<String>,
    /// Optional replacement cron expression.
    pub cron: Option<String>,
    /// Optional enabled flag.
    pub enabled: Option<bool>,
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
    /// Effective setting source.
    pub source: String,
    /// Database update timestamp.
    pub updated_at: Option<f64>,
}

/// Runtime settings update payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSettingsUpdate {
    /// Values keyed by API field name.
    #[serde(default)]
    pub values: HashMap<String, String>,
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

/// Return the default announcement priority.
pub fn default_announcement_priority() -> String {
    "normal".to_string()
}
