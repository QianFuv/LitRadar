//! Parsed weekly manifest cache contracts and an opt-in synthetic query benchmark.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use litradar_storage::index::{
    get_weekly_update_articles, get_weekly_update_articles_with_cache, get_weekly_updates_summary,
    get_weekly_updates_summary_with_cache, WeeklyArticlePageParams,
};
use litradar_storage::weekly_manifest::{
    load_weekly_manifests, load_weekly_manifests_with_cache, weekly_window_start,
};
use litradar_storage::{
    migrate_index_database, open_sqlite_connection, StorageConfig, WeeklyManifestCache,
};
use tempfile::tempdir;

fn write_manifest(root: &Path, catalog: &str, count: i64) -> StorageConfig {
    let config = StorageConfig::from_project_root(root);
    let directory = root.join("data/push_state");
    fs::create_dir_all(&directory).expect("manifest directory should exist");
    fs::write(directory.join(format!("{catalog}.changes.json")), serde_json::to_vec(&serde_json::json!({
        "db_name": format!("{catalog}.sqlite"), "run_id": format!("run-{catalog}"),
        "generated_at": "2026-09-04T00:00:00Z", "notifiable_article_ids": (1..=count).collect::<Vec<_>>()
    })).expect("manifest should serialize")).expect("manifest should write");
    config
}

fn window_end() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-05T12:00:00Z")
        .expect("window should parse")
        .with_timezone(&Utc)
}

#[test]
fn cached_manifests_match_cold_reads_and_recover_after_replacement_or_corruption() {
    let root = tempdir().expect("temporary project should exist");
    let config = write_manifest(root.path(), "fixture", 10);
    let cache = WeeklyManifestCache::default();
    let end = window_end();
    let start = weekly_window_start(end);
    let cold = load_weekly_manifests(&config, start, end).expect("cold read should work");
    assert_eq!(
        load_weekly_manifests_with_cache(&config, start, end, &cache).expect("cache should fill"),
        cold
    );
    assert_eq!(
        load_weekly_manifests_with_cache(&config, start, end, &cache)
            .expect("warm read should work"),
        cold
    );
    assert_eq!(cache.stats().parse_attempts, 1);
    let path = root.path().join("data/push_state/fixture.changes.json");
    let source = fs::read_to_string(&path)
        .expect("source should read")
        .replace("run-fixture", "run-updated");
    fs::write(&path, source).expect("same-length source should replace");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("source should open")
        .set_modified(SystemTime::now() + Duration::from_secs(2))
        .expect("fingerprint should change");
    let updated = load_weekly_manifests_with_cache(&config, start, end, &cache)
        .expect("replacement should read");
    assert_eq!(updated[0].run_id.as_deref(), Some("run-updated"));
    fs::write(&path, "{").expect("broken source should write");
    assert!(load_weekly_manifests_with_cache(&config, start, end, &cache).is_err());
    assert_eq!(cache.stats().entries, 0);
    write_manifest(root.path(), "fixture", 11);
    assert_eq!(
        load_weekly_manifests_with_cache(&config, start, end, &cache)
            .expect("repaired source should recover")[0]
            .article_ids
            .len(),
        11
    );
    assert_eq!(cache.stats().parse_attempts, 4);
    fs::remove_file(&path).expect("source should remove");
    assert!(
        load_weekly_manifests_with_cache(&config, start, end, &cache)
            .expect("missing sources should remain empty")
            .is_empty()
    );
}

#[test]
fn manifest_cache_evicts_least_recent_entries_at_the_capacity_bound() {
    let root = tempdir().expect("temporary project should exist");
    let cache = WeeklyManifestCache::default();
    let configs = (0..65)
        .map(|index| write_manifest(&root.path().join(index.to_string()), "fixture", 1))
        .collect::<Vec<_>>();
    let end = window_end();
    for config in &configs[..64] {
        load_weekly_manifests_with_cache(config, weekly_window_start(end), end, &cache)
            .expect("entry should cache");
    }
    load_weekly_manifests_with_cache(&configs[0], weekly_window_start(end), end, &cache)
        .expect("old entry should become recent");
    load_weekly_manifests_with_cache(&configs[64], weekly_window_start(end), end, &cache)
        .expect("new entry should evict one");
    assert_eq!(cache.stats().entries, 64);
    assert_eq!(cache.stats().parse_attempts, 65);
    load_weekly_manifests_with_cache(&configs[0], weekly_window_start(end), end, &cache)
        .expect("recent entry should remain");
    assert_eq!(cache.stats().parse_attempts, 65);
    load_weekly_manifests_with_cache(&configs[1], weekly_window_start(end), end, &cache)
        .expect("evicted entry should reread");
    assert_eq!(cache.stats().parse_attempts, 66);
}

