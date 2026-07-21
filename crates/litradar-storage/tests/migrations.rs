//! Integration tests for versioned auth and index database migrations.

use std::fs;
use std::path::Path;

use litradar_storage::{
    count_users, get_journal, migrate_auth_database, migrate_index_database, migrate_storage,
    MigrationError, StorageConfig, AUTH_SCHEMA_VERSION, INDEX_SCHEMA_VERSION,
};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn empty_auth_database_migration_creates_current_schema() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("data/auth.sqlite");

    migrate_auth_database(&path).expect("empty auth database should migrate");

    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
    assert!(table_exists(&path, "users"));
    assert!(table_exists(&path, "notification_settings"));
    assert!(table_exists(&path, "scheduled_tasks"));
    assert!(table_columns(&path, "users").contains(&"is_admin".to_string()));
    assert!(table_columns(&path, "announcements").contains(&"priority".to_string()));
    assert!(table_columns(&path, "scheduled_tasks").contains(&"job_spec".to_string()));
    assert!(table_columns(&path, "scheduled_tasks").contains(&"timezone".to_string()));
    assert!(table_columns(&path, "scheduled_tasks").contains(&"timeout_seconds".to_string()));
    assert!(!table_columns(&path, "scheduled_tasks").contains(&"command".to_string()));
    assert!(table_exists(&path, "scheduled_task_runs"));
    assert!(table_exists(&path, "scheduler_workers"));
    assert!(table_exists(&path, "service_heartbeats"));
    assert!(table_exists(&path, "managed_meta_catalogs"));
}

#[test]
fn managed_meta_migration_preserves_version_five_rows() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("version-five-auth.sqlite");
    migrate_auth_database(&path).expect("current auth database should migrate");
    let connection = Connection::open(&path).expect("auth database should open");
    connection
        .execute_batch(
            "
            INSERT INTO users (
                id, username, password_hash, salt, is_admin, created_at, updated_at
            ) VALUES (
                71, 'version-five-user', 'preserved-hash', 'preserved-salt', 1,
                10.0, 11.0
            );
            DROP TABLE managed_meta_catalogs;
            PRAGMA user_version = 5;
            ",
        )
        .expect("version five fixture should be created");
    drop(connection);

    migrate_auth_database(&path).expect("version five database should migrate");

    let connection = Connection::open(&path).expect("migrated database should open");
    let username: String = connection
        .query_row("SELECT username FROM users WHERE id = 71", [], |row| {
            row.get(0)
        })
        .expect("existing user should remain");
    assert_eq!(username, "version-five-user");
    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
    assert!(table_exists(&path, "managed_meta_catalogs"));
    assert_eq!(
        table_columns(&path, "managed_meta_catalogs"),
        ["filename", "bundle_version", "applied_sha256"]
    );
}

#[test]
fn service_heartbeat_migration_preserves_version_three_scheduler_rows() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("auth.sqlite");
    migrate_auth_database(&path).expect("current auth database should migrate");
    let connection = Connection::open(&path).expect("auth database should open");
    connection
        .execute_batch(
            "DROP TABLE managed_meta_catalogs;
             DROP TABLE service_heartbeats;
             PRAGMA user_version = 3;
             INSERT INTO scheduler_workers (worker_id, started_at, heartbeat_at)
             VALUES ('worker-v3', 10, 20);",
        )
        .expect("version three fixture should be created");
    drop(connection);

    migrate_auth_database(&path).expect("version three database should migrate");

    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
    assert!(table_exists(&path, "service_heartbeats"));
    let worker_count: i64 = Connection::open(&path)
        .expect("migrated database should open")
        .query_row(
            "SELECT COUNT(*) FROM scheduler_workers WHERE worker_id = 'worker-v3'",
            [],
            |row| row.get(0),
        )
        .expect("worker row should load");
    assert_eq!(worker_count, 1);
}

