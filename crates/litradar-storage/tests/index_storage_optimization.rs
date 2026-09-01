//! Storage and schema measurements used by index optimization regression tests.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_storage::{
    migrate_auth_database, migrate_index_database, open_sqlite_connection, optimize_index_storage,
    preflight_index_database, preflight_storage, record_service_heartbeat,
    IndexStorageOptimizationOptions, IndexStorageOptimizationOutcome, ServiceKind, StorageConfig,
    INDEX_SCHEMA_VERSION,
};
use rusqlite::Connection;
use tempfile::tempdir;

#[derive(Debug, PartialEq, Eq)]
struct StorageMeasurement {
    file_bytes: u64,
    page_bytes: u64,
    freelist_bytes: u64,
    fts_bytes: u64,
    objects: BTreeSet<String>,
}

fn measure_database(path: &Path) -> StorageMeasurement {
    let connection = Connection::open(path).expect("measurement database should open");
    let page_size = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
        .expect("page size should read");
    let page_count = connection
        .query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))
        .expect("page count should read");
    let freelist_count = connection
        .query_row("PRAGMA freelist_count", [], |row| row.get::<_, u64>(0))
        .expect("freelist count should read");
    let fts_bytes = connection
        .query_row(
            "SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name GLOB 'article_search*'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .expect("FTS allocation should read");
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .expect("schema object query should prepare");
    let objects = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("schema objects should query")
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .expect("schema objects should collect");
    drop(statement);
    drop(connection);

    StorageMeasurement {
        file_bytes: fs::metadata(path)
            .expect("database metadata should read")
            .len(),
        page_bytes: page_size * page_count,
        freelist_bytes: page_size * freelist_count,
        fts_bytes,
        objects,
    }
}

#[test]
fn storage_measurement_reports_bytes_and_schema_objects_without_row_contents() {
    let directory = tempdir().expect("temporary directory should create");
    let path = directory.path().join("catalog.sqlite");
    migrate_index_database(&path, None).expect("index database should initialize");
    let connection = open_sqlite_connection(&path).expect("index database should open");
    connection
        .execute_batch(
            r#"
            INSERT INTO journals (
                journal_id, catalog_id, title, title_aliases_json, issns_json, area
            ) VALUES (1, 'fixture', 'Fixture Journal', '[]', '[]', 'Testing');

            INSERT INTO articles (
                article_id, journal_id, title, publication_year, date, authors_json,
                abstract_text, doi, pmid, open_access, in_press
            ) VALUES (
                10, 1, 'Measured Search Row', 2026, '2026-09-02', '["Author"]',
                'storage measurement token', '10.1000/measure', '10', 1, 0
            );

            INSERT INTO article_listing (
                article_id, journal_id, publication_year, date, open_access, in_press,
                doi, pmid, area
            ) VALUES (
                10, 1, 2026, '2026-09-02', 1, 0, '10.1000/measure', '10', 'Testing'
            );

            INSERT INTO article_search (
                rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
            ) VALUES (
                10, 10, 'Measured Search Row', 'storage measurement token',
                '10.1000/measure', '10', 'Author', 'Fixture Journal'
            );
            "#,
        )
        .expect("measurement fixture should insert");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("measurement database should checkpoint");
    drop(connection);

    let first = measure_database(&path);
    let second = measure_database(&path);

    assert_eq!(first, second);
    assert_eq!(first.file_bytes, first.page_bytes);
    assert_eq!(first.freelist_bytes, 0);
    assert!(first.fts_bytes > 0);
    assert!(first.objects.contains("articles"));
    assert!(first.objects.contains("article_listing"));
    assert!(first.objects.contains("article_search"));
}