#[test]
fn manifest_cache_bounds_retained_ids_and_bypasses_oversized_publications() {
    let root = tempdir().expect("temporary project should exist");
    let cache = WeeklyManifestCache::default();
    let end = window_end();
    for (name, count) in [
        ("first", 600_000),
        ("second", 500_000),
        ("oversized", 1_000_001),
    ] {
        let config = write_manifest(&root.path().join(name), name, count);
        load_weekly_manifests_with_cache(&config, weekly_window_start(end), end, &cache)
            .expect("bounded read should work");
        assert!(cache.stats().article_ids <= 1_000_000);
    }
    assert_eq!(cache.stats().entries, 1);
    assert_eq!(cache.stats().article_ids, 500_000);
}

#[test]
#[ignore = "synthetic performance comparison; run explicitly in release mode"]
fn benchmark_weekly_manifest_cache() {
    let root = tempdir().expect("temporary project should exist");
    let config = StorageConfig::from_project_root(root.path());
    fs::create_dir_all(config.index_dir()).expect("index directory should exist");
    for index in 0..8 {
        let catalog = format!("catalog-{index}");
        write_manifest(root.path(), &catalog, 10_000);
        let manifest_path = root
            .path()
            .join("data/push_state")
            .join(format!("{catalog}.changes.json"));
        let mut payload: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest should read"))
                .expect("manifest should parse");
        payload["generated_at"] = serde_json::json!((DateTime::<Utc>::from(SystemTime::now())
            - chrono::TimeDelta::days(1))
        .to_rfc3339());
        fs::write(
            &manifest_path,
            serde_json::to_vec(&payload).expect("manifest should encode"),
        )
        .expect("benchmark window should update");
        let path = config.index_dir().join(format!("{catalog}.sqlite"));
        migrate_index_database(&path, None).expect("index schema should migrate");
        let connection = open_sqlite_connection(path).expect("index should open");
        connection.execute_batch(
            "INSERT INTO journals (journal_id, catalog_id, title, title_aliases_json, issns_json) VALUES (1, 'fixture', 'Fixture journal', '[]', '[]'); \
             WITH RECURSIVE sequence(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 10000) \
             INSERT INTO articles (article_id, journal_id, title, publication_year, date, authors_json) SELECT value, 1, 'Fixture article', 2026, '2026-09-04', '[]' FROM sequence; \
             INSERT INTO article_listing (article_id, journal_id, publication_year, date) SELECT article_id, journal_id, publication_year, date FROM articles;"
        ).expect("query fixture should populate");
    }
    let cache = WeeklyManifestCache::default();
    let started = Instant::now();
    let cold_summary = get_weekly_updates_summary(&config).expect("cold summary should work");
    let cold_summary_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let cached_summary =
        get_weekly_updates_summary_with_cache(&config, &cache).expect("cached summary should work");
    let fill_summary_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cold_summary.databases, cached_summary.databases);
    let params = WeeklyArticlePageParams {
        db_name: "catalog-0.sqlite".into(),
        journal_id: 1,
        window_end: cached_summary.window_end.clone(),
        q: None,
        limit: 50,
        cursor: None,
    };
    let first = get_weekly_update_articles_with_cache(&config, &params, &cache)
        .expect("first page should work");
    assert_eq!(first.items.len(), 50);
    let mut params = params;
    params.cursor = first.page.next_cursor;
    let expected =
        get_weekly_update_articles(&config, &params).expect("uncached continuation should work");
    let started = Instant::now();
    for _ in 0..10 {
        assert_eq!(
            get_weekly_update_articles(&config, &params).expect("cold continuation should work"),
            expected
        );
    }
    let cold_pages_ms = started.elapsed().as_secs_f64() * 1000.0;
    let parse_count = cache.stats().parse_attempts;
    let started = Instant::now();
    for _ in 0..10 {
        assert_eq!(
            get_weekly_update_articles_with_cache(&config, &params, &cache)
                .expect("warm continuation should work"),
            expected
        );
    }
    let warm_pages_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cache.stats().parse_attempts, parse_count);
    println!("8 catalogs x 10000 IDs; cold summary={cold_summary_ms:.2} ms; cache fill summary={fill_summary_ms:.2} ms; 10 uncached pages={cold_pages_ms:.2} ms; 10 warm pages={warm_pages_ms:.2} ms; retained={:?}", cache.stats());
}