#[test]
fn cancellation_status_migration_preserves_version_four_runs() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("version-four-auth.sqlite");
    let connection = Connection::open(&path).expect("version four database should open");
    connection
        .execute_batch(
            "
            CREATE TABLE scheduled_task_runs (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id          INTEGER NOT NULL,
                task_name        TEXT    NOT NULL,
                scheduled_for    INTEGER NOT NULL,
                status           TEXT    NOT NULL
                    CHECK (status IN ('pending', 'claimed', 'running', 'success',
                                      'failed', 'timed_out', 'error', 'unknown')),
                worker_id        TEXT,
                claim_expires_at REAL,
                claimed_at       REAL,
                started_at       REAL,
                finished_at      REAL,
                output_summary   TEXT NOT NULL DEFAULT '',
                UNIQUE(task_id, scheduled_for)
            );
            CREATE INDEX idx_scheduled_task_runs_task
                ON scheduled_task_runs(task_id, scheduled_for DESC);
            CREATE INDEX idx_scheduled_task_runs_status
                ON scheduled_task_runs(status, claim_expires_at);
            INSERT INTO scheduled_task_runs
                (id, task_id, task_name, scheduled_for, status, worker_id,
                 claim_expires_at, claimed_at, started_at, finished_at,
                 output_summary)
            VALUES
                (7, 11, 'Existing run', 1800, 'running', 'worker-a',
                 1900.0, 1810.0, 1820.0, NULL, 'partial output'),
                (8, 12, 'Finished run', 1860, 'success', 'worker-b',
                 NULL, 1861.0, 1862.0, 1870.0, 'complete output');
            PRAGMA user_version = 4;
            ",
        )
        .expect("version four fixture should be created");
    drop(connection);

    migrate_auth_database(&path).expect("version four database should migrate");

    let connection = Connection::open(&path).expect("migrated database should open");
    let rows = connection
        .prepare(
            "SELECT id, task_id, task_name, scheduled_for, status, worker_id,
                    claim_expires_at, claimed_at, started_at, finished_at,
                    output_summary
             FROM scheduled_task_runs ORDER BY id",
        )
        .expect("scheduled runs query should prepare")
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<f64>>(6)?,
                row.get::<_, Option<f64>>(7)?,
                row.get::<_, Option<f64>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, String>(10)?,
            ))
        })
        .expect("scheduled runs should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("scheduled runs should collect");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 7);
    assert_eq!(rows[0].1, 11);
    assert_eq!(rows[0].2, "Existing run");
    assert_eq!(rows[0].3, 1800);
    assert_eq!(rows[0].4, "running");
    assert_eq!(rows[0].5.as_deref(), Some("worker-a"));
    assert_eq!(rows[0].6, Some(1900.0));
    assert_eq!(rows[0].7, Some(1810.0));
    assert_eq!(rows[0].8, Some(1820.0));
    assert_eq!(rows[0].9, None);
    assert_eq!(rows[0].10, "partial output");
    assert_eq!(rows[1].0, 8);
    assert_eq!(rows[1].4, "success");
    assert_eq!(rows[1].9, Some(1870.0));
    assert_eq!(rows[1].10, "complete output");

    for (offset, status) in [
        "pending",
        "claimed",
        "running",
        "success",
        "failed",
        "timed_out",
        "error",
        "unknown",
        "cancelled",
    ]
    .into_iter()
    .enumerate()
    {
        connection
            .execute(
                "INSERT INTO scheduled_task_runs
                    (task_id, task_name, scheduled_for, status)
                 VALUES (?1, 'Status fixture', ?2, ?3)",
                rusqlite::params![100 + offset as i64, 3000 + offset as i64, status],
            )
            .expect("supported scheduled run status should insert");
    }
    assert!(connection
        .execute(
            "INSERT INTO scheduled_task_runs
                (task_id, task_name, scheduled_for, status)
             VALUES (999, 'Invalid status', 9999, 'stopped')",
            [],
        )
        .is_err());
    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
    assert!(index_exists(&path, "idx_scheduled_task_runs_task"));
    assert!(index_exists(&path, "idx_scheduled_task_runs_status"));
}

#[test]
fn scheduler_migration_disables_and_preserves_legacy_commands() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("auth.sqlite");
    let connection = Connection::open(&path).expect("version one database should open");
    connection
        .execute_batch(
            "
            CREATE TABLE scheduled_tasks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT    NOT NULL,
                command     TEXT    NOT NULL,
                cron        TEXT    NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 1,
                last_run_at REAL,
                last_status TEXT    NOT NULL DEFAULT '',
                created_at  REAL    NOT NULL,
                updated_at  REAL    NOT NULL
            );
            INSERT INTO scheduled_tasks
                (id, name, command, cron, enabled, last_run_at, last_status,
                 created_at, updated_at)
            VALUES
                (9, 'Legacy shell task', 'index --update && push', '0 1 * * *',
                 1, 20.0, 'success', 10.0, 21.0);
            PRAGMA user_version = 1;
            ",
        )
        .expect("version one fixture should be created");
    drop(connection);

    migrate_auth_database(&path).expect("version one database should migrate");

    let connection = Connection::open(&path).expect("migrated database should open");
    let task: (Option<String>, Option<String>, i64, Option<f64>, String) = connection
        .query_row(
            "SELECT job_spec, legacy_command, enabled, last_run_at, last_status
             FROM scheduled_tasks WHERE id = 9",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("migrated scheduled task should remain");

    assert_eq!(
        task,
        (
            None,
            Some("index --update && push".to_string()),
            0,
            Some(20.0),
            "success".to_string(),
        )
    );
    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
}

