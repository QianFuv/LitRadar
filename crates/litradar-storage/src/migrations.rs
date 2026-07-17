//! Ordered, transactional migrations for auth and index SQLite databases.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{
    params, params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior,
};

use crate::{try_load_extension, DatabaseResolutionError, StorageConfig};

/// Current auth and business database schema version.
pub const AUTH_SCHEMA_VERSION: i64 = 6;

/// Current index database schema version.
pub const INDEX_SCHEMA_VERSION: i64 = 3;

const AUTH_DATABASE: &str = "auth";
const INDEX_DATABASE: &str = "index";
const BUSY_TIMEOUT_SECONDS: u64 = 30;
const PROJECTION_RECONCILE_BATCH_SIZE: i64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MigrationSummary {
    from_version: i64,
    to_version: i64,
}

/// Errors returned while discovering or migrating SQLite databases.
#[derive(Debug)]
pub enum MigrationError {
    /// Filesystem setup failed.
    Io(std::io::Error),
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// Index database discovery failed.
    DatabaseResolution(DatabaseResolutionError),
    /// A database was created by a newer application schema.
    UnsupportedSchemaVersion {
        /// Database family being migrated.
        database: &'static str,
        /// Version stored in the database.
        found: i64,
        /// Highest version supported by this binary.
        supported: i64,
    },
}

impl fmt::Display for MigrationError {
    /// Format a migration error without exposing database contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::DatabaseResolution(error) => write!(formatter, "{error}"),
            Self::UnsupportedSchemaVersion {
                database,
                found,
                supported,
            } => write!(
                formatter,
                "unsupported {database} database schema version {found}; this binary supports up to {supported}"
            ),
        }
    }
}

impl Error for MigrationError {
    /// Return the underlying IO, SQLite, or discovery error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::DatabaseResolution(error) => Some(error),
            Self::UnsupportedSchemaVersion { .. } => None,
        }
    }
}

impl From<std::io::Error> for MigrationError {
    /// Convert filesystem errors into migration errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for MigrationError {
    /// Convert SQLite errors into migration errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<DatabaseResolutionError> for MigrationError {
    /// Convert index discovery errors into migration errors.
    fn from(error: DatabaseResolutionError) -> Self {
        Self::DatabaseResolution(error)
    }
}

/// Migrate the configured auth database and every existing index database.
///
/// # Arguments
///
/// * `config` - Storage paths rooted at the active project directory.
///
/// # Returns
///
/// Empty result after every configured database reaches its current version.
pub fn migrate_storage(config: &StorageConfig) -> Result<(), MigrationError> {
    migrate_auth_database(config.auth_db_path())?;
    migrate_existing_index_databases(config)
}

/// Migrate every existing index database discovered by a storage configuration.
///
/// # Arguments
///
/// * `config` - Storage paths used to discover index databases and the optional tokenizer.
///
/// # Returns
///
/// Empty result after all discovered index databases reach the current version.
pub fn migrate_existing_index_databases(config: &StorageConfig) -> Result<(), MigrationError> {
    let started_at = Instant::now();
    tracing::info!(
        event = "storage.migration.batch.started",
        component = "storage",
        database_kind = INDEX_DATABASE,
        target_version = INDEX_SCHEMA_VERSION,
    );
    let tokenizer_path = config.simple_tokenizer_path();
    let paths = match config.list_index_databases() {
        Ok(paths) => paths,
        Err(error) => {
            let error = MigrationError::from(error);
            tracing::warn!(
                event = "storage.migration.batch.failed",
                component = "storage",
                database_kind = INDEX_DATABASE,
                target_version = INDEX_SCHEMA_VERSION,
                discovered_count = 0,
                completed_count = 0,
                duration_ms = started_at.elapsed().as_millis() as u64,
                error_kind = migration_error_kind(&error),
            );
            return Err(error);
        }
    };
    let discovered_count = paths.len();
    let mut completed_count = 0_usize;
    for path in paths {
        if let Err(error) = migrate_index_database(path, tokenizer_path.as_deref()) {
            tracing::warn!(
                event = "storage.migration.batch.failed",
                component = "storage",
                database_kind = INDEX_DATABASE,
                target_version = INDEX_SCHEMA_VERSION,
                discovered_count,
                completed_count,
                duration_ms = started_at.elapsed().as_millis() as u64,
                error_kind = migration_error_kind(&error),
            );
            return Err(error);
        }
        completed_count += 1;
    }
    tracing::info!(
        event = "storage.migration.batch.completed",
        component = "storage",
        database_kind = INDEX_DATABASE,
        target_version = INDEX_SCHEMA_VERSION,
        discovered_count,
        completed_count,
        duration_ms = started_at.elapsed().as_millis() as u64,
    );
    Ok(())
}

/// Migrate one auth and business database to the current schema version.
///
/// # Arguments
///
/// * `path` - Auth SQLite database path.
///
/// # Returns
///
/// Empty result after all pending migrations commit.
pub fn migrate_auth_database(path: impl AsRef<Path>) -> Result<(), MigrationError> {
    run_database_migration(AUTH_DATABASE, AUTH_SCHEMA_VERSION, || {
        migrate_auth_database_inner(path.as_ref())
    })
}

