//! Manual weekly snapshots shared with weekly queries and counts.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManualWeeklyManifest {
    pub(super) db_name: String,
    pub(super) manifest: litradar_recommend::ChangeManifest,
}

pub(super) fn manual_weekly_manifests(
    storage: &litradar_storage::StorageConfig,
    selected_databases: &[String],
    window_end: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<ManualWeeklyManifest>, DeliveryError> {
    Ok(
        litradar_storage::weekly_manifest::load_available_weekly_manifests(
            storage,
            window_end,
            selected_databases,
        )?
        .into_iter()
        .filter(|manifest| is_database_selected(selected_databases, &manifest.db_name))
        .map(|manifest| ManualWeeklyManifest {
            db_name: manifest.db_name,
            manifest: litradar_recommend::ChangeManifest {
                pending_issue_keys: Vec::new(),
                pending_inpress_keys: Vec::new(),
                pending_article_ids: manifest.article_ids,
                run_id: manifest.run_id,
            },
        })
        .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn manual_weekly_manifests_share_fixed_history_membership_and_identity() {
        let root = tempdir().expect("temporary project should exist");
        let storage = litradar_storage::StorageConfig::from_project_root(root.path());
        let window_end = chrono::DateTime::parse_from_rfc3339("2026-09-05T12:00:00.250Z")
            .expect("window should parse")
            .with_timezone(&chrono::Utc);
        let history = root.path().join("data/push_state/history/fixture");
        fs::create_dir_all(&history).expect("history should exist");
        fs::create_dir_all(storage.index_dir()).expect("index directory should exist");
        let connection =
            litradar_storage::open_sqlite_connection(storage.index_dir().join("fixture.sqlite"))
                .expect("fixture database should open");
        connection.execute_batch(
            "CREATE TABLE journals (journal_id INTEGER PRIMARY KEY); \
             CREATE TABLE article_listing (article_id INTEGER PRIMARY KEY, journal_id INTEGER); \
             INSERT INTO journals VALUES (1); INSERT INTO article_listing VALUES (101, 1), (102, 1);"
        ).expect("article membership should exist");
        let payload = serde_json::json!({
            "db_name": "fixture.sqlite", "run_id": " original-source-run ",
            "generated_at": "2026-09-05T12:00:00.250Z", "notifiable_article_ids": [101, 101, 999],
            "backfill_article_ids": [102]
        });
        let history_path = history.join(format!("{}.changes.json", "a".repeat(64)));
        fs::write(&history_path, payload.to_string()).expect("history should write");
        fs::write(
            root.path().join("data/push_state/fixture.changes.json"),
            payload.to_string(),
        )
        .expect("duplicate current publication should write");
        let manifests =
            manual_weekly_manifests(&storage, &[], window_end).expect("snapshots should load");
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].manifest.pending_article_ids, vec![101]);
        assert_eq!(
            manifests[0].manifest.run_id.as_deref(),
            Some("original-source-run")
        );
        assert!(
            manual_weekly_manifests(&storage, &["other.sqlite".into()], window_end)
                .expect("selection should apply")
                .is_empty()
        );
        assert!(manual_weekly_manifests(
            &storage,
            &[],
            window_end - chrono::TimeDelta::milliseconds(1)
        )
        .expect("future publication should be excluded")
        .is_empty());
        fs::write(root.path().join("data/push_state/fixture.changes.json"),
            r#"{"db_name":"fixture.sqlite","generated_at":"2000-01-01T00:00:00Z","notifiable_article_ids":[102]}"#)
            .expect("stale current publication should write");
        let history_only =
            manual_weekly_manifests(&storage, &[], window_end).expect("history should remain");
        assert_eq!(history_only, manifests);
        fs::write(&history_path, "{").expect("invalid history should write");
        assert!(manual_weekly_manifests(&storage, &[], window_end).is_err());
    }
}