#[test]
fn scheduler_durable_migration_preserves_tasks_and_adds_safe_defaults() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("auth.sqlite");
    let connection = Connection::open(&path).expect("version two database should open");
    connection
        .execute_batch(
            r#"
            CREATE TABLE scheduled_tasks (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                name           TEXT NOT NULL,
                job_spec       TEXT,
                legacy_command TEXT,
                cron           TEXT NOT NULL,
                enabled        INTEGER NOT NULL DEFAULT 1,
                last_run_at    REAL,
                last_status    TEXT NOT NULL DEFAULT '',
                created_at     REAL NOT NULL,
                updated_at     REAL NOT NULL
            );
            INSERT INTO scheduled_tasks
                (id, name, job_spec, legacy_command, cron, enabled, created_at, updated_at)
            VALUES
                (12, 'Typed task', '{"kind":"index","notify":false,"push":false}',
                 NULL, '0 8 * * *', 1, 1.0, 2.0),
                (13, 'Legacy task', NULL, 'index --update', '0 9 * * *', 0, 1.0, 2.0);
            PRAGMA user_version = 2;
            "#,
        )
        .expect("version two fixture should be created");
    drop(connection);

    migrate_auth_database(&path).expect("version two database should migrate");

    let connection = Connection::open(&path).expect("migrated database should open");
    let defaults: (String, i64, i64) = connection
        .query_row(
            "SELECT timezone, timeout_seconds, coalesce FROM scheduled_tasks WHERE id = 12",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("scheduler defaults should load");
    let legacy_enabled: i64 = connection
        .query_row(
            "SELECT enabled FROM scheduled_tasks WHERE id = 13",
            [],
            |row| row.get(0),
        )
        .expect("legacy task should remain");
    let cursor: Option<f64> = connection
        .query_row(
            "SELECT last_checked_at FROM scheduler_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("scheduler cursor should exist");

    assert_eq!(defaults, ("UTC".to_string(), 3_600, 1));
    assert_eq!(legacy_enabled, 0);
    assert_eq!(cursor, None);
    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
}

#[test]
fn legacy_auth_database_migration_preserves_rows_and_adds_columns() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("auth.sqlite");
    let connection = Connection::open(&path).expect("legacy auth database should open");
    connection
        .execute_batch(
            "
            CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE COLLATE NOCASE,
                password_hash TEXT NOT NULL,
                salt TEXT NOT NULL,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            CREATE TABLE notification_settings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL UNIQUE,
                keywords TEXT NOT NULL DEFAULT '[]',
                directions TEXT NOT NULL DEFAULT '[]',
                delivery_method TEXT NOT NULL DEFAULT 'folder',
                pushplus_token TEXT NOT NULL DEFAULT '',
                pushplus_template TEXT NOT NULL DEFAULT 'markdown',
                pushplus_topic TEXT NOT NULL DEFAULT '',
                pushplus_channel TEXT NOT NULL DEFAULT 'wechat',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            CREATE TABLE announcements (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                message TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            INSERT INTO users
                (id, username, password_hash, salt, created_at, updated_at)
            VALUES
                (7, 'legacy-user', 'legacy-hash', 'legacy-salt', 10.0, 11.0);
            INSERT INTO notification_settings
                (user_id, keywords, directions, delivery_method, pushplus_token,
                 pushplus_template, pushplus_topic, pushplus_channel, enabled,
                 created_at, updated_at)
            VALUES
                (7, '[\"legacy\"]', '[]', 'folder', '', 'markdown', '',
                 'wechat', 1, 12.0, 13.0);
            INSERT INTO announcements
                (title, message, enabled, created_at, updated_at)
            VALUES
                ('Legacy notice', 'Preserve me', 1, 14.0, 15.0);
            ",
        )
        .expect("legacy auth schema should be created");
    drop(connection);

    migrate_auth_database(&path).expect("legacy auth database should migrate");

    let connection = Connection::open(&path).expect("migrated auth database should open");
    let user: (String, String, i64) = connection
        .query_row(
            "SELECT username, password_hash, is_admin FROM users WHERE id = 7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("legacy user should remain");
    let notification: (String, String, i64) = connection
        .query_row(
            "SELECT keywords, selected_databases, ai_retry_attempts
             FROM notification_settings WHERE user_id = 7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("legacy notification settings should remain");
    let priority: String = connection
        .query_row(
            "SELECT priority FROM announcements WHERE title = 'Legacy notice'",
            [],
            |row| row.get(0),
        )
        .expect("legacy announcement should remain");

    assert_eq!(
        user,
        ("legacy-user".to_string(), "legacy-hash".to_string(), 1)
    );
    assert_eq!(
        notification,
        ("[\"legacy\"]".to_string(), "[]".to_string(), 3)
    );
    assert_eq!(priority, "normal");
    assert_eq!(user_version(&path), AUTH_SCHEMA_VERSION);
}

#[test]
fn empty_index_database_migration_creates_exact_provider_neutral_schema() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("index.sqlite");

    migrate_index_database(&path, Some(Path::new("missing-tokenizer")))
        .expect("empty index database should migrate without a tokenizer extension");

    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
    assert_eq!(
        content_table_names(&path),
        [
            "article_change_events",
            "article_identity_keys",
            "article_listing",
            "article_retraction_dois",
            "article_search",
            "articles",
            "issues",
            "journal_identity_keys",
            "journals",
        ]
    );
    assert_eq!(
        table_columns(&path, "journals"),
        [
            "journal_id",
            "catalog_id",
            "title",
            "title_aliases_json",
            "issns_json",
            "issn",
            "eissn",
            "area",
            "utd_rank",
            "utd_rating",
            "abs_rank",
            "abs_rating",
            "fms_rank",
            "fms_rating",
            "fmscn_rank",
            "fmscn_rating",
        ]
    );
    assert_eq!(
        table_columns(&path, "journal_identity_keys"),
        ["identity_kind", "identity_value", "canonical_catalog_id"]
    );
    assert_eq!(
        table_columns(&path, "articles"),
        [
            "article_id",
            "journal_id",
            "issue_id",
            "title",
            "publication_year",
            "date",
            "authors_json",
            "start_page",
            "end_page",
            "abstract_text",
            "doi",
            "pmid",
            "open_access",
            "in_press",
        ]
    );
    assert_eq!(
        table_columns(&path, "article_retraction_dois"),
        ["article_id", "retraction_doi"]
    );
    assert!(index_exists(&path, "idx_article_identity_keys_article"));
    assert!(index_exists(&path, "idx_article_change_events_revision"));
    assert!(index_exists(&path, "idx_article_retraction_dois_doi"));
    assert!(index_exists(&path, "idx_journal_identity_keys_catalog"));

    let schema = sqlite_schema_sql(&path);
    assert!(schema.contains("identity_kind in ('catalog_id', 'issn')"));
    assert_eq!(foreign_key_count(&path, "journal_identity_keys"), 0);
    assert_eq!(foreign_key_count(&path, "article_retraction_dois"), 1);
    for forbidden in [
        "provider",
        "source_csv",
        "library_id",
        "platform_id",
        "url",
        "permalink",
        "content_location",
        "full_text",
        "checkpoint",
        "lease",
        "index_runs",
        "stats",
    ] {
        assert!(!schema.contains(forbidden), "found {forbidden}");
    }
}

#[test]
fn version_four_index_migration_preserves_content_and_seeds_identity_keys() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("version-four.sqlite");
    create_version_four_index_database(&path, true);
    let before = index_content_snapshot(&path);

    migrate_index_database(&path, None).expect("version four index should migrate");

    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
    assert_eq!(index_content_snapshot(&path), before);
    assert_eq!(
        query_text_rows(
            &path,
            "SELECT identity_kind || '|' || identity_value || '|' || canonical_catalog_id
             FROM journal_identity_keys
             ORDER BY identity_kind, identity_value",
        ),
        [
            "catalog_id|journal-1|journal-1",
            "catalog_id|journal-2|journal-2",
            "issn|1234-5679|journal-1",
            "issn|2049-3630|journal-1",
            "issn|2434-561X|journal-2",
        ]
    );
    assert_eq!(foreign_key_count(&path, "journal_identity_keys"), 0);
    let after_first = fs::read(&path).expect("migrated index bytes should read");

    migrate_index_database(&path, None).expect("current version should be a no-op");

    assert_eq!(
        fs::read(&path).expect("no-op index bytes should read"),
        after_first
    );
}

#[test]
fn version_five_index_migration_discards_legacy_scalar_and_preserves_other_content() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("version-five.sqlite");
    create_version_five_index_database(&path);
    let before = index_content_snapshot(&path);
    let identity_before = query_text_rows(
        &path,
        "SELECT json_array(identity_kind, identity_value, canonical_catalog_id)
         FROM journal_identity_keys ORDER BY identity_kind, identity_value",
    );
    assert_eq!(
        query_text_rows(
            &path,
            "SELECT retraction_doi FROM articles WHERE article_id = 100"
        ),
        ["10.1000/legacy-relation"]
    );

    migrate_index_database(&path, None).expect("version five index should migrate");

    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
    assert_eq!(index_content_snapshot(&path), before);
    assert_eq!(
        query_text_rows(
            &path,
            "SELECT json_array(identity_kind, identity_value, canonical_catalog_id)
             FROM journal_identity_keys ORDER BY identity_kind, identity_value"
        ),
        identity_before
    );
    assert!(!table_columns(&path, "articles")
        .iter()
        .any(|column| column == "retraction_doi"));
    assert_eq!(table_row_count(&path, "article_retraction_dois"), 0);
    assert_eq!(foreign_key_violation_count(&path), 0);
}

