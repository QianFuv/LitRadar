//! Change manifest data types retained for JSON compatibility.

use serde::Serialize;

/// Python-compatible change manifest payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeManifest {
    /// Run identifier.
    pub run_id: String,
    /// Manifest generation timestamp.
    pub generated_at: String,
    /// Index database filename.
    pub db_name: String,
    /// Changed issue keys.
    pub changed_issue_keys: Vec<String>,
    /// Changed in-press journal ids.
    pub changed_inpress_journal_ids: Vec<i64>,
    /// Article ids eligible for notification.
    pub notifiable_article_ids: Vec<i64>,
    /// Backfill issue keys.
    pub backfill_issue_keys: Vec<String>,
    /// Backfill in-press journal ids.
    pub backfill_inpress_journal_ids: Vec<i64>,
    /// Backfill article ids.
    pub backfill_article_ids: Vec<i64>,
    /// Change summary.
    pub summary: ChangeSummary,
}

/// Change manifest summary payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeSummary {
    /// Changed issue count.
    pub changed_issue_count: usize,
    /// Changed in-press journal count.
    pub changed_inpress_count: usize,
    /// Added article count.
    pub added_article_count: usize,
    /// Removed article count.
    pub removed_article_count: usize,
    /// Added article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub added_article_ids: Vec<i64>,
    /// Removed article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub removed_article_ids: Vec<i64>,
    /// Changed issue details.
    pub issues: Vec<IssueChangeDetail>,
    /// Changed in-press details.
    pub inpress: Vec<InpressChangeDetail>,
    /// Raw changed issue count.
    pub raw_changed_issue_count: usize,
    /// Raw changed in-press count.
    pub raw_changed_inpress_count: usize,
    /// Backfill article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub backfill_article_ids: Vec<i64>,
    /// Backfill article count.
    pub backfill_article_count: usize,
    /// Backfill issue keys.
    pub backfill_issue_keys: Vec<String>,
    /// Backfill in-press journal ids.
    pub backfill_inpress_journal_ids: Vec<i64>,
}

/// Issue-level change detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueChangeDetail {
    /// Issue key.
    pub issue_key: String,
    /// Before article count.
    pub before_count: usize,
    /// After article count.
    pub after_count: usize,
    /// Added article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub added_article_ids: Vec<i64>,
    /// Removed article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub removed_article_ids: Vec<i64>,
    /// Notifiable article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub notifiable_added_article_ids: Vec<i64>,
    /// Backfill article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub backfill_added_article_ids: Vec<i64>,
}

/// In-press change detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InpressChangeDetail {
    /// Journal id.
    pub journal_id: i64,
    /// Before article count.
    pub before_count: usize,
    /// After article count.
    pub after_count: usize,
    /// Added article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub added_article_ids: Vec<i64>,
    /// Removed article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub removed_article_ids: Vec<i64>,
    /// Notifiable article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub notifiable_added_article_ids: Vec<i64>,
    /// Backfill article ids retained by in-memory compatibility callers.
    #[serde(skip_serializing)]
    pub backfill_added_article_ids: Vec<i64>,
}