fn migrate_auth_database_inner(path: &Path) -> Result<MigrationSummary, MigrationError> {
    let connection = open_migration_connection(path)?;
    let mut version = schema_version(&connection)?;
    let from_version = version;
    reject_newer_version(AUTH_DATABASE, version, AUTH_SCHEMA_VERSION)?;
    if version == AUTH_SCHEMA_VERSION {
        return Ok(MigrationSummary {
            from_version,
            to_version: version,
        });
    }
    configure_writable_connection(&connection)?;

    while version < AUTH_SCHEMA_VERSION {
        let next_version = version + 1;
        let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
        match next_version {
            1 => apply_auth_version_one(&transaction)?,
            2 => apply_auth_version_two(&transaction)?,
            3 => apply_auth_version_three(&transaction)?,
            4 => apply_auth_version_four(&transaction)?,
            5 => apply_auth_version_five(&transaction)?,
            6 => apply_auth_version_six(&transaction)?,
            _ => unreachable!("auth migration version should be implemented"),
        }
        transaction.pragma_update(None, "user_version", next_version)?;
        transaction.commit()?;
        version = next_version;
    }
    Ok(MigrationSummary {
        from_version,
        to_version: version,
    })
}

/// Migrate one index database to the current schema version.
///
/// # Arguments
///
/// * `path` - Index SQLite database path.
/// * `simple_tokenizer_path` - Optional SQLite `simple` tokenizer extension path.
///
/// # Returns
///
/// Empty result after all pending migrations commit.
pub fn migrate_index_database(
    path: impl AsRef<Path>,
    simple_tokenizer_path: Option<&Path>,
) -> Result<(), MigrationError> {
    run_database_migration(INDEX_DATABASE, INDEX_SCHEMA_VERSION, || {
        migrate_index_database_inner(path.as_ref(), simple_tokenizer_path)
    })
}

fn migrate_index_database_inner(
    path: &Path,
    simple_tokenizer_path: Option<&Path>,
) -> Result<MigrationSummary, MigrationError> {
    let connection = open_migration_connection(path)?;
    let mut version = schema_version(&connection)?;
    let from_version = version;
    reject_newer_version(INDEX_DATABASE, version, INDEX_SCHEMA_VERSION)?;
    let existing_simple_tokenizer = article_search_uses_simple_tokenizer(&connection)?;
    if existing_simple_tokenizer != Some(false) {
        let resolved_path = resolve_simple_tokenizer_path(simple_tokenizer_path)
            .ok_or_else(missing_simple_tokenizer_error)?;
        try_load_extension(&connection, Some(&resolved_path))?;
        if existing_simple_tokenizer == Some(true) {
            probe_article_search(&connection)?;
        }
    }
    if version == INDEX_SCHEMA_VERSION {
        return Ok(MigrationSummary {
            from_version,
            to_version: version,
        });
    }
    configure_writable_connection(&connection)?;

    while version < INDEX_SCHEMA_VERSION {
        let next_version = version + 1;
        let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
        match next_version {
            1 => apply_index_version_one(&transaction)?,
            2 => apply_index_version_two(&transaction)?,
            3 => apply_index_version_three(&transaction)?,
            _ => unreachable!("index migration version should be implemented"),
        }
        transaction.pragma_update(None, "user_version", next_version)?;
        transaction.commit()?;
        version = next_version;
    }
    Ok(MigrationSummary {
        from_version,
        to_version: version,
    })
}

fn run_database_migration<Migrate>(
    database_kind: &'static str,
    target_version: i64,
    migrate: Migrate,
) -> Result<(), MigrationError>
where
    Migrate: FnOnce() -> Result<MigrationSummary, MigrationError>,
{
    let started_at = Instant::now();
    tracing::info!(
        event = "storage.migration.started",
        component = "storage",
        database_kind,
        target_version,
    );
    match migrate() {
        Ok(summary) => {
            tracing::info!(
                event = "storage.migration.completed",
                component = "storage",
                database_kind,
                target_version,
                from_version = summary.from_version,
                to_version = summary.to_version,
                applied_count = summary.to_version.saturating_sub(summary.from_version),
                duration_ms = started_at.elapsed().as_millis() as u64,
            );
            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                event = "storage.migration.failed",
                component = "storage",
                database_kind,
                target_version,
                database_version = migration_error_database_version(&error),
                duration_ms = started_at.elapsed().as_millis() as u64,
                error_kind = migration_error_kind(&error),
            );
            Err(error)
        }
    }
}

fn migration_error_kind(error: &MigrationError) -> &'static str {
    match error {
        MigrationError::Io(_) => "io",
        MigrationError::Sqlite(_) => "sqlite",
        MigrationError::DatabaseResolution(_) => "database_resolution",
        MigrationError::UnsupportedSchemaVersion { .. } => "unsupported_schema_version",
    }
}

fn migration_error_database_version(error: &MigrationError) -> i64 {
    match error {
        MigrationError::UnsupportedSchemaVersion { found, .. } => *found,
        MigrationError::Io(_)
        | MigrationError::Sqlite(_)
        | MigrationError::DatabaseResolution(_) => -1,
    }
}

fn open_migration_connection(path: &Path) -> Result<Connection, MigrationError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(BUSY_TIMEOUT_SECONDS))?;
    Ok(connection)
}

fn configure_writable_connection(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        ",
    )
}

fn schema_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn reject_newer_version(
    database: &'static str,
    found: i64,
    supported: i64,
) -> Result<(), MigrationError> {
    if found > supported {
        return Err(MigrationError::UnsupportedSchemaVersion {
            database,
            found,
            supported,
        });
    }
    Ok(())
}

fn apply_auth_version_one(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(AUTH_TABLES_SQL)?;

    let user_columns = table_columns(transaction, "users")?;
    if !user_columns.iter().any(|column| column == "is_admin") {
        transaction.execute(
            "ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        transaction.execute(
            "UPDATE users SET is_admin = 1 WHERE id = (SELECT MIN(id) FROM users)",
            [],
        )?;
    }

    let notification_columns = table_columns(transaction, "notification_settings")?;
    for (column, statement) in NOTIFICATION_COLUMN_MIGRATIONS {
        if !notification_columns
            .iter()
            .any(|existing| existing == column)
        {
            transaction.execute(statement, [])?;
        }
    }

    let announcement_columns = table_columns(transaction, "announcements")?;
    if !announcement_columns
        .iter()
        .any(|column| column == "priority")
    {
        transaction.execute(
            "ALTER TABLE announcements ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal'",
            [],
        )?;
    }

    transaction.execute_batch(AUTH_INDEXES_SQL)
}