#[test]
fn empty_version_four_index_migration_creates_an_empty_identity_map() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("empty-version-four.sqlite");
    create_version_four_index_database(&path, false);

    migrate_index_database(&path, None).expect("empty version four index should migrate");

    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
    assert!(table_exists(&path, "journal_identity_keys"));
    assert_eq!(table_row_count(&path, "journal_identity_keys"), 0);
}

#[test]
fn version_four_identity_conflict_rolls_back_atomically() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("conflicting-version-four.sqlite");
    create_version_four_index_database(&path, true);
    let connection = Connection::open(&path).expect("conflict fixture should open");
    connection
        .execute(
            "UPDATE journals
             SET issns_json = '[\"1234-5679\"]', issn = '1234-5679', eissn = NULL
             WHERE catalog_id = 'journal-2'",
            [],
        )
        .expect("conflicting ISSN owner should be installed");
    drop(connection);
    let before = index_content_snapshot(&path);

    let error = migrate_index_database(&path, None)
        .expect_err("conflicting version four identities should fail");

    assert!(matches!(&error, MigrationError::IndexIdentityConflict));
    assert_eq!(
        error.to_string(),
        "index journal identity ownership conflicts across legacy journal rows"
    );
    assert_eq!(user_version(&path), 4);
    assert!(!table_exists(&path, "journal_identity_keys"));
    assert!(!index_exists(&path, "idx_journal_identity_keys_catalog"));
    assert_eq!(index_content_snapshot(&path), before);
}

