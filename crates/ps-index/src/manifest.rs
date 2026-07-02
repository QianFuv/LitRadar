//! Change manifest generation for indexed scholarly articles.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::transforms::ArticleRecord;

/// Python-compatible change manifest payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeManifest {
    /// Run identifier.
    pub run_id: String,
    /// Manifest generation timestamp.
    pub generated_at: String,
    /// Index database filename.
    pub db_name: String,
    /// Index database path.
    pub db_path: String,
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
    /// Added article ids.
    pub added_article_ids: Vec<i64>,
    /// Removed article ids.
    pub removed_article_ids: Vec<i64>,
    /// Changed issue details.
    pub issues: Vec<IssueChangeDetail>,
    /// Changed in-press details.
    pub inpress: Vec<InpressChangeDetail>,
    /// Raw changed issue count.
    pub raw_changed_issue_count: usize,
    /// Raw changed in-press count.
    pub raw_changed_inpress_count: usize,
    /// Backfill article ids.
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
    /// Added article ids.
    pub added_article_ids: Vec<i64>,
    /// Removed article ids.
    pub removed_article_ids: Vec<i64>,
    /// Notifiable added article ids.
    pub notifiable_added_article_ids: Vec<i64>,
    /// Backfill added article ids.
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
    /// Added article ids.
    pub added_article_ids: Vec<i64>,
    /// Removed article ids.
    pub removed_article_ids: Vec<i64>,
    /// Notifiable added article ids.
    pub notifiable_added_article_ids: Vec<i64>,
    /// Backfill added article ids.
    pub backfill_added_article_ids: Vec<i64>,
}

/// Build a change manifest for a fresh fixture index run.
///
/// # Arguments
///
/// * `db_name` - Index database filename.
/// * `db_path` - Index database path.
/// * `run_id` - Run identifier.
/// * `generated_at` - Generation timestamp.
/// * `articles` - Written article records.
///
/// # Returns
///
/// Change manifest payload.
pub fn build_change_manifest(
    db_name: &str,
    db_path: &Path,
    run_id: &str,
    generated_at: &str,
    articles: &[ArticleRecord],
) -> ChangeManifest {
    let mut issues: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut inpress: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut article_ids = Vec::new();
    for article in articles {
        article_ids.push(article.article_id);
        if let Some(issue_id) = article.issue_id {
            issues
                .entry(format!("{}:{issue_id}", article.journal_id))
                .or_default()
                .push(article.article_id);
        } else if article.in_press.unwrap_or_default() == 1 {
            inpress
                .entry(article.journal_id)
                .or_default()
                .push(article.article_id);
        }
    }
    article_ids.sort_unstable();
    article_ids.dedup();

    let issue_details = issues
        .iter_mut()
        .map(|(issue_key, ids)| {
            ids.sort_unstable();
            ids.dedup();
            IssueChangeDetail {
                issue_key: issue_key.clone(),
                before_count: 0,
                after_count: ids.len(),
                added_article_ids: ids.clone(),
                removed_article_ids: Vec::new(),
                notifiable_added_article_ids: ids.clone(),
                backfill_added_article_ids: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    let inpress_details = inpress
        .iter_mut()
        .map(|(journal_id, ids)| {
            ids.sort_unstable();
            ids.dedup();
            InpressChangeDetail {
                journal_id: *journal_id,
                before_count: 0,
                after_count: ids.len(),
                added_article_ids: ids.clone(),
                removed_article_ids: Vec::new(),
                notifiable_added_article_ids: ids.clone(),
                backfill_added_article_ids: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    let changed_issue_keys = issues.keys().cloned().collect::<Vec<_>>();
    let changed_inpress_journal_ids = inpress.keys().copied().collect::<Vec<_>>();
    let summary = ChangeSummary {
        changed_issue_count: changed_issue_keys.len(),
        changed_inpress_count: changed_inpress_journal_ids.len(),
        added_article_count: article_ids.len(),
        removed_article_count: 0,
        added_article_ids: article_ids.clone(),
        removed_article_ids: Vec::new(),
        issues: issue_details,
        inpress: inpress_details,
        raw_changed_issue_count: changed_issue_keys.len(),
        raw_changed_inpress_count: changed_inpress_journal_ids.len(),
        backfill_article_ids: Vec::new(),
        backfill_article_count: 0,
        backfill_issue_keys: Vec::new(),
        backfill_inpress_journal_ids: Vec::new(),
    };
    ChangeManifest {
        run_id: run_id.to_string(),
        generated_at: generated_at.to_string(),
        db_name: db_name.to_string(),
        db_path: db_path.display().to_string(),
        changed_issue_keys,
        changed_inpress_journal_ids,
        notifiable_article_ids: article_ids,
        backfill_issue_keys: Vec::new(),
        backfill_inpress_journal_ids: Vec::new(),
        backfill_article_ids: Vec::new(),
        summary,
    }
}

/// Write a change manifest JSON file.
///
/// # Arguments
///
/// * `manifest` - Manifest payload.
/// * `path` - Output path.
///
/// # Returns
///
/// IO result.
pub fn write_change_manifest(manifest: &ChangeManifest, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload =
        serde_json::to_string_pretty(manifest).expect("change manifest payload should serialize");
    fs::write(path, format!("{payload}\n"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::transforms::ArticleRecord;

    use super::build_change_manifest;

    #[test]
    fn manifest_contains_notification_fields() {
        let article = ArticleRecord {
            article_id: 10,
            journal_id: 1,
            issue_id: Some(2),
            title: None,
            date: None,
            authors: Some("Ada Lovelace".into()),
            start_page: None,
            end_page: None,
            abstract_text: None,
            doi: None,
            pmid: None,
            permalink: None,
            suppressed: None,
            in_press: None,
            open_access: None,
            platform_id: None,
            retraction_doi: None,
            within_library_holdings: None,
            content_location: None,
            full_text_file: None,
        };

        let manifest = build_change_manifest(
            "contract.sqlite",
            Path::new("contract.sqlite"),
            "run-1",
            "2026-07-03T00:00:00Z",
            &[article],
        );

        assert_eq!(manifest.changed_issue_keys, vec!["1:2"]);
        assert_eq!(manifest.notifiable_article_ids, vec![10]);
        assert_eq!(manifest.summary.added_article_count, 1);
    }
}
