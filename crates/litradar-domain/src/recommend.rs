//! Recommendation and notification delivery compatibility models.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Candidate article used by notification and tracking delivery.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Whether the article is open access.
    pub open_access: bool,
    /// Whether the article is in press.
    pub in_press: bool,
}

impl std::fmt::Debug for ArticleCandidateInfo {
    /// Format candidate metadata without exposing article content.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArticleCandidateInfo")
            .field("article_id", &self.article_id)
            .field("journal_id", &self.journal_id)
            .field("issue_id", &self.issue_id)
            .field("content", &"[REDACTED]")
            .field("open_access", &self.open_access)
            .field("in_press", &self.in_press)
            .finish()
    }
}

/// Notification subscriber row with tracking-folder metadata.
#[derive(Clone, PartialEq, Eq)]
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

impl std::fmt::Debug for NotificationSubscriberInfo {
    /// Format subscriber metadata without exposing integration credentials.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotificationSubscriberInfo")
            .field("subscriber_id", &self.subscriber_id)
            .field("user_id", &self.user_id)
            .field("name", &"[REDACTED]")
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
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
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionResultInfo {
    /// Selection summary.
    pub summary: String,
    /// Ranked selections.
    pub selections: Vec<RankedSelectionInfo>,
}

impl std::fmt::Debug for SelectionResultInfo {
    /// Format a selection without exposing model-generated summary content.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelectionResultInfo")
            .field("summary", &"[REDACTED]")
            .field("selection_count", &self.selections.len())
            .finish()
    }
}

/// Manual weekly push job status payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ManualWeeklyPushStatus {
    /// Background job identifier.
    pub job_id: Option<String>,
    /// Job status: `idle`, `running`, `completed`, or `failed`.
    pub status: String,
    /// Human-readable status message.
    pub message: String,
    /// Unix timestamp when the job started.
    pub started_at: Option<f64>,
    /// Unix timestamp when the job finished.
    pub finished_at: Option<f64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_debug_omits_user_article_and_model_content() {
        let candidate = ArticleCandidateInfo {
            article_id: 1,
            journal_id: 2,
            issue_id: Some(3),
            title: "article-title-sentinel".to_string(),
            abstract_text: "article-abstract-sentinel".to_string(),
            date: Some("2026-07-27".to_string()),
            journal_title: "journal-title-sentinel".to_string(),
            doi: Some("doi-sentinel".to_string()),
            open_access: true,
            in_press: false,
        };
        let subscriber = NotificationSubscriberInfo {
            subscriber_id: "subscriber-1".to_string(),
            user_id: 1,
            name: "subscriber-name-sentinel".to_string(),
            pushplus_token: "pushplus-sentinel".to_string(),
            channel: None,
            keywords: vec!["keyword-sentinel".to_string()],
            directions: vec!["direction-sentinel".to_string()],
            selected_databases: Vec::new(),
            topic: None,
            template: None,
            delivery_method: "folder".to_string(),
            tracking_folder_id: None,
            sync_to_tracking_folder: false,
            ai_base_url: None,
            ai_api_key: Some("ai-key-sentinel".to_string()),
            ai_model: None,
            ai_system_prompt: Some("prompt-sentinel".to_string()),
            ai_backup_base_url: None,
            ai_backup_api_key: None,
            ai_backup_model: None,
            ai_backup_system_prompt: Some("backup-prompt-sentinel".to_string()),
            ai_retry_attempts: 1,
        };
        let selection = SelectionResultInfo {
            summary: "model-summary-sentinel".to_string(),
            selections: vec![RankedSelectionInfo {
                article_id: 1,
                score: 90.0,
            }],
        };

        let debug = format!("{candidate:?} {subscriber:?} {selection:?}");

        for sentinel in [
            "article-title-sentinel",
            "article-abstract-sentinel",
            "journal-title-sentinel",
            "doi-sentinel",
            "subscriber-name-sentinel",
            "pushplus-sentinel",
            "keyword-sentinel",
            "direction-sentinel",
            "ai-key-sentinel",
            "prompt-sentinel",
            "backup-prompt-sentinel",
            "model-summary-sentinel",
        ] {
            assert!(!debug.contains(sentinel));
        }
        assert!(debug.contains("[REDACTED]"));
    }
}