#[test]
fn pre_v4_index_versions_require_rebuild_without_modifying_files() {
    let temp_dir = tempdir().expect("temp directory should be created");

    for version in 0..4 {
        let path = temp_dir.path().join(format!("legacy-v{version}.sqlite"));
        create_nonempty_index_database(&path, version);
        let before = fs::read(&path).expect("legacy bytes should read");

        let error = migrate_index_database(&path, None)
            .expect_err("legacy index database should require a rebuild");

        match error {
            MigrationError::IndexRebuildRequired {
                path: error_path,
                found,
                required,
            } => {
                assert_eq!(error_path, path);
                assert_eq!(found, version);
                assert_eq!(required, INDEX_SCHEMA_VERSION);
            }
            other => panic!("unexpected migration error: {other}"),
        }
        assert_eq!(
            fs::read(&path).expect("legacy bytes should remain readable"),
            before,
            "version {version} changed"
        );
    }
}

#[test]
fn malformed_current_index_schema_is_rejected_without_modifying_files() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("malformed-current.sqlite");
    migrate_index_database(&path, None).expect("current index database should initialize");
    let connection = Connection::open(&path).expect("current index database should open");
    connection
        .execute("ALTER TABLE articles ADD COLUMN provider TEXT", [])
        .expect("forbidden fixture column should be added");
    drop(connection);
    let before = fs::read(&path).expect("malformed current bytes should read");

    migrate_index_database(&path, None).expect_err("malformed current schema should be rejected");

    assert_eq!(
        fs::read(&path).expect("malformed current bytes should remain readable"),
        before
    );
}

#[test]
fn current_database_migrations_are_idempotent() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let auth_path = temp_dir.path().join("auth.sqlite");
    let index_path = temp_dir.path().join("index.sqlite");
    migrate_auth_database(&auth_path).expect("auth database should migrate once");
    migrate_index_database(&index_path, None).expect("index database should migrate once");
    let auth_before = fs::read(&auth_path).expect("auth database bytes should read");
    let index_before = fs::read(&index_path).expect("index database bytes should read");

    migrate_auth_database(&auth_path).expect("current auth database should be a no-op");
    migrate_index_database(&index_path, None).expect("current index database should be a no-op");

    assert_eq!(
        fs::read(&auth_path).expect("auth bytes should read"),
        auth_before
    );
    assert_eq!(
        fs::read(&index_path).expect("index bytes should read"),
        index_before
    );
}

