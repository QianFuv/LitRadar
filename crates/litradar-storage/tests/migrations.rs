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
}

#[test]
fn service_heartbeat_migration_preserves_version_three_scheduler_rows() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("auth.sqlite");
    migrate_auth_database(&path).expect("current auth database should migrate");
    let connection = Connection::open(&path).expect("auth database should open");
    connection
        .execute_batch(
            "DROP TABLE service_heartbeats;
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
fn empty_index_database_migration_creates_current_schema() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("index.sqlite");

    migrate_index_database(&path, None).expect("empty index database should migrate");

    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
    assert!(table_exists(&path, "journals"));
    assert!(table_exists(&path, "articles"));
    assert!(table_exists(&path, "article_search"));
    assert!(table_columns(&path, "journals").contains(&"platform_journal_id".to_string()));
}

#[test]
fn legacy_index_database_migration_preserves_rows_and_adds_platform_id() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("legacy-index.sqlite");
    let connection = Connection::open(&path).expect("legacy index database should open");
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
            INSERT INTO journals
                (journal_id, library_id, title, issn, available, has_articles)
            VALUES
                (42, 'scholarly', 'Legacy Journal', '1234-5678', 1, 1);
            ",
        )
        .expect("legacy index schema should be created");
    drop(connection);

    migrate_index_database(&path, None).expect("legacy index database should migrate");

    let connection = Connection::open(&path).expect("migrated index database should open");
    let journal: (String, Option<String>) = connection
        .query_row(
            "SELECT title, platform_journal_id FROM journals WHERE journal_id = 42",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy journal should remain");

    assert_eq!(journal, ("Legacy Journal".to_string(), None));
    assert_eq!(user_version(&path), INDEX_SCHEMA_VERSION);
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
fn failed_index_migration_rolls_back_schema_changes() {
    let temp_dir = tempdir().expect("temp directory should be created");
    let path = temp_dir.path().join("broken-index.sqlite");
    let connection = Connection::open(&path).expect("broken index database should open");
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
            CREATE TABLE articles (article_id INTEGER PRIMARY KEY);
            ",
        )
        .expect("broken index fixture should be created");
    drop(connection);

    migrate_index_database(&path, None).expect_err("invalid legacy index schema should fail");

    assert_eq!(user_version(&path), 0);
    assert!(!table_columns(&path, "journals").contains(&"platform_journal_id".to_string()));
    assert!(!table_exists(&path, "issues"));
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
