//! Ordered, transactional migrations for auth and index SQLite databases.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::{try_load_extension, DatabaseResolutionError, StorageConfig};

/// Current auth and business database schema version.
pub const AUTH_SCHEMA_VERSION: i64 = 2;

/// Current index database schema version.
pub const INDEX_SCHEMA_VERSION: i64 = 1;

const AUTH_DATABASE: &str = "auth";
const INDEX_DATABASE: &str = "index";
const BUSY_TIMEOUT_SECONDS: u64 = 30;

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
    let tokenizer_path = config.simple_tokenizer_path();
    for path in config.list_index_databases()? {
        migrate_index_database(path, tokenizer_path.as_deref())?;
    }
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
    let connection = open_migration_connection(path.as_ref())?;
    let mut version = schema_version(&connection)?;
    reject_newer_version(AUTH_DATABASE, version, AUTH_SCHEMA_VERSION)?;
    if version == AUTH_SCHEMA_VERSION {
        return Ok(());
    }
    configure_writable_connection(&connection)?;

    while version < AUTH_SCHEMA_VERSION {
        let next_version = version + 1;
        let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
        match next_version {
            1 => apply_auth_version_one(&transaction)?,
            2 => apply_auth_version_two(&transaction)?,
            _ => unreachable!("auth migration version should be implemented"),
        }
        transaction.pragma_update(None, "user_version", next_version)?;
        transaction.commit()?;
        version = next_version;
    }
    Ok(())
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
    let connection = open_migration_connection(path.as_ref())?;
    let mut version = schema_version(&connection)?;
    reject_newer_version(INDEX_DATABASE, version, INDEX_SCHEMA_VERSION)?;
    if version == INDEX_SCHEMA_VERSION {
        return Ok(());
    }
    configure_writable_connection(&connection)?;
    let is_simple_tokenizer_loaded = try_load_extension(&connection, simple_tokenizer_path)?;

    while version < INDEX_SCHEMA_VERSION {
        let next_version = version + 1;
        let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
        match next_version {
            1 => apply_index_version_one(&transaction, is_simple_tokenizer_loaded)?,
            _ => unreachable!("index migration version should be implemented"),
        }
        transaction.pragma_update(None, "user_version", next_version)?;
        transaction.commit()?;
        version = next_version;
    }
    Ok(())
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

fn apply_index_version_one(
    transaction: &Transaction<'_>,
    is_simple_tokenizer_loaded: bool,
) -> rusqlite::Result<()> {
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

    let tokenizer_sql = if is_simple_tokenizer_loaded {
        ", tokenize = 'simple'"
    } else {
        ""
    };
    transaction.execute_batch(&format!(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS article_search
        USING fts5(
            article_id UNINDEXED,
            title,
            abstract,
            doi,
            authors,
            journal_title
            {tokenizer_sql}
        );
        "
    ))?;
    transaction.execute_batch(INDEX_INDEXES_SQL)
}

fn table_columns(connection: &Connection, table_name: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

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