#[test]
fn failed_auth_migration_rolls_back_schema_changes() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("broken-auth.sqlite");
    let connection = Connection::open(&path).expect("broken auth database should open");
    connection
        .execute_batch(
            "
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                username TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                salt TEXT NOT NULL,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            CREATE TABLE access_tokens (id INTEGER PRIMARY KEY);
            INSERT INTO users
                (id, username, password_hash, salt, created_at, updated_at)
            VALUES
                (1, 'rollback-user', 'hash', 'salt', 1.0, 1.0);
            ",
        )
        .expect("broken auth fixture should be created");
    drop(connection);

    migrate_auth_database(&path).expect_err("invalid legacy auth schema should fail");

    assert_eq!(user_version(&path), 0);
    assert!(!table_columns(&path, "users").contains(&"is_admin".to_string()));
    assert!(!table_exists(&path, "folders"));
    let connection = Connection::open(&path).expect("rolled back auth database should open");
    let username: String = connection
        .query_row("SELECT username FROM users WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("original user should remain");
    assert_eq!(username, "rollback-user");
}

#[test]
fn storage_migration_rejects_legacy_index_without_modifying_it() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let config = StorageConfig::from_project_root(temp_dir.path());
    fs::create_dir_all(config.index_dir()).expect("index directory should be created");
    let index_path = config.index_dir().join("legacy.sqlite");
    create_nonempty_index_database(&index_path, 3);
    let before = fs::read(&index_path).expect("legacy bytes should read");

    let error = migrate_storage(&config).expect_err("legacy index should stop storage migration");

    assert!(matches!(
        error,
        MigrationError::IndexRebuildRequired {
            path,
            found: 3,
            required: INDEX_SCHEMA_VERSION,
        } if path == index_path
    ));
    assert_eq!(
        fs::read(&index_path).expect("legacy bytes should remain readable"),
        before
    );
    assert_eq!(user_version(config.auth_db_path()), AUTH_SCHEMA_VERSION);
}

#[test]
fn newer_database_migrations_fail_without_modifying_files() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let auth_path = temp_dir.path().join("future-auth.sqlite");
    let index_path = temp_dir.path().join("future-index.sqlite");
    create_future_database(&auth_path, AUTH_SCHEMA_VERSION + 1);
    create_future_database(&index_path, INDEX_SCHEMA_VERSION + 1);
    let auth_before = fs::read(&auth_path).expect("future auth bytes should read");
    let index_before = fs::read(&index_path).expect("future index bytes should read");

    let auth_error =
        migrate_auth_database(&auth_path).expect_err("newer auth database should be rejected");
    let index_error = migrate_index_database(&index_path, None)
        .expect_err("newer index database should be rejected");

    assert!(matches!(
        auth_error,
        MigrationError::UnsupportedSchemaVersion {
            database: "auth",
            found,
            supported: AUTH_SCHEMA_VERSION,
        } if found == AUTH_SCHEMA_VERSION + 1
    ));
    assert!(matches!(
        index_error,
        MigrationError::UnsupportedSchemaVersion {
            database: "index",
            found,
            supported: INDEX_SCHEMA_VERSION,
        } if found == INDEX_SCHEMA_VERSION + 1
    ));
    assert_eq!(
        fs::read(&auth_path).expect("future auth bytes should read"),
        auth_before
    );
    assert_eq!(
        fs::read(&index_path).expect("future index bytes should read"),
        index_before
    );
}

#[test]
fn storage_migration_discovers_existing_index_databases() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let config = StorageConfig::from_project_root(temp_dir.path());
    fs::create_dir_all(config.index_dir()).expect("index directory should be created");
    let index_path = config.index_dir().join("fixture.sqlite");
    Connection::open(&index_path).expect("empty index database should be created");

    migrate_storage(&config).expect("configured databases should migrate");

    assert_eq!(user_version(config.auth_db_path()), AUTH_SCHEMA_VERSION);
    assert_eq!(user_version(&index_path), INDEX_SCHEMA_VERSION);
}

#[test]
fn repository_reads_do_not_run_migrations() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let config = StorageConfig::from_project_root(temp_dir.path());

    count_users(config.auth_db_path()).expect_err("unmigrated auth read should fail");
    assert!(!table_exists(config.auth_db_path(), "users"));

    fs::create_dir_all(config.index_dir()).expect("index directory should be created");
    let index_path = config.index_dir().join("legacy.sqlite");
    let connection = Connection::open(&index_path).expect("legacy index database should open");
    connection
        .execute_batch(
            "
            CREATE TABLE journals (
                journal_id INTEGER PRIMARY KEY,
                library_id TEXT NOT NULL,
                title TEXT,
                issn TEXT,
                eissn TEXT,
                scimago_rank REAL,
                cover_url TEXT,
                available INTEGER,
                toc_data_approved_and_live INTEGER,
                has_articles INTEGER
            );
            CREATE TABLE journal_meta (
                journal_id INTEGER PRIMARY KEY,
                source_csv TEXT NOT NULL,
                area TEXT,
                csv_title TEXT,
                csv_issn TEXT,
                csv_library TEXT
            );
            ",
        )
        .expect("legacy read fixture should be created");
    drop(connection);

    get_journal(&config, Some("legacy.sqlite"), 1).expect_err("unmigrated index read should fail");

    assert_eq!(user_version(&index_path), 0);
    assert!(!table_columns(&index_path, "journals").contains(&"platform_journal_id".to_string()));
}