fn apply_auth_version_two(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        CREATE TABLE scheduled_tasks_v2 (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            name           TEXT    NOT NULL,
            job_spec       TEXT,
            legacy_command TEXT,
            cron           TEXT    NOT NULL,
            enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            last_run_at    REAL,
            last_status    TEXT    NOT NULL DEFAULT '',
            created_at     REAL    NOT NULL,
            updated_at     REAL    NOT NULL,
            CHECK (
                (job_spec IS NOT NULL AND legacy_command IS NULL)
                OR (job_spec IS NULL AND legacy_command IS NOT NULL)
            ),
            CHECK (job_spec IS NOT NULL OR enabled = 0)
        );

        INSERT INTO scheduled_tasks_v2
            (id, name, job_spec, legacy_command, cron, enabled, last_run_at,
             last_status, created_at, updated_at)
        SELECT
            id, name, NULL, command, cron, 0, last_run_at, last_status,
            created_at, updated_at
        FROM scheduled_tasks;

        DROP TABLE scheduled_tasks;
        ALTER TABLE scheduled_tasks_v2 RENAME TO scheduled_tasks;
        CREATE INDEX idx_scheduled_tasks_enabled ON scheduled_tasks(enabled);
        ",
    )
}

fn apply_auth_version_three(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        ALTER TABLE scheduled_tasks
            ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
        ALTER TABLE scheduled_tasks
            ADD COLUMN timeout_seconds INTEGER NOT NULL DEFAULT 3600
            CHECK (timeout_seconds BETWEEN 1 AND 86400);
        ALTER TABLE scheduled_tasks
            ADD COLUMN coalesce INTEGER NOT NULL DEFAULT 1
            CHECK (coalesce IN (0, 1));

        CREATE TABLE scheduler_state (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            last_checked_at REAL
        );

        INSERT INTO scheduler_state (id, last_checked_at) VALUES (1, NULL);

        CREATE TABLE scheduler_workers (
            worker_id    TEXT PRIMARY KEY,
            started_at   REAL NOT NULL,
            heartbeat_at REAL NOT NULL
        );

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
        CREATE INDEX idx_scheduler_workers_heartbeat
            ON scheduler_workers(heartbeat_at DESC);
        ",
    )
}

fn apply_auth_version_four(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        CREATE TABLE service_heartbeats (
            service      TEXT NOT NULL CHECK (service IN ('api', 'worker')),
            instance_id  TEXT NOT NULL,
            started_at   REAL NOT NULL,
            heartbeat_at REAL NOT NULL,
            PRIMARY KEY(service, instance_id)
        );

        CREATE INDEX idx_service_heartbeats_recent
            ON service_heartbeats(heartbeat_at DESC);
        ",
    )
}

fn apply_auth_version_five(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        CREATE TABLE scheduled_task_runs_v5 (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id          INTEGER NOT NULL,
            task_name        TEXT    NOT NULL,
            scheduled_for    INTEGER NOT NULL,
            status           TEXT    NOT NULL
                CHECK (status IN ('pending', 'claimed', 'running', 'success',
                                  'failed', 'timed_out', 'error', 'unknown',
                                  'cancelled')),
            worker_id        TEXT,
            claim_expires_at REAL,
            claimed_at       REAL,
            started_at       REAL,
            finished_at      REAL,
            output_summary   TEXT NOT NULL DEFAULT '',
            UNIQUE(task_id, scheduled_for)
        );

        INSERT INTO scheduled_task_runs_v5
            (id, task_id, task_name, scheduled_for, status, worker_id,
             claim_expires_at, claimed_at, started_at, finished_at,
             output_summary)
        SELECT
            id, task_id, task_name, scheduled_for, status, worker_id,
            claim_expires_at, claimed_at, started_at, finished_at,
            output_summary
        FROM scheduled_task_runs;

        DROP TABLE scheduled_task_runs;
        ALTER TABLE scheduled_task_runs_v5 RENAME TO scheduled_task_runs;
        CREATE INDEX idx_scheduled_task_runs_task
            ON scheduled_task_runs(task_id, scheduled_for DESC);
        CREATE INDEX idx_scheduled_task_runs_status
            ON scheduled_task_runs(status, claim_expires_at);
        ",
    )
}

fn apply_auth_version_six(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        CREATE TABLE managed_meta_catalogs (
            filename       TEXT PRIMARY KEY,
            bundle_version INTEGER NOT NULL CHECK (bundle_version > 0),
            applied_sha256 TEXT NOT NULL CHECK (length(applied_sha256) = 64)
        );
        ",
    )
}

fn apply_index_version_one(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(INDEX_TABLES_SQL)?;
    let journal_columns = table_columns(transaction, "journals")?;
    if !journal_columns
        .iter()
        .any(|column| column == "platform_journal_id")
    {
        transaction.execute(
            "ALTER TABLE journals ADD COLUMN platform_journal_id TEXT",
            [],
        )?;
    }

    transaction.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS article_search
        USING fts5(
            article_id UNINDEXED,
            title,
            abstract,
            doi,
            authors,
            journal_title,
            tokenize = 'simple'
        );
        ",
    )?;
    probe_article_search(transaction)?;
    transaction.execute_batch(INDEX_INDEXES_SQL)
}