#[test]
fn confirmed_optimizer_rebuilds_v6_from_canonical_rows_and_is_repeatable_on_v7() {
    let root = tempdir().expect("temporary root should create");
    let config = StorageConfig::from_project_root(root.path());
    fs::create_dir_all(config.index_dir()).expect("index directory should create");
    let path = config.index_dir().join("fixture.sqlite");
    create_version_six_fixture(&path);
    let canonical_before = canonical_snapshot(&path);
    let search_before = search_snapshot(&path);

    let report = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: config.clone(),
        confirmed: true,
    })
    .expect("confirmed optimization should succeed");

    assert_eq!(report.outcome, IndexStorageOptimizationOutcome::Optimized);
    assert_eq!(report.database_count, 1);
    assert!(report.temporary_bytes_required > report.source_bytes);
    assert_eq!(report.databases[0].source_schema_version, 6);
    assert_eq!(
        report.databases[0].target_schema_version,
        INDEX_SCHEMA_VERSION
    );
    assert!(!report.databases[0].after.has_content_shadow);
    assert!(
        report.databases[0].after.freelist_count.saturating_mul(100)
            <= report.databases[0].after.page_count
    );
    assert_eq!(canonical_snapshot(&path), canonical_before);
    assert_eq!(search_snapshot(&path), search_before);
    preflight_index_database(&path, None).expect("optimized database should preflight");
    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
    assert_no_maintenance_artifacts(root.path());

    let repeated = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: config,
        confirmed: true,
    })
    .expect("current v7 optimization should be repeatable");

    assert_eq!(repeated.outcome, IndexStorageOptimizationOutcome::Optimized);
    assert_eq!(repeated.databases[0].source_schema_version, 7);
    assert_eq!(canonical_snapshot(&path), canonical_before);
    assert_eq!(search_snapshot(&path), search_before);
    assert_no_maintenance_artifacts(root.path());
}

#[test]
fn optimizer_confirmation_noop_and_unsupported_schema_gates_precede_source_mutation() {
    let root = tempdir().expect("temporary root should create");
    let config = StorageConfig::from_project_root(root.path());
    let confirmation_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: config.clone(),
        confirmed: false,
    })
    .expect_err("missing confirmation should fail");
    assert_eq!(confirmation_error.code(), "confirmation_required");
    assert!(!root.path().join("data").exists());

    let noop = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: config.clone(),
        confirmed: true,
    })
    .expect("missing index directory should be a no-op");
    assert_eq!(noop.outcome, IndexStorageOptimizationOutcome::Noop);
    assert!(!root.path().join("data").exists());

    fs::create_dir_all(config.index_dir()).expect("index directory should create");
    let unsupported_path = config.index_dir().join("unsupported.sqlite");
    let connection = Connection::open(&unsupported_path).expect("fixture should open");
    connection
        .pragma_update(None, "user_version", INDEX_SCHEMA_VERSION + 1)
        .expect("fixture version should write");
    drop(connection);
    let bytes_before = fs::read(&unsupported_path).expect("fixture bytes should read");

    let unsupported_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: config,
        confirmed: true,
    })
    .expect_err("newer schema should fail closed");

    assert_eq!(unsupported_error.code(), "unsupported_schema");
    assert_eq!(
        fs::read(&unsupported_path).expect("fixture bytes should remain"),
        bytes_before
    );
    assert_no_maintenance_artifacts(root.path());
}