fn create_future_database(path: &Path, version: i64) {
    let connection = Connection::open(path).expect("future database should open");
    connection
        .execute_batch(
            "CREATE TABLE sentinel (value TEXT NOT NULL); INSERT INTO sentinel VALUES ('keep');",
        )
        .expect("future database sentinel should be created");
    connection
        .pragma_update(None, "user_version", version)
        .expect("future schema version should be set");
}

fn user_version(path: &Path) -> i64 {
    let connection = Connection::open(path).expect("database should open for version query");
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version should be readable")
}

fn table_exists(path: &Path, table_name: &str) -> bool {
    let connection = Connection::open(path).expect("database should open for table query");
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?1
             )",
            [table_name],
            |row| row.get(0),
        )
        .expect("table existence should be readable")
}

fn index_exists(path: &Path, index_name: &str) -> bool {
    let connection = Connection::open(path).expect("database should open for index query");
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1
             )",
            [index_name],
            |row| row.get(0),
        )
        .expect("index existence should be readable")
}

fn table_columns(path: &Path, table_name: &str) -> Vec<String> {
    let connection = Connection::open(path).expect("database should open for column query");
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .expect("table columns should prepare");
    let rows = statement
        .query_map([], |row| row.get(1))
        .expect("table columns should query");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("table columns should collect")
}

fn content_table_names(path: &Path) -> Vec<String> {
    let connection = Connection::open(path).expect("database should open for table inventory");
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
               AND name NOT LIKE 'article_search_%'
             ORDER BY name",
        )
        .expect("table inventory should prepare");
    let rows = statement
        .query_map([], |row| row.get(0))
        .expect("table inventory should query");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("table inventory should collect")
}

fn sqlite_schema_sql(path: &Path) -> String {
    Connection::open(path)
        .expect("database should open for schema SQL")
        .query_row(
            "SELECT group_concat(sql, '\n') FROM sqlite_schema WHERE sql IS NOT NULL",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("schema SQL should load")
        .to_ascii_lowercase()
}

fn create_version_four_index_database(path: &Path, has_content: bool) {
    migrate_index_database(path, None).expect("current index fixture should initialize");
    let connection = Connection::open(path).expect("version four fixture should open");
    connection
        .execute_batch(
            "DROP TABLE article_retraction_dois;
             ALTER TABLE articles ADD COLUMN retraction_doi TEXT;
             DROP TABLE journal_identity_keys;
             PRAGMA user_version = 4;",
        )
        .expect("current-only index objects should be removed");
    if has_content {
        connection
            .execute_batch(
                r#"
                INSERT INTO journals (
                    journal_id, catalog_id, title, title_aliases_json, issns_json,
                    issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating,
                    fms_rank, fms_rating, fmscn_rank, fmscn_rating
                ) VALUES
                    (1, 'journal-1', 'Journal One', '[]', '["1234-5679","2049-3630"]',
                     '1234-5679', '2049-3630', 'Area One', NULL, NULL, NULL, NULL,
                     NULL, NULL, NULL, NULL),
                    (2, 'journal-2', 'Journal Two', '[]', '["2434-561X"]',
                     '2434-561X', NULL, 'Area Two', NULL, NULL, NULL, NULL,
                     NULL, NULL, NULL, NULL);

                INSERT INTO issues (
                    issue_id, journal_id, publication_year, title, volume, number, date
                ) VALUES (10, 1, 2026, 'Issue One', '1', '1', '2026-01-01');

                INSERT INTO articles (
                    article_id, journal_id, issue_id, title, publication_year, date,
                    authors_json, start_page, end_page, abstract_text, doi, pmid,
                    open_access, in_press, retraction_doi
                ) VALUES (
                    100, 1, 10, 'Article One', 2026, '2026-01-01', '["Author"]',
                    '1', '9', 'Abstract', '10.1000/article-one', NULL, 1, 0, NULL
                );

                INSERT INTO article_identity_keys (
                    identity_kind, identity_value, article_id
                ) VALUES
                    ('doi', '10.1000/article-one', 100),
                    ('bibliographic', 'journal-1|2026|article one|1', 100);

                INSERT INTO article_listing (
                    article_id, journal_id, issue_id, publication_year, date,
                    open_access, in_press, doi, pmid, area
                ) VALUES (
                    100, 1, 10, 2026, '2026-01-01', 1, 0,
                    '10.1000/article-one', NULL, 'Area One'
                );

                INSERT INTO article_search (
                    rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
                ) VALUES (
                    100, 100, 'Article One', 'Abstract', '10.1000/article-one', '',
                    'Author', 'Journal One'
                );

                INSERT INTO article_change_events (
                    event_id, content_revision, article_id, change_kind, journal_id,
                    issue_id, in_press, created_at
                ) VALUES (
                    1000, 'fixture:revision', 100, 'upsert', 1, 10, 0,
                    '2026-07-20T00:00:00Z'
                );
                "#,
            )
            .expect("version four content should be inserted");
    }
}

fn create_version_five_index_database(path: &Path) {
    create_version_four_index_database(path, true);
    let connection = Connection::open(path).expect("version five fixture should open");
    connection
        .execute_batch(
            "CREATE TABLE journal_identity_keys (
                 identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
                 identity_value TEXT NOT NULL,
                 canonical_catalog_id TEXT NOT NULL,
                 PRIMARY KEY (identity_kind, identity_value)
             );
             CREATE INDEX idx_journal_identity_keys_catalog
                 ON journal_identity_keys(canonical_catalog_id);
             INSERT INTO journal_identity_keys (
                 identity_kind, identity_value, canonical_catalog_id
             ) VALUES
                 ('catalog_id', 'journal-1', 'journal-1'),
                 ('catalog_id', 'journal-2', 'journal-2'),
                 ('issn', '1234-5679', 'journal-1'),
                 ('issn', '2049-3630', 'journal-1'),
                 ('issn', '2434-561X', 'journal-2');
             UPDATE articles SET retraction_doi = '10.1000/legacy-relation';
             PRAGMA user_version = 5;",
        )
        .expect("version five identity and scalar state should be installed");
}

