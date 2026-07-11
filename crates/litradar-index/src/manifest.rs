//! Change manifest generation for indexed scholarly articles.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use rusqlite::Connection;
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
    /// Added article ids retained in memory.
    #[serde(skip_serializing)]
    pub added_article_ids: Vec<i64>,
    /// Removed article ids retained in memory.
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
    /// Backfill article ids retained in memory.
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
    /// Added article ids retained in memory.
    #[serde(skip_serializing)]
    pub added_article_ids: Vec<i64>,
    /// Removed article ids retained in memory.
    #[serde(skip_serializing)]
    pub removed_article_ids: Vec<i64>,
    /// Notifiable added article ids retained in memory.
    #[serde(skip_serializing)]
    pub notifiable_added_article_ids: Vec<i64>,
    /// Backfill added article ids retained in memory.
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
    /// Added article ids retained in memory.
    #[serde(skip_serializing)]
    pub added_article_ids: Vec<i64>,
    /// Removed article ids retained in memory.
    #[serde(skip_serializing)]
    pub removed_article_ids: Vec<i64>,
    /// Notifiable added article ids retained in memory.
    #[serde(skip_serializing)]
    pub notifiable_added_article_ids: Vec<i64>,
    /// Backfill added article ids retained in memory.
    #[serde(skip_serializing)]
    pub backfill_added_article_ids: Vec<i64>,
}

/// Article ids grouped by issue and in-press journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArticleSnapshot {
    /// Article ids keyed by `journal_id:issue_id`.
    pub issue_articles: BTreeMap<String, BTreeSet<i64>>,
    /// In-press article ids keyed by journal id.
    pub inpress_articles: BTreeMap<i64, BTreeSet<i64>>,
}

/// Collect a database article snapshot for change detection.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
///
/// # Returns
///
/// Article snapshot grouped for change manifests.
pub fn collect_article_snapshot(connection: &Connection) -> rusqlite::Result<ArticleSnapshot> {
    let mut snapshot = ArticleSnapshot::default();
    let mut issue_statement = connection.prepare(
        "
        SELECT journal_id, issue_id, article_id
        FROM articles
        WHERE issue_id IS NOT NULL
        ",
    )?;
    let issue_rows = issue_statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in issue_rows {
        let (journal_id, issue_id, article_id) = row?;
        snapshot
            .issue_articles
            .entry(format!("{journal_id}:{issue_id}"))
            .or_default()
            .insert(article_id);
    }

    let mut inpress_statement = connection.prepare(
        "
        SELECT journal_id, article_id
        FROM articles
        WHERE issue_id IS NULL AND COALESCE(in_press, 0) = 1
        ",
    )?;
    let inpress_rows = inpress_statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in inpress_rows {
        let (journal_id, article_id) = row?;
        snapshot
            .inpress_articles
            .entry(journal_id)
            .or_default()
            .insert(article_id);
    }
    Ok(snapshot)
}