#[test]
fn optimizer_refuses_recent_heartbeats_and_unexpired_index_leases() {
    let heartbeat_root = tempdir().expect("heartbeat root should create");
    let heartbeat_config = StorageConfig::from_project_root(heartbeat_root.path());
    fs::create_dir_all(heartbeat_config.index_dir()).expect("index directory should create");
    let heartbeat_index = heartbeat_config.index_dir().join("fixture.sqlite");
    create_version_six_fixture(&heartbeat_index);
    migrate_auth_database(heartbeat_config.auth_db_path())
        .expect("auth database should initialize");
    let now = current_epoch_seconds();
    record_service_heartbeat(
        heartbeat_config.auth_db_path(),
        ServiceKind::Api,
        "active-api",
        now as f64,
    )
    .expect("heartbeat should persist");
    let heartbeat_bytes = fs::read(&heartbeat_index).expect("source bytes should read");

    let heartbeat_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: heartbeat_config,
        confirmed: true,
    })
    .expect_err("recent heartbeat should block maintenance");

    assert_eq!(heartbeat_error.code(), "active_target");
    assert_eq!(
        fs::read(&heartbeat_index).expect("source bytes should remain"),
        heartbeat_bytes
    );
    assert_no_maintenance_artifacts(heartbeat_root.path());

    let lease_root = tempdir().expect("lease root should create");
    let lease_config = StorageConfig::from_project_root(lease_root.path());
    fs::create_dir_all(lease_config.index_dir()).expect("index directory should create");
    let lease_index = lease_config.index_dir().join("fixture.sqlite");
    create_version_six_fixture(&lease_index);
    fs::create_dir_all(lease_config.index_control_dir()).expect("control directory should create");
    let control = Connection::open(
        lease_config
            .index_control_dir()
            .join("index-batches.sqlite"),
    )
    .expect("control database should open");
    control
        .execute_batch(
            "CREATE TABLE index_batch_lease (
                 lease_key INTEGER PRIMARY KEY,
                 expires_at INTEGER NOT NULL
             );",
        )
        .expect("lease table should create");
    control
        .execute(
            "INSERT INTO index_batch_lease (lease_key, expires_at) VALUES (1, ?1)",
            [now + 600],
        )
        .expect("active lease should insert");
    drop(control);
    let lease_bytes = fs::read(&lease_index).expect("source bytes should read");

    let lease_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: lease_config.clone(),
        confirmed: true,
    })
    .expect_err("unexpired lease should block maintenance");

    assert_eq!(lease_error.code(), "active_lease");
    assert_eq!(
        fs::read(&lease_index).expect("source bytes should remain"),
        lease_bytes
    );
    assert_no_maintenance_artifacts(lease_root.path());

    let control = Connection::open(
        lease_config
            .index_control_dir()
            .join("index-batches.sqlite"),
    )
    .expect("control database should reopen");
    control
        .execute_batch(
            "DELETE FROM index_batch_lease;
             CREATE TABLE provider_leases (
                 catalog_name TEXT NOT NULL,
                 provider_name TEXT NOT NULL,
                 expires_at INTEGER NOT NULL
             );",
        )
        .expect("Provider lease table should create");
    control
        .execute(
            "INSERT INTO provider_leases (
                 catalog_name, provider_name, expires_at
             ) VALUES ('fixture', 'provider', ?1)",
            [now + 600],
        )
        .expect("active Provider lease should insert");
    drop(control);

    let provider_lease_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: lease_config,
        confirmed: true,
    })
    .expect_err("unexpired Provider lease should block maintenance");

    assert_eq!(provider_lease_error.code(), "active_lease");
    assert_eq!(
        fs::read(&lease_index).expect("source bytes should remain"),
        lease_bytes
    );
    assert_no_maintenance_artifacts(lease_root.path());
}

#[test]
fn optimizer_rejects_unexpected_files_and_nonempty_transaction_sidecars() {
    let unexpected_root = tempdir().expect("unexpected root should create");
    let unexpected_config = StorageConfig::from_project_root(unexpected_root.path());
    fs::create_dir_all(unexpected_config.index_dir()).expect("index directory should create");
    create_version_six_fixture(&unexpected_config.index_dir().join("fixture.sqlite"));
    fs::write(
        unexpected_config.index_dir().join("operator-note.txt"),
        b"unexpected",
    )
    .expect("unexpected file should write");

    let unexpected_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: unexpected_config,
        confirmed: true,
    })
    .expect_err("unexpected files should fail closed");
    assert_eq!(unexpected_error.code(), "invalid_layout");
    assert_no_maintenance_artifacts(unexpected_root.path());

    let wal_root = tempdir().expect("WAL root should create");
    let wal_config = StorageConfig::from_project_root(wal_root.path());
    fs::create_dir_all(wal_config.index_dir()).expect("index directory should create");
    let database_path = wal_config.index_dir().join("fixture.sqlite");
    create_version_six_fixture(&database_path);
    fs::write(
        wal_config.index_dir().join("fixture.sqlite-wal"),
        b"uncheckpointed",
    )
    .expect("non-empty WAL should write");
    let bytes_before = fs::read(&database_path).expect("source bytes should read");

    let wal_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: wal_config,
        confirmed: true,
    })
    .expect_err("non-empty WAL should fail closed");

    assert_eq!(wal_error.code(), "invalid_layout");
    assert_eq!(
        fs::read(&database_path).expect("source bytes should remain"),
        bytes_before
    );
    assert_no_maintenance_artifacts(wal_root.path());
}