fn index_content_snapshot(path: &Path) -> Vec<Vec<String>> {
    [
        "SELECT json_array(journal_id, catalog_id, title, title_aliases_json, issns_json, issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating, fms_rank, fms_rating, fmscn_rank, fmscn_rating) FROM journals ORDER BY journal_id",
        "SELECT json_array(issue_id, journal_id, publication_year, title, volume, number, date) FROM issues ORDER BY issue_id",
        "SELECT json_array(article_id, journal_id, issue_id, title, publication_year, date, authors_json, start_page, end_page, abstract_text, doi, pmid, open_access, in_press) FROM articles ORDER BY article_id",
        "SELECT json_array(identity_kind, identity_value, article_id) FROM article_identity_keys ORDER BY identity_kind, identity_value",
        "SELECT json_array(article_id, journal_id, issue_id, publication_year, date, open_access, in_press, doi, pmid, area) FROM article_listing ORDER BY article_id",
        "SELECT json_array(rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title) FROM article_search ORDER BY rowid",
        "SELECT json_array(event_id, content_revision, article_id, change_kind, journal_id, issue_id, in_press, created_at) FROM article_change_events ORDER BY event_id",
    ]
    .into_iter()
    .map(|query| query_text_rows(path, query))
    .collect()
}

fn query_text_rows(path: &Path, query: &str) -> Vec<String> {
    let connection = Connection::open(path).expect("database should open for text query");
    let mut statement = connection
        .prepare(query)
        .expect("text query should prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("text rows should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("text rows should collect")
}

fn foreign_key_count(path: &Path, table_name: &str) -> i64 {
    Connection::open(path)
        .expect("database should open for foreign key query")
        .query_row(
            &format!("SELECT COUNT(*) FROM pragma_foreign_key_list('{table_name}')"),
            [],
            |row| row.get(0),
        )
        .expect("foreign key count should read")
}

fn foreign_key_violation_count(path: &Path) -> i64 {
    Connection::open(path)
        .expect("database should open for foreign key check")
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign key violations should be readable")
}

fn table_row_count(path: &Path, table_name: &str) -> i64 {
    Connection::open(path)
        .expect("database should open for row count")
        .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
            row.get(0)
        })
        .expect("table row count should read")
}

fn create_nonempty_index_database(path: &Path, version: i64) {
    let connection = Connection::open(path).expect("legacy index database should open");
    connection
        .execute_batch(
            "CREATE TABLE legacy_articles (
                 article_id INTEGER PRIMARY KEY,
                 provider TEXT NOT NULL
             );
             INSERT INTO legacy_articles (article_id, provider) VALUES (1, 'fixture');",
        )
        .expect("legacy index fixture should initialize");
    connection
        .pragma_update(None, "user_version", version)
        .expect("legacy index version should be set");
}