/// Build a change manifest from before/after article snapshots.
///
/// # Arguments
///
/// * `db_name` - Index database filename.
/// * `run_id` - Run identifier.
/// * `generated_at` - Generation timestamp.
/// * `before` - Snapshot before indexing.
/// * `after` - Snapshot after indexing.
///
/// # Returns
///
/// Change manifest payload.
pub fn build_change_manifest_from_snapshots(
    db_name: &str,
    run_id: &str,
    generated_at: &str,
    before: &ArticleSnapshot,
    after: &ArticleSnapshot,
) -> ChangeManifest {
    let mut issue_keys = before
        .issue_articles
        .keys()
        .chain(after.issue_articles.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut issue_details = Vec::new();
    let mut changed_issue_keys = Vec::new();
    let mut added_article_ids = BTreeSet::new();
    let mut removed_article_ids = BTreeSet::new();
    for issue_key in std::mem::take(&mut issue_keys) {
        let before_ids = before
            .issue_articles
            .get(&issue_key)
            .cloned()
            .unwrap_or_default();
        let after_ids = after
            .issue_articles
            .get(&issue_key)
            .cloned()
            .unwrap_or_default();
        if before_ids == after_ids {
            continue;
        }
        let added = after_ids
            .difference(&before_ids)
            .copied()
            .collect::<Vec<_>>();
        let removed = before_ids
            .difference(&after_ids)
            .copied()
            .collect::<Vec<_>>();
        added_article_ids.extend(added.iter().copied());
        removed_article_ids.extend(removed.iter().copied());
        changed_issue_keys.push(issue_key.clone());
        issue_details.push(IssueChangeDetail {
            issue_key,
            before_count: before_ids.len(),
            after_count: after_ids.len(),
            added_article_ids: added.clone(),
            removed_article_ids: removed,
            notifiable_added_article_ids: added,
            backfill_added_article_ids: Vec::new(),
        });
    }

    let mut inpress_keys = before
        .inpress_articles
        .keys()
        .chain(after.inpress_articles.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut inpress_details = Vec::new();
    let mut changed_inpress_journal_ids = Vec::new();
    for journal_id in std::mem::take(&mut inpress_keys) {
        let before_ids = before
            .inpress_articles
            .get(&journal_id)
            .cloned()
            .unwrap_or_default();
        let after_ids = after
            .inpress_articles
            .get(&journal_id)
            .cloned()
            .unwrap_or_default();
        if before_ids == after_ids {
            continue;
        }
        let added = after_ids
            .difference(&before_ids)
            .copied()
            .collect::<Vec<_>>();
        let removed = before_ids
            .difference(&after_ids)
            .copied()
            .collect::<Vec<_>>();
        added_article_ids.extend(added.iter().copied());
        removed_article_ids.extend(removed.iter().copied());
        changed_inpress_journal_ids.push(journal_id);
        inpress_details.push(InpressChangeDetail {
            journal_id,
            before_count: before_ids.len(),
            after_count: after_ids.len(),
            added_article_ids: added.clone(),
            removed_article_ids: removed,
            notifiable_added_article_ids: added,
            backfill_added_article_ids: Vec::new(),
        });
    }

    let added_article_ids = added_article_ids.into_iter().collect::<Vec<_>>();
    let removed_article_ids = removed_article_ids.into_iter().collect::<Vec<_>>();
    let summary = ChangeSummary {
        changed_issue_count: changed_issue_keys.len(),
        changed_inpress_count: changed_inpress_journal_ids.len(),
        added_article_count: added_article_ids.len(),
        removed_article_count: removed_article_ids.len(),
        added_article_ids: added_article_ids.clone(),
        removed_article_ids: removed_article_ids.clone(),
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
        changed_issue_keys,
        changed_inpress_journal_ids,
        notifiable_article_ids: added_article_ids,
        backfill_issue_keys: Vec::new(),
        backfill_inpress_journal_ids: Vec::new(),
        backfill_article_ids: Vec::new(),
        summary,
    }
}

/// Build a change manifest for a fresh fixture index run.
///
/// # Arguments
///
/// * `db_name` - Index database filename.
/// * `run_id` - Run identifier.
/// * `generated_at` - Generation timestamp.
/// * `articles` - Written article records.
///
/// # Returns
///
/// Change manifest payload.
pub fn build_change_manifest(
    db_name: &str,
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
        serde_json::to_string(manifest).expect("change manifest payload should serialize");
    fs::write(path, format!("{payload}\n"))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;

    use rusqlite::Connection;
    use serde_json::json;

    use crate::schema::init_index_db;
    use crate::transforms::ArticleRecord;

    use super::{
        build_change_manifest, build_change_manifest_from_snapshots, collect_article_snapshot,
        write_change_manifest, ArticleSnapshot,
    };

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
            "run-1",
            "2026-07-03T00:00:00Z",
            &[article],
        );

        assert_eq!(manifest.changed_issue_keys, vec!["1:2"]);
        assert_eq!(manifest.notifiable_article_ids, vec![10]);
        assert_eq!(manifest.summary.added_article_count, 1);
    }

    #[test]
    fn collect_article_snapshot_groups_issue_and_inpress_rows() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        connection
            .execute_batch(
                "
                INSERT INTO journals (journal_id, library_id, title)
                VALUES (1, 'scholarly', 'Alpha'), (2, 'cnki', 'Beta');

                INSERT INTO issues (issue_id, journal_id, publication_year)
                VALUES (10, 1, 2026), (20, 2, 2026);

                INSERT INTO articles
                    (article_id, journal_id, issue_id, title, in_press)
                VALUES
                    (1001, 1, 10, 'A', 0),
                    (1002, 1, 10, 'B', 0),
                    (2001, 2, 20, 'C', 0),
                    (3001, 1, NULL, 'In Press', 1),
                    (3002, 2, NULL, 'Not In Press', 0);
                ",
            )
            .expect("fixture rows should insert");

        let snapshot = collect_article_snapshot(&connection).expect("snapshot should collect");

        assert_eq!(
            snapshot.issue_articles,
            BTreeMap::from([
                ("1:10".to_string(), BTreeSet::from([1001, 1002])),
                ("2:20".to_string(), BTreeSet::from([2001])),
            ])
        );
        assert_eq!(
            snapshot.inpress_articles,
            BTreeMap::from([(1, BTreeSet::from([3001]))])
        );
    }

    #[test]
    fn snapshot_manifest_tracks_added_and_removed_issue_and_inpress_articles() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let before = ArticleSnapshot {
            issue_articles: BTreeMap::from([("1:2".to_string(), BTreeSet::from([10, 11]))]),
            inpress_articles: BTreeMap::from([(1, BTreeSet::from([20]))]),
        };
        let after = ArticleSnapshot {
            issue_articles: BTreeMap::from([("1:2".to_string(), BTreeSet::from([11, 12]))]),
            inpress_articles: BTreeMap::from([(1, BTreeSet::from([20, 21]))]),
        };

        let manifest = build_change_manifest_from_snapshots(
            "fixture.sqlite",
            "run-1",
            "2026-07-05T00:00:00Z",
            &before,
            &after,
        );
        let manifest_path = temp_dir.path().join("nested").join("changes.json");
        write_change_manifest(&manifest, &manifest_path).expect("manifest should write");
        let payload = fs::read_to_string(&manifest_path).expect("manifest should be readable");

        assert_eq!(manifest.changed_issue_keys, vec!["1:2"]);
        assert_eq!(manifest.changed_inpress_journal_ids, vec![1]);
        assert_eq!(manifest.notifiable_article_ids, vec![12, 21]);
        assert_eq!(manifest.summary.removed_article_ids, vec![10]);
        assert!(payload.ends_with('\n'));
        assert!(!payload.trim_end().contains('\n'));
        let payload_json: serde_json::Value =
            serde_json::from_str(&payload).expect("manifest JSON should parse");
        assert!(payload_json.get("db_path").is_none());
        assert_eq!(payload_json["notifiable_article_ids"], json!([12, 21]));
        assert_eq!(payload_json["backfill_article_ids"], json!([]));
        assert!(payload_json["summary"].get("added_article_ids").is_none());
        assert!(payload_json["summary"].get("removed_article_ids").is_none());
        assert!(payload_json["summary"]
            .get("backfill_article_ids")
            .is_none());
        assert!(payload_json["summary"]["issues"][0]
            .get("added_article_ids")
            .is_none());
        assert!(payload_json["summary"]["issues"][0]
            .get("removed_article_ids")
            .is_none());
        assert!(payload_json["summary"]["issues"][0]
            .get("notifiable_added_article_ids")
            .is_none());
        assert!(payload_json["summary"]["issues"][0]
            .get("backfill_added_article_ids")
            .is_none());
        assert!(payload_json["summary"]["inpress"][0]
            .get("added_article_ids")
            .is_none());
        assert!(payload_json["summary"]["inpress"][0]
            .get("removed_article_ids")
            .is_none());
        assert!(payload_json["summary"]["inpress"][0]
            .get("notifiable_added_article_ids")
            .is_none());
        assert!(payload_json["summary"]["inpress"][0]
            .get("backfill_added_article_ids")
            .is_none());
    }
}