#[test]
fn stale_maintenance_state_blocks_optimizer_and_normal_startup_before_auth_creation() {
    let root = tempdir().expect("temporary root should create");
    let config = StorageConfig::from_project_root(root.path());
    fs::create_dir_all(root.path().join("data")).expect("data directory should create");
    let marker = root
        .path()
        .join("data")
        .join(".litradar-index-maintenance.json");
    fs::write(&marker, b"retained maintenance evidence\n").expect("marker should write");

    let optimizer_error = optimize_index_storage(&IndexStorageOptimizationOptions {
        storage_config: config.clone(),
        confirmed: true,
    })
    .expect_err("stale marker should block optimizer");
    let startup_error = preflight_storage(&config).expect_err("stale marker should block startup");

    assert_eq!(optimizer_error.code(), "interrupted_state");
    assert!(optimizer_error.recovery_paths().is_some());
    assert!(startup_error
        .to_string()
        .contains("interrupted index maintenance requires recovery"));
    assert!(!config.auth_db_path().exists());
    assert!(marker.is_file());
}

fn create_version_six_fixture(path: &Path) {
    migrate_index_database(path, None).expect("current index should initialize");
    let connection = open_sqlite_connection(path).expect("fixture database should open");
    connection
        .execute_batch(
            r#"
            INSERT INTO journals (
                journal_id, catalog_id, title, title_aliases_json, issns_json,
                issn, eissn, area, abs_rank, abs_rating
            ) VALUES (
                1, 'fixture', 'Alpha Journal', '["A Journal"]', '["0001-0001"]',
                '0001-0001', '0001-0002', 'Medicine', '7', '3'
            );
            INSERT INTO journal_identity_keys (
                identity_kind, identity_value, canonical_catalog_id
            ) VALUES
                ('catalog_id', 'fixture', 'fixture'),
                ('issn', '0001-0001', 'fixture');
            INSERT INTO issues (
                issue_id, journal_id, publication_year, title, volume, number, date
            ) VALUES (11, 1, 2026, 'Issue One', '1', '1', '2026-09-02');
            INSERT INTO articles (
                article_id, journal_id, issue_id, title, publication_year, date,
                authors_json, start_page, end_page, abstract_text, doi, pmid,
                open_access, in_press
            ) VALUES
                (
                    101, 1, 11, 'Genome sequencing methods', 2026, '2026-09-02',
                    '[{"display_name":"Alice"},{"display_name":"Bob"}]',
                    '1', '10', 'Clinical precision study', '10.1000/genome', '1001', 1, 0
                ),
                (
                    102, 1, NULL, 'Preview article', 2026, NULL,
                    '["Chloé"]', NULL, NULL, 'Résumé genome preview', NULL, NULL, 0, 1
                );
            INSERT INTO article_retraction_dois (article_id, retraction_doi)
            VALUES (101, '10.1000/retraction');
            INSERT INTO article_identity_keys (identity_kind, identity_value, article_id)
            VALUES
                ('doi', '10.1000/genome', 101),
                ('pmid', '1001', 101),
                ('bibliographic', 'fixture-preview', 102);
            INSERT INTO article_listing (
                article_id, journal_id, issue_id, publication_year, date,
                open_access, in_press, doi, pmid, area
            ) VALUES
                (101, 1, 11, 2026, '2026-09-02', 1, 0, '10.1000/genome', '1001', 'Medicine'),
                (102, 1, NULL, 2026, NULL, 0, 1, NULL, NULL, 'Medicine');
            INSERT INTO article_change_events (
                event_id, content_revision, article_id, change_kind,
                journal_id, issue_id, in_press, created_at
            ) VALUES (501, 'revision-1', 101, 'upsert', 1, 11, 0, '2026-09-02T00:00:00Z');

            DROP TABLE article_search;
            CREATE VIRTUAL TABLE article_search
            USING fts5(
                article_id UNINDEXED,
                title,
                abstract_text,
                doi,
                pmid,
                authors,
                journal_title,
                tokenize = 'unicode61 remove_diacritics 2'
            );
            INSERT INTO article_search (
                rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
            ) VALUES
                (
                    101, 101, 'Genome sequencing methods', 'Clinical precision study',
                    '10.1000/genome', '1001', 'Alice; Bob', 'Alpha Journal'
                ),
                (
                    102, 102, 'Preview article', 'Résumé genome preview',
                    NULL, NULL, 'Chloé', 'Alpha Journal'
                );
            PRAGMA user_version = 6;
            PRAGMA wal_checkpoint(TRUNCATE);
            PRAGMA journal_mode = DELETE;
            "#,
        )
        .expect("version six fixture should write");
    drop(connection);
}

