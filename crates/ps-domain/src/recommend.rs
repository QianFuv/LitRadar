//! Recommendation and notification delivery compatibility models.

use serde::{Deserialize, Serialize};

/// Candidate article used by notification and tracking delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleCandidateInfo {
    /// Article identifier.
    pub article_id: i64,
    /// Journal identifier.
    pub journal_id: i64,
    /// Issue identifier when available.
    pub issue_id: Option<i64>,
    /// Article title.
    pub title: String,
    /// Article abstract.
    pub abstract_text: String,
    /// Publication date text.
    pub date: Option<String>,
    /// Journal title.
    pub journal_title: String,
    /// DOI value.
    pub doi: Option<String>,
    /// Stored full-text URL or path.
    pub full_text_file: Option<String>,
    /// External article permalink.
    pub permalink: Option<String>,
    /// Whether the article is open access.
    pub open_access: bool,
    /// Whether the article is in press.
    pub in_press: bool,
    /// Whether the article is inside library holdings.
    pub within_library_holdings: bool,
}

/// Notification subscriber row with tracking-folder metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSubscriberInfo {
    /// Stable subscriber identifier.
    pub subscriber_id: String,
    /// User row identifier.
    pub user_id: i64,
    /// Display name.
    pub name: String,
    /// PushPlus token.
    pub pushplus_token: String,
    /// PushPlus channel override.
    pub channel: Option<String>,
    /// Keyword preferences.
    pub keywords: Vec<String>,
    /// Research direction preferences.
    pub directions: Vec<String>,
    /// Selected database names. Empty means all databases.
    pub selected_databases: Vec<String>,
    /// PushPlus topic override.
    pub topic: Option<String>,
    /// PushPlus template override.
    pub template: Option<String>,
    /// Delivery method, either `pushplus` or `folder`.
    pub delivery_method: String,
    /// Tracking folder id when configured.
    pub tracking_folder_id: Option<i64>,
    /// Whether PushPlus delivery also writes tracking favorites.
    pub sync_to_tracking_folder: bool,
    /// Primary OpenAI-compatible API base URL.
    pub ai_base_url: Option<String>,
    /// Primary OpenAI-compatible API key.
    pub ai_api_key: Option<String>,
    /// Primary OpenAI-compatible model.
    pub ai_model: Option<String>,
    /// Primary OpenAI-compatible system prompt.
    pub ai_system_prompt: Option<String>,
    /// Backup OpenAI-compatible API base URL.
    pub ai_backup_base_url: Option<String>,
    /// Backup OpenAI-compatible API key.
    pub ai_backup_api_key: Option<String>,
    /// Backup OpenAI-compatible model.
    pub ai_backup_model: Option<String>,
    /// Backup OpenAI-compatible system prompt.
    pub ai_backup_system_prompt: Option<String>,
    /// Retry attempts per AI endpoint.
    pub ai_retry_attempts: i64,
}

/// Ranked article selection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankedSelectionInfo {
    /// Selected article identifier.
    pub article_id: i64,
    /// Model or fallback score.
    pub score: f64,
}

/// Structured article selection result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionResultInfo {
    /// Selection summary.
    pub summary: String,
    /// Ranked selections.
    pub selections: Vec<RankedSelectionInfo>,
}