fn apply_index_version_two(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let journal_meta_columns = table_columns(transaction, "journal_meta")?;
    for (column, statement) in JOURNAL_META_COLUMN_MIGRATIONS {
        if !journal_meta_columns
            .iter()
            .any(|existing| existing == column)
        {
            transaction.execute(statement, [])?;
        }
    }

    transaction.execute_batch(INDEX_CHANGE_EVENTS_SQL)?;
    validate_required_index_columns(transaction)?;
    reconcile_missing_article_projections(transaction)
}

fn apply_index_version_three(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(INDEX_RUN_LEASE_SQL)?;
    let columns = table_columns(transaction, "index_run_lease")?;
    for required_column in ["id", "run_id", "heartbeat_at", "expires_at"] {
        if !columns.iter().any(|existing| existing == required_column) {
            return Err(rusqlite::Error::InvalidColumnName(format!(
                "index_run_lease.{required_column}"
            )));
        }
    }
    Ok(())
}

fn validate_required_index_columns(connection: &Connection) -> rusqlite::Result<()> {
    for (table, required_columns) in REQUIRED_INDEX_COLUMNS {
        let existing_columns = table_columns(connection, table)?;
        for required_column in *required_columns {
            if !existing_columns
                .iter()
                .any(|existing| existing == required_column)
            {
                return Err(rusqlite::Error::InvalidColumnName(format!(
                    "{table}.{required_column}"
                )));
            }
        }
    }
    Ok(())
}

fn reconcile_missing_article_projections(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let mut last_article_id = None;
    loop {
        let article_ids = missing_projection_article_ids(transaction, last_article_id)?;
        let Some(next_last_article_id) = article_ids.last().copied() else {
            return Ok(());
        };
        reconcile_article_listing(transaction, &article_ids)?;
        reconcile_article_search(transaction, &article_ids)?;
        last_article_id = Some(next_last_article_id);
    }
}

fn missing_projection_article_ids(
    connection: &Connection,
    last_article_id: Option<i64>,
) -> rusqlite::Result<Vec<i64>> {
    let mut statement = connection.prepare(
        "
        SELECT a.article_id
        FROM articles a
        LEFT JOIN article_listing l ON l.article_id = a.article_id
        LEFT JOIN article_search s ON s.rowid = a.article_id
        WHERE (?1 IS NULL OR a.article_id > ?1)
          AND (l.article_id IS NULL OR s.rowid IS NULL)
        ORDER BY a.article_id
        LIMIT ?2
        ",
    )?;
    let rows = statement.query_map(
        params![last_article_id, PROJECTION_RECONCILE_BATCH_SIZE],
        |row| row.get::<_, i64>(0),
    )?;
    rows.collect()
}

fn reconcile_article_listing(connection: &Connection, article_ids: &[i64]) -> rusqlite::Result<()> {
    let placeholders = std::iter::repeat_n("?", article_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        INSERT INTO article_listing (
            article_id, journal_id, issue_id, publication_year, date, open_access,
            in_press, suppressed, within_library_holdings, doi, pmid, area
        )
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            i.publication_year,
            a.date,
            a.open_access,
            a.in_press,
            a.suppressed,
            a.within_library_holdings,
            a.doi,
            a.pmid,
            m.area
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        LEFT JOIN journal_meta m ON m.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
          AND NOT EXISTS (
              SELECT 1 FROM article_listing l WHERE l.article_id = a.article_id
          )
        "
    );
    connection.execute(&sql, params_from_iter(article_ids.iter()))?;
    Ok(())
}

fn reconcile_article_search(connection: &Connection, article_ids: &[i64]) -> rusqlite::Result<()> {
    let placeholders = std::iter::repeat_n("?", article_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        INSERT INTO article_search (
            rowid, article_id, title, abstract, doi, authors, journal_title
        )
        SELECT
            a.article_id,
            a.article_id,
            COALESCE(a.title, ''),
            COALESCE(a.abstract, ''),
            COALESCE(a.doi, ''),
            COALESCE(a.authors, ''),
            COALESCE(j.title, '')
        FROM articles a
        LEFT JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
          AND NOT EXISTS (
              SELECT 1 FROM article_search s WHERE s.rowid = a.article_id
          )
        "
    );
    connection.execute(&sql, params_from_iter(article_ids.iter()))?;
    Ok(())
}

fn article_search_uses_simple_tokenizer(connection: &Connection) -> rusqlite::Result<Option<bool>> {
    let sql = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'article_search'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(sql.map(|value| {
        let compact = value
            .to_ascii_lowercase()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        compact.contains("tokenize='simple'")
            || compact.contains("tokenize=\"simple\"")
            || compact.contains("tokenize=simple")
    }))
}