fn canonical_snapshot(path: &Path) -> Vec<Vec<String>> {
    [
        "SELECT json_array(journal_id, catalog_id, title, title_aliases_json, issns_json, issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating, fms_rank, fms_rating, fmscn_rank, fmscn_rating) FROM journals ORDER BY journal_id",
        "SELECT json_array(identity_kind, identity_value, canonical_catalog_id) FROM journal_identity_keys ORDER BY identity_kind, identity_value",
        "SELECT json_array(issue_id, journal_id, publication_year, title, volume, number, date) FROM issues ORDER BY issue_id",
        "SELECT json_array(article_id, journal_id, issue_id, title, publication_year, date, authors_json, start_page, end_page, abstract_text, doi, pmid, open_access, in_press) FROM articles ORDER BY article_id",
        "SELECT json_array(article_id, retraction_doi) FROM article_retraction_dois ORDER BY article_id, retraction_doi",
        "SELECT json_array(identity_kind, identity_value, article_id) FROM article_identity_keys ORDER BY identity_kind, identity_value",
        "SELECT json_array(article_id, journal_id, issue_id, publication_year, date, open_access, in_press, doi, pmid, area) FROM article_listing ORDER BY article_id",
        "SELECT json_array(event_id, content_revision, article_id, change_kind, journal_id, issue_id, in_press, created_at) FROM article_change_events ORDER BY event_id",
    ]
    .into_iter()
    .map(|query| query_text_rows(path, query))
    .collect()
}

fn search_snapshot(path: &Path) -> Vec<Vec<i64>> {
    [
        "\"Genome sequencing\"",
        "genom*",
        "genome NOT preview",
        "genome OR clinical",
        "title:Clinical",
        "journal_title:Alpha",
        "authors:Alice",
        "doi:\"10.1000/genome\"",
        "pmid:1001",
        "\"resume\"",
    ]
    .into_iter()
    .map(|query| {
        let connection = Connection::open(path).expect("database should open for search");
        let mut statement = connection
            .prepare(
                "SELECT rowid FROM article_search
                 WHERE article_search MATCH ?1
                 ORDER BY rowid",
            )
            .expect("search query should prepare");
        statement
            .query_map([query], |row| row.get::<_, i64>(0))
            .expect("search rows should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("search rows should collect")
    })
    .collect()
}

fn query_text_rows(path: &Path, query: &str) -> Vec<String> {
    let connection = Connection::open(path).expect("database should open for snapshot");
    let mut statement = connection
        .prepare(query)
        .expect("snapshot query should prepare");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("snapshot rows should query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("snapshot rows should collect")
}

fn user_version(path: &Path) -> i64 {
    Connection::open(path)
        .expect("database should open for version")
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("schema version should read")
}

fn current_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn assert_no_maintenance_artifacts(project_root: &Path) {
    for name in [
        ".litradar-index-maintenance.json",
        ".litradar-index-staging",
        ".litradar-index-rollback",
    ] {
        assert!(!project_root.join("data").join(name).exists(), "{name}");
    }
}