fn probe_article_search(connection: &Connection) -> rusqlite::Result<()> {
    connection
        .query_row(
            "SELECT rowid FROM article_search WHERE article_search MATCH ?1 LIMIT 1",
            ["litradartokenizerprobe"],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(())
}

fn resolve_simple_tokenizer_path(configured_path: Option<&Path>) -> Option<PathBuf> {
    configured_path.map(Path::to_path_buf).or_else(|| {
        let current_dir = std::env::current_dir().ok()?;
        current_dir.ancestors().find_map(|root| {
            StorageConfig::from_project_root(root)
                .simple_tokenizer_path()
                .filter(|path| path.exists())
        })
    })
}

fn missing_simple_tokenizer_error() -> rusqlite::Error {
    rusqlite::Error::InvalidPath(expected_simple_tokenizer_path())
}

fn expected_simple_tokenizer_path() -> PathBuf {
    let libs_dir = PathBuf::from("libs");
    if cfg!(windows) {
        libs_dir
            .join("simple-windows")
            .join("libsimple-windows-x64")
            .join("simple.dll")
    } else if cfg!(target_os = "linux") {
        libs_dir
            .join("simple-linux")
            .join("libsimple-linux-ubuntu-latest")
            .join("libsimple.so")
    } else {
        libs_dir.join("simple-tokenizer-extension")
    }
}

fn table_columns(connection: &Connection, table_name: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

const JOURNAL_META_COLUMN_MIGRATIONS: &[(&str, &str)] = &[
    (
        "resolved_source",
        "ALTER TABLE journal_meta ADD COLUMN resolved_source TEXT",
    ),
    (
        "resolved_source_id",
        "ALTER TABLE journal_meta ADD COLUMN resolved_source_id TEXT",
    ),
    (
        "resolved_title",
        "ALTER TABLE journal_meta ADD COLUMN resolved_title TEXT",
    ),
    (
        "resolved_issn",
        "ALTER TABLE journal_meta ADD COLUMN resolved_issn TEXT",
    ),
    (
        "resolved_eissn",
        "ALTER TABLE journal_meta ADD COLUMN resolved_eissn TEXT",
    ),
];

const REQUIRED_INDEX_COLUMNS: &[(&str, &[&str])] = &[
    (
        "journals",
        &["journal_id", "library_id", "platform_journal_id", "title"],
    ),
    (
        "journal_meta",
        &[
            "journal_id",
            "source_csv",
            "area",
            "csv_title",
            "csv_issn",
            "csv_library",
            "resolved_source",
            "resolved_source_id",
            "resolved_title",
            "resolved_issn",
            "resolved_eissn",
        ],
    ),
    ("issues", &["issue_id", "journal_id", "publication_year"]),
    (
        "articles",
        &[
            "article_id",
            "journal_id",
            "issue_id",
            "title",
            "date",
            "authors",
            "abstract",
            "doi",
            "pmid",
            "suppressed",
            "in_press",
            "open_access",
            "within_library_holdings",
        ],
    ),
    (
        "article_listing",
        &[
            "article_id",
            "journal_id",
            "issue_id",
            "publication_year",
            "date",
            "open_access",
            "in_press",
            "suppressed",
            "within_library_holdings",
            "doi",
            "pmid",
            "area",
        ],
    ),
    (
        "article_search",
        &[
            "article_id",
            "title",
            "abstract",
            "doi",
            "authors",
            "journal_title",
        ],
    ),
    (
        "index_change_events",
        &[
            "event_id",
            "run_id",
            "worker_id",
            "article_id",
            "event_type",
            "membership_type",
            "journal_id",
            "issue_id",
            "is_backfill",
            "created_at",
        ],
    ),
];

const NOTIFICATION_COLUMN_MIGRATIONS: &[(&str, &str)] = &[
    (
        "selected_databases",
        "ALTER TABLE notification_settings ADD COLUMN selected_databases TEXT NOT NULL DEFAULT '[]'",
    ),
    (
        "ai_base_url",
        "ALTER TABLE notification_settings ADD COLUMN ai_base_url TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_api_key",
        "ALTER TABLE notification_settings ADD COLUMN ai_api_key TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_model",
        "ALTER TABLE notification_settings ADD COLUMN ai_model TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_system_prompt",
        "ALTER TABLE notification_settings ADD COLUMN ai_system_prompt TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_backup_base_url",
        "ALTER TABLE notification_settings ADD COLUMN ai_backup_base_url TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_backup_api_key",
        "ALTER TABLE notification_settings ADD COLUMN ai_backup_api_key TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_backup_model",
        "ALTER TABLE notification_settings ADD COLUMN ai_backup_model TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_backup_system_prompt",
        "ALTER TABLE notification_settings ADD COLUMN ai_backup_system_prompt TEXT NOT NULL DEFAULT ''",
    ),
    (
        "ai_retry_attempts",
        "ALTER TABLE notification_settings ADD COLUMN ai_retry_attempts INTEGER NOT NULL DEFAULT 3",
    ),
    (
        "sync_to_tracking_folder",
        "ALTER TABLE notification_settings ADD COLUMN sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0",
    ),
];

const AUTH_TABLES_SQL: &str = "
    CREATE TABLE IF NOT EXISTS users (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        password_hash TEXT    NOT NULL,
        salt          TEXT    NOT NULL,
        is_admin      INTEGER NOT NULL DEFAULT 0,
        created_at    REAL    NOT NULL,
        updated_at    REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS access_tokens (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        token_hash  TEXT    NOT NULL UNIQUE,
        name        TEXT    NOT NULL DEFAULT '',
        expires_at  REAL    NOT NULL,
        created_at  REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS cnki_sessions (
        user_id          INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
        session_json     TEXT    NOT NULL DEFAULT '{}',
        qr_uuid          TEXT    NOT NULL DEFAULT '',
        status           TEXT    NOT NULL DEFAULT 'empty',
        token_expires_at REAL,
        created_at       REAL    NOT NULL,
        updated_at       REAL    NOT NULL,
        last_used_at     REAL
    );

    CREATE TABLE IF NOT EXISTS folders (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        name        TEXT    NOT NULL,
        is_tracking INTEGER NOT NULL DEFAULT 0,
        created_at  REAL    NOT NULL,
        updated_at  REAL    NOT NULL,
        UNIQUE(user_id, name)
    );

    CREATE TABLE IF NOT EXISTS favorites (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        folder_id   INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        article_id  INTEGER NOT NULL,
        db_name     TEXT    NOT NULL DEFAULT '',
        note        TEXT    NOT NULL DEFAULT '',
        created_at  REAL    NOT NULL,
        UNIQUE(user_id, folder_id, article_id, db_name)
    );

    CREATE TABLE IF NOT EXISTS invite_codes (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        code        TEXT    NOT NULL UNIQUE,
        created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_at     REAL,
        created_at  REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS notification_settings (
        id                      INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id                 INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
        keywords                TEXT    NOT NULL DEFAULT '[]',
        directions              TEXT    NOT NULL DEFAULT '[]',
        selected_databases      TEXT    NOT NULL DEFAULT '[]',
        delivery_method         TEXT    NOT NULL DEFAULT 'folder',
        pushplus_token          TEXT    NOT NULL DEFAULT '',
        pushplus_template       TEXT    NOT NULL DEFAULT 'markdown',
        pushplus_topic          TEXT    NOT NULL DEFAULT '',
        pushplus_channel        TEXT    NOT NULL DEFAULT 'wechat',
        sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0,
        ai_base_url             TEXT    NOT NULL DEFAULT '',
        ai_api_key              TEXT    NOT NULL DEFAULT '',
        ai_model                TEXT    NOT NULL DEFAULT '',
        ai_system_prompt        TEXT    NOT NULL DEFAULT '',
        ai_backup_base_url      TEXT    NOT NULL DEFAULT '',
        ai_backup_api_key       TEXT    NOT NULL DEFAULT '',
        ai_backup_model         TEXT    NOT NULL DEFAULT '',
        ai_backup_system_prompt TEXT    NOT NULL DEFAULT '',
        ai_retry_attempts       INTEGER NOT NULL DEFAULT 3,
        enabled                 INTEGER NOT NULL DEFAULT 1,
        created_at              REAL    NOT NULL,
        updated_at              REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS scheduled_tasks (
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

    CREATE TABLE IF NOT EXISTS runtime_settings (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL DEFAULT '',
        updated_at REAL NOT NULL
    );

    CREATE TABLE IF NOT EXISTS announcements (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        title      TEXT    NOT NULL,
        message    TEXT    NOT NULL,
        priority   TEXT    NOT NULL DEFAULT 'normal',
        enabled    INTEGER NOT NULL DEFAULT 1,
        created_at REAL    NOT NULL,
        updated_at REAL    NOT NULL
    );
";

const AUTH_INDEXES_SQL: &str = "
    CREATE INDEX IF NOT EXISTS idx_access_tokens_user ON access_tokens(user_id);
    CREATE INDEX IF NOT EXISTS idx_folders_user ON folders(user_id);
    CREATE INDEX IF NOT EXISTS idx_favorites_folder ON favorites(folder_id);
    CREATE INDEX IF NOT EXISTS idx_favorites_user ON favorites(user_id);
    CREATE INDEX IF NOT EXISTS idx_invite_codes_code ON invite_codes(code);
    CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by ON invite_codes(created_by);
    CREATE INDEX IF NOT EXISTS idx_notification_settings_user ON notification_settings(user_id);
    CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_enabled ON scheduled_tasks(enabled);
    CREATE INDEX IF NOT EXISTS idx_announcements_enabled ON announcements(enabled);
";

const INDEX_TABLES_SQL: &str = "
    CREATE TABLE IF NOT EXISTS journals (
        journal_id INTEGER PRIMARY KEY,
        library_id TEXT NOT NULL,
        platform_journal_id TEXT,
        title TEXT,
        issn TEXT,
        eissn TEXT,
        scimago_rank REAL,
        cover_url TEXT,
        available INTEGER,
        toc_data_approved_and_live INTEGER,
        has_articles INTEGER
    );

    CREATE TABLE IF NOT EXISTS journal_meta (
        journal_id INTEGER PRIMARY KEY,
        source_csv TEXT NOT NULL,
        area TEXT,
        csv_title TEXT,
        csv_issn TEXT,
        csv_library TEXT,
        resolved_source TEXT,
        resolved_source_id TEXT,
        resolved_title TEXT,
        resolved_issn TEXT,
        resolved_eissn TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS issues (
        issue_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        publication_year INTEGER,
        title TEXT,
        volume TEXT,
        number TEXT,
        date TEXT,
        is_valid_issue INTEGER,
        suppressed INTEGER,
        embargoed INTEGER,
        within_subscription INTEGER,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS articles (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        title TEXT,
        date TEXT,
        authors TEXT,
        start_page TEXT,
        end_page TEXT,
        abstract TEXT,
        doi TEXT,
        pmid TEXT,
        permalink TEXT,
        suppressed INTEGER,
        in_press INTEGER,
        open_access INTEGER,
        platform_id TEXT,
        retraction_doi TEXT,
        within_library_holdings INTEGER,
        content_location TEXT,
        full_text_file TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE TABLE IF NOT EXISTS article_listing (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        publication_year INTEGER,
        date TEXT,
        open_access INTEGER,
        in_press INTEGER,
        suppressed INTEGER,
        within_library_holdings INTEGER,
        doi TEXT,
        pmid TEXT,
        area TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE TABLE IF NOT EXISTS listing_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        status TEXT NOT NULL,
        updated_at TEXT
    );

    CREATE TABLE IF NOT EXISTS journal_year_state (
        journal_id INTEGER NOT NULL,
        year INTEGER NOT NULL,
        status TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        PRIMARY KEY (journal_id, year)
    );

    CREATE TABLE IF NOT EXISTS journal_state (
        journal_id INTEGER PRIMARY KEY,
        status TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS index_runs (
        run_id TEXT PRIMARY KEY,
        csv_file TEXT NOT NULL,
        started_at TEXT NOT NULL,
        finished_at TEXT,
        status TEXT NOT NULL,
        total_journals INTEGER NOT NULL,
        succeeded_journals INTEGER NOT NULL,
        failed_journals INTEGER NOT NULL,
        resumed_journals INTEGER NOT NULL,
        error_summary TEXT
    );

    CREATE TABLE IF NOT EXISTS index_path_stats (
        run_id TEXT NOT NULL,
        source TEXT NOT NULL,
        path TEXT NOT NULL,
        journal_id INTEGER,
        journal_title TEXT,
        status TEXT NOT NULL,
        started_at TEXT NOT NULL,
        finished_at TEXT,
        works_count INTEGER NOT NULL,
        issues_count INTEGER NOT NULL,
        article_summaries_count INTEGER NOT NULL,
        article_details_count INTEGER NOT NULL,
        articles_written_count INTEGER NOT NULL,
        articles_deleted_no_authors_count INTEGER NOT NULL,
        error_type TEXT,
        error_message TEXT,
        FOREIGN KEY (run_id) REFERENCES index_runs(run_id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS index_api_call_stats (
        run_id TEXT NOT NULL,
        source TEXT NOT NULL,
        service TEXT NOT NULL,
        endpoint TEXT NOT NULL,
        method TEXT NOT NULL,
        url_path TEXT NOT NULL,
        journal_id INTEGER,
        journal_title TEXT,
        logical_calls INTEGER NOT NULL,
        attempts INTEGER NOT NULL,
        successes INTEGER NOT NULL,
        failures INTEGER NOT NULL,
        retry_count INTEGER NOT NULL,
        status_codes_json TEXT NOT NULL,
        transport_errors INTEGER NOT NULL,
        rate_limit_failures INTEGER NOT NULL,
        total_latency_ms INTEGER NOT NULL,
        error_samples_json TEXT NOT NULL,
        FOREIGN KEY (run_id) REFERENCES index_runs(run_id) ON DELETE CASCADE
    );
";

const INDEX_CHANGE_EVENTS_SQL: &str = "
    CREATE TABLE IF NOT EXISTS index_change_events (
        event_id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL,
        worker_id TEXT NOT NULL DEFAULT '',
        article_id INTEGER NOT NULL,
        event_type TEXT NOT NULL CHECK (event_type IN ('add', 'remove')),
        membership_type TEXT NOT NULL
            CHECK (membership_type IN ('issue', 'inpress')),
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        is_backfill INTEGER NOT NULL DEFAULT 0 CHECK (is_backfill IN (0, 1)),
        created_at TEXT NOT NULL,
        CHECK (
            (membership_type = 'issue' AND issue_id IS NOT NULL)
            OR (membership_type = 'inpress' AND issue_id IS NULL)
        )
    );

    CREATE UNIQUE INDEX IF NOT EXISTS idx_index_change_events_identity
        ON index_change_events(
            run_id, article_id, event_type, membership_type, journal_id,
            COALESCE(issue_id, -1)
        );
    CREATE INDEX IF NOT EXISTS idx_index_change_events_run_order
        ON index_change_events(run_id, event_id);
    CREATE INDEX IF NOT EXISTS idx_index_change_events_run_membership
        ON index_change_events(
            run_id, membership_type, journal_id, issue_id, event_id
        );
    CREATE INDEX IF NOT EXISTS idx_index_change_events_run_article
        ON index_change_events(run_id, article_id, event_id);
";

const INDEX_RUN_LEASE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS index_run_lease (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        run_id TEXT NOT NULL,
        heartbeat_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        FOREIGN KEY (run_id) REFERENCES index_runs(run_id) ON DELETE CASCADE
    );
";

const INDEX_INDEXES_SQL: &str = "
    CREATE INDEX IF NOT EXISTS idx_journals_issn ON journals(issn);
    CREATE INDEX IF NOT EXISTS idx_journals_library_id ON journals(library_id);
    CREATE INDEX IF NOT EXISTS idx_journals_available ON journals(available);
    CREATE INDEX IF NOT EXISTS idx_journals_has_articles ON journals(has_articles);
    CREATE INDEX IF NOT EXISTS idx_journals_scimago_rank ON journals(scimago_rank);
    CREATE INDEX IF NOT EXISTS idx_journal_meta_area ON journal_meta(area);
    CREATE INDEX IF NOT EXISTS idx_journal_meta_area_journal ON journal_meta(area, journal_id);
    CREATE INDEX IF NOT EXISTS idx_issues_journal_year ON issues(journal_id, publication_year);
    CREATE INDEX IF NOT EXISTS idx_issues_publication_year ON issues(publication_year);
    CREATE INDEX IF NOT EXISTS idx_articles_journal ON articles(journal_id);
    CREATE INDEX IF NOT EXISTS idx_articles_issue ON articles(issue_id);
    CREATE INDEX IF NOT EXISTS idx_articles_date ON articles(date);
    CREATE INDEX IF NOT EXISTS idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_journal_date_id ON articles(journal_id, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_issue_date_id ON articles(issue_id, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_open_access ON articles(open_access);
    CREATE INDEX IF NOT EXISTS idx_articles_open_access_date_id ON articles(open_access, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_in_press ON articles(in_press);
    CREATE INDEX IF NOT EXISTS idx_articles_in_press_date_id ON articles(in_press, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_suppressed ON articles(suppressed);
    CREATE INDEX IF NOT EXISTS idx_articles_suppressed_date_id ON articles(suppressed, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_within_holdings ON articles(within_library_holdings);
    CREATE INDEX IF NOT EXISTS idx_articles_within_holdings_date_id ON articles(within_library_holdings, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_articles_doi ON articles(doi);
    CREATE INDEX IF NOT EXISTS idx_articles_pmid ON articles(pmid);
    CREATE INDEX IF NOT EXISTS idx_article_listing_date_id ON article_listing(date, article_id);
    CREATE INDEX IF NOT EXISTS idx_article_listing_area ON article_listing(area);
    CREATE INDEX IF NOT EXISTS idx_article_listing_area_date_id ON article_listing(area, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_article_listing_publication_year ON article_listing(publication_year);
    CREATE INDEX IF NOT EXISTS idx_article_listing_journal ON article_listing(journal_id);
    CREATE INDEX IF NOT EXISTS idx_article_listing_journal_date_id ON article_listing(journal_id, date, article_id);
    CREATE INDEX IF NOT EXISTS idx_article_listing_issue ON article_listing(issue_id);
    CREATE INDEX IF NOT EXISTS idx_index_api_call_stats_run ON index_api_call_stats(run_id);
";

#[cfg(test)]
/// Shared structured-log capture helpers for storage module tests.
pub(crate) mod test_support {
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, Once, OnceLock};

    use serde_json::Value;
    use tracing_subscriber::fmt::MakeWriter;

    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());
    static CAPTURE_BYTES: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    static CAPTURE_SUBSCRIBER: Once = Once::new();
    static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

    /// Thread-safe byte buffer used as a tracing test writer.
    #[derive(Clone)]
    pub(crate) struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
        capture_id: u64,
    }

    impl Default for CapturedLogs {
        fn default() -> Self {
            let bytes = Arc::clone(CAPTURE_BYTES.get_or_init(|| Arc::new(Mutex::new(Vec::new()))));
            CAPTURE_SUBSCRIBER.call_once(|| {
                let subscriber = tracing_subscriber::fmt()
                    .with_ansi(false)
                    .with_max_level(tracing::Level::TRACE)
                    .with_writer(CapturedSink {
                        bytes: Arc::clone(&bytes),
                    })
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .finish();
                tracing::subscriber::set_global_default(subscriber)
                    .expect("storage tests should install one global tracing subscriber");
            });
            Self {
                bytes,
                capture_id: NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed),
            }
        }
    }

    impl CapturedLogs {
        /// Run an operation inside a uniquely identifiable capture span.
        ///
        /// # Arguments
        ///
        /// * `operation` - Operation whose structured events should be captured.
        ///
        /// # Returns
        ///
        /// Operation result after synchronous event capture.
        pub(crate) fn capture<T>(&self, operation: impl FnOnce() -> T) -> T {
            let _capture_guard = CAPTURE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let capture_span = tracing::info_span!(
                "test.capture",
                component = "test",
                capture_id = self.capture_id,
            );
            capture_span.in_scope(operation)
        }

        /// Return all captured bytes as UTF-8 text.
        ///
        /// # Returns
        ///
        /// Captured JSON Lines text.
        pub(crate) fn text(&self) -> String {
            self.events()
                .into_iter()
                .map(|event| serde_json::to_string(&event).expect("event should serialize"))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// Parse captured JSON Lines into event values.
        ///
        /// # Returns
        ///
        /// Parsed event objects in emission order.
        pub(crate) fn events(&self) -> Vec<Value> {
            let text = String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured logs should be UTF-8");
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_str(line).expect("captured log should be JSON"))
                .filter(|event: &Value| {
                    event["spans"].as_array().is_some_and(|spans| {
                        spans
                            .iter()
                            .any(|span| span["capture_id"].as_u64() == Some(self.capture_id))
                    })
                })
                .collect()
        }
    }

    #[derive(Clone)]
    struct CapturedSink {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    struct CapturedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedSink {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::test_support::CapturedLogs;
    use super::{migrate_auth_database, MigrationError, AUTH_SCHEMA_VERSION};

    #[test]
    fn migration_events_report_versions_without_database_paths() {
        const PATH_SENTINEL: &str = "migration-path-sentinel-never-log";

        let root = tempdir().expect("temporary root should be created");
        let success_path = root.path().join(PATH_SENTINEL).join("auth.sqlite");
        let success_logs = CapturedLogs::default();
        success_logs
            .capture(|| migrate_auth_database(&success_path))
            .expect("auth migration should complete");

        let success_events = success_logs.events();
        let completed = success_events
            .iter()
            .find(|event| event["event"] == "storage.migration.completed")
            .expect("migration completion event should be captured");
        assert_eq!(completed["database_kind"], "auth");
        assert_eq!(completed["target_version"], AUTH_SCHEMA_VERSION);
        assert_eq!(completed["from_version"], 0);
        assert_eq!(completed["to_version"], AUTH_SCHEMA_VERSION);
        assert_eq!(completed["applied_count"], AUTH_SCHEMA_VERSION);
        assert!(!success_logs.text().contains(PATH_SENTINEL));

        let unsupported_path = root.path().join(PATH_SENTINEL).join("newer.sqlite");
        let connection =
            Connection::open(&unsupported_path).expect("unsupported-version fixture should open");
        connection
            .pragma_update(None, "user_version", AUTH_SCHEMA_VERSION + 1)
            .expect("unsupported version should write");
        drop(connection);
        let failure_logs = CapturedLogs::default();
        let error = failure_logs
            .capture(|| migrate_auth_database(&unsupported_path))
            .expect_err("newer auth schema should be rejected");
        assert!(matches!(
            error,
            MigrationError::UnsupportedSchemaVersion { .. }
        ));

        let failure_events = failure_logs.events();
        let failed = failure_events
            .iter()
            .find(|event| event["event"] == "storage.migration.failed")
            .expect("migration failure event should be captured");
        assert_eq!(failed["database_kind"], "auth");
        assert_eq!(failed["target_version"], AUTH_SCHEMA_VERSION);
        assert_eq!(failed["database_version"], AUTH_SCHEMA_VERSION + 1);
        assert_eq!(failed["error_kind"], "unsupported_schema_version");
        assert!(!failure_logs.text().contains(PATH_SENTINEL));
    }
}
