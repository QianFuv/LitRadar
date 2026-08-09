//! Ordered, transactional migrations for auth and index SQLite databases.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};

use litradar_domain::{normalize_contract_issn, ProviderOrderConfiguration};

use crate::business::{import_legacy_delivery_state_files, DeliveryRepositoryError};
use crate::{DatabaseResolutionError, StorageConfig};

/// Current auth and business database schema version.
pub const AUTH_SCHEMA_VERSION: i64 = 15;

/// Current index database schema version.
pub const INDEX_SCHEMA_VERSION: i64 = 6;

const AUTH_DATABASE: &str = "auth";
const INDEX_DATABASE: &str = "index";
const BUSY_TIMEOUT_SECONDS: u64 = 30;

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
    /// A legacy or non-empty unversioned index must be rebuilt explicitly.
    IndexRebuildRequired {
        /// Exact legacy index path that was inspected read-only.
        path: PathBuf,
        /// Existing SQLite user version.
        found: i64,
        /// Provider-neutral content version required by this binary.
        required: i64,
    },
    /// Existing journal identity values are malformed and cannot be migrated safely.
    InvalidIndexIdentityState,
    /// Two existing journals claim the same canonical identity key.
    IndexIdentityConflict,
    /// Legacy Provider order settings cannot be migrated without changing their meaning.
    InvalidRuntimeProviderOrderState,
    /// Notification list JSON cannot be migrated without changing its meaning.
    InvalidNotificationSettingsState,
    /// Legacy mutable delivery state could not be imported safely.
    DeliveryState(DeliveryRepositoryError),
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
            Self::IndexRebuildRequired {
                path,
                found,
                required,
            } => write!(
                formatter,
                "index database {} uses legacy schema version {found}; move or delete that exact file and rebuild it as content schema v{required}",
                path.display()
            ),
            Self::InvalidIndexIdentityState => {
                write!(formatter, "index journal identity state is invalid for migration")
            }
            Self::IndexIdentityConflict => write!(
                formatter,
                "index journal identity ownership conflicts across legacy journal rows"
            ),
            Self::InvalidRuntimeProviderOrderState => {
                formatter.write_str("legacy runtime Provider order state is invalid for migration")
            }
            Self::InvalidNotificationSettingsState => {
                formatter.write_str("notification settings list state is invalid for migration")
            }
            Self::DeliveryState(error) => write!(formatter, "{error}"),
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
            Self::DeliveryState(error) => Some(error),
            Self::UnsupportedSchemaVersion { .. }
            | Self::IndexRebuildRequired { .. }
            | Self::InvalidIndexIdentityState
            | Self::IndexIdentityConflict
            | Self::InvalidRuntimeProviderOrderState
            | Self::InvalidNotificationSettingsState => None,
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

impl From<DeliveryRepositoryError> for MigrationError {
    /// Convert durable delivery import errors into startup migration errors.
    fn from(error: DeliveryRepositoryError) -> Self {
        Self::DeliveryState(error)
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    import_legacy_delivery_state_files(config, now)?;
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
            7 => apply_auth_version_seven(&transaction)?,
            8 => apply_auth_version_eight(&transaction, from_version)?,
            9 => apply_auth_version_nine(&transaction)?,
            10 => apply_auth_version_ten(&transaction)?,
            11 => apply_auth_version_eleven(&transaction)?,
            12 => apply_auth_version_twelve(&transaction)?,
            13 => apply_auth_version_thirteen(&transaction)?,
            14 => apply_auth_version_fourteen(&transaction)?,
            15 => apply_auth_version_fifteen(&transaction)?,
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
    _simple_tokenizer_path: Option<&Path>,
) -> Result<MigrationSummary, MigrationError> {
    let inspection = inspect_existing_index_database(path)?;
    if let Some((version, object_count)) = inspection {
        reject_newer_version(INDEX_DATABASE, version, INDEX_SCHEMA_VERSION)?;
        if version == INDEX_SCHEMA_VERSION {
            let connection = open_read_only_index_connection(path)?;
            validate_index_v6_schema(&connection)?;
            return Ok(MigrationSummary {
                from_version: version,
                to_version: version,
            });
        }
        if version == 4 || version == 5 {
            {
                let connection = open_read_only_index_connection(path)?;
                if version == 4 {
                    validate_index_v4_schema(&connection)?;
                } else {
                    validate_index_v5_schema(&connection)?;
                }
            }
            let connection = open_migration_connection(path)?;
            configure_writable_connection(&connection)?;
            connection.pragma_update(None, "foreign_keys", false)?;
            let transaction =
                Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
            if version == 4 {
                apply_index_version_five(&transaction)?;
            }
            apply_index_version_six(&transaction)?;
            transaction.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;
            transaction.commit()?;
            connection.pragma_update(None, "foreign_keys", true)?;
            validate_index_v6_schema(&connection)?;
            return Ok(MigrationSummary {
                from_version: version,
                to_version: INDEX_SCHEMA_VERSION,
            });
        }
        if version != 0 || object_count != 0 {
            return Err(MigrationError::IndexRebuildRequired {
                path: path.to_path_buf(),
                found: version,
                required: INDEX_SCHEMA_VERSION,
            });
        }
    }

    let connection = open_migration_connection(path)?;
    let version = schema_version(&connection)?;
    let from_version = version;
    configure_writable_connection(&connection)?;
    let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
    transaction.execute_batch(INDEX_CONTENT_TABLES_SQL)?;
    transaction.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;
    transaction.commit()?;
    validate_index_v6_schema(&connection)?;
    Ok(MigrationSummary {
        from_version,
        to_version: INDEX_SCHEMA_VERSION,
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
        MigrationError::IndexRebuildRequired { .. } => "index_rebuild_required",
        MigrationError::InvalidIndexIdentityState => "invalid_index_identity_state",
        MigrationError::IndexIdentityConflict => "index_identity_conflict",
        MigrationError::InvalidRuntimeProviderOrderState => "invalid_runtime_provider_order_state",
        MigrationError::InvalidNotificationSettingsState => "invalid_notification_settings_state",
        MigrationError::DeliveryState(_) => "delivery_state",
    }
}

fn migration_error_database_version(error: &MigrationError) -> i64 {
    match error {
        MigrationError::UnsupportedSchemaVersion { found, .. }
        | MigrationError::IndexRebuildRequired { found, .. } => *found,
        MigrationError::Io(_)
        | MigrationError::Sqlite(_)
        | MigrationError::DatabaseResolution(_)
        | MigrationError::DeliveryState(_) => -1,
        MigrationError::InvalidIndexIdentityState | MigrationError::IndexIdentityConflict => 4,
        MigrationError::InvalidRuntimeProviderOrderState => 6,
        MigrationError::InvalidNotificationSettingsState => 13,
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

fn inspect_existing_index_database(path: &Path) -> Result<Option<(i64, i64)>, MigrationError> {
    if !path.exists() || fs::metadata(path)?.len() == 0 {
        return Ok(None);
    }
    let connection = open_read_only_index_connection(path)?;
    let version = schema_version(&connection)?;
    let object_count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(Some((version, object_count)))
}

fn open_read_only_index_connection(path: &Path) -> Result<Connection, MigrationError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(BUSY_TIMEOUT_SECONDS))?;
    Ok(connection)
}

fn validate_index_v4_schema(connection: &Connection) -> Result<(), MigrationError> {
    validate_index_schema(connection, false, false)
}

fn validate_index_v5_schema(connection: &Connection) -> Result<(), MigrationError> {
    validate_index_schema(connection, true, false)
}

fn validate_index_v6_schema(connection: &Connection) -> Result<(), MigrationError> {
    validate_index_schema(connection, true, true)
}

fn validate_index_schema(
    connection: &Connection,
    has_journal_identity_keys: bool,
    has_retraction_dois: bool,
) -> Result<(), MigrationError> {
    let mut expected = [
        "article_change_events",
        "article_identity_keys",
        "article_listing",
        "article_search",
        "articles",
        "issues",
        "journals",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    if has_journal_identity_keys {
        expected.insert("journal_identity_keys".to_string());
    }
    if has_retraction_dois {
        expected.insert("article_retraction_dois".to_string());
    }
    let mut statement = connection.prepare(
        "SELECT name
         FROM sqlite_schema
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND name NOT LIKE 'article_search_%'
         ORDER BY name",
    )?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if actual != expected {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    let expected_columns: &[(&str, &[&str])] = &[
        (
            "journals",
            &[
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
            ],
        ),
        (
            "issues",
            &[
                "issue_id",
                "journal_id",
                "publication_year",
                "title",
                "volume",
                "number",
                "date",
            ],
        ),
        (
            "article_identity_keys",
            &["identity_kind", "identity_value", "article_id"],
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
                "abstract_text",
                "doi",
                "pmid",
                "authors",
                "journal_title",
            ],
        ),
        (
            "article_change_events",
            &[
                "event_id",
                "content_revision",
                "article_id",
                "change_kind",
                "journal_id",
                "issue_id",
                "in_press",
                "created_at",
            ],
        ),
    ];
    for (table_name, expected) in expected_columns {
        if table_columns(connection, table_name)? != *expected {
            return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
        }
    }
    let expected_article_columns = if has_retraction_dois {
        &[
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
        ][..]
    } else {
        &[
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
            "retraction_doi",
        ][..]
    };
    if table_columns(connection, "articles")? != expected_article_columns {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    if has_journal_identity_keys
        && table_columns(connection, "journal_identity_keys")?
            != ["identity_kind", "identity_value", "canonical_catalog_id"]
    {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    if has_retraction_dois
        && table_columns(connection, "article_retraction_dois")? != ["article_id", "retraction_doi"]
    {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    let mut expected_indexes = [
        "idx_article_change_events_order",
        "idx_article_change_events_revision",
        "idx_article_identity_keys_article",
        "idx_article_listing_date_id",
        "idx_article_listing_issue",
        "idx_article_listing_journal_date_id",
        "idx_articles_date_id",
        "idx_articles_doi",
        "idx_articles_issue",
        "idx_articles_journal",
        "idx_articles_pmid",
        "idx_issues_journal_year",
        "idx_journals_eissn",
        "idx_journals_issn",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    if has_journal_identity_keys {
        expected_indexes.insert("idx_journal_identity_keys_catalog".to_string());
    }
    if has_retraction_dois {
        expected_indexes.insert("idx_article_retraction_dois_doi".to_string());
    }
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let actual_indexes = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if actual_indexes != expected_indexes {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    if has_retraction_dois {
        let foreign_key_violation_count =
            connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })?;
        if foreign_key_violation_count != 0 {
            return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
        }
    }
    Ok(())
}

fn apply_index_version_five(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    transaction.execute_batch(INDEX_VERSION_FIVE_SQL)?;
    let journals = {
        let mut statement = transaction.prepare(
            "SELECT catalog_id, issns_json, issn, eissn FROM journals ORDER BY catalog_id",
        )?;
        let journals = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        journals
    };
    let mut owners = BTreeMap::new();
    for (catalog_id, issns_json, issn, eissn) in journals {
        if !is_canonical_catalog_id(&catalog_id) {
            return Err(MigrationError::InvalidIndexIdentityState);
        }
        register_index_identity_owner(&mut owners, "catalog_id", catalog_id.clone(), &catalog_id)?;
        let mut issns = serde_json::from_str::<Vec<String>>(&issns_json)
            .map_err(|_| MigrationError::InvalidIndexIdentityState)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        issns.extend([issn, eissn].into_iter().flatten());
        for issn in issns {
            let normalized = normalize_contract_issn(&issn)
                .filter(|normalized| normalized == &issn)
                .ok_or(MigrationError::InvalidIndexIdentityState)?;
            register_index_identity_owner(&mut owners, "issn", normalized, &catalog_id)?;
        }
    }
    let mut statement = transaction.prepare(
        "INSERT INTO journal_identity_keys (
             identity_kind, identity_value, canonical_catalog_id
         ) VALUES (?1, ?2, ?3)",
    )?;
    for ((identity_kind, identity_value), canonical_catalog_id) in owners {
        statement.execute((identity_kind, identity_value, canonical_catalog_id))?;
    }
    Ok(())
}

fn apply_index_version_six(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    transaction.execute_batch(INDEX_VERSION_SIX_SQL)?;
    let foreign_key_violation_count =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })?;
    if foreign_key_violation_count != 0 {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    Ok(())
}

fn register_index_identity_owner(
    owners: &mut BTreeMap<(String, String), String>,
    identity_kind: &str,
    identity_value: String,
    canonical_catalog_id: &str,
) -> Result<(), MigrationError> {
    let key = (identity_kind.to_string(), identity_value);
    if let Some(owner) = owners.get(&key) {
        if owner != canonical_catalog_id {
            return Err(MigrationError::IndexIdentityConflict);
        }
        return Ok(());
    }
    owners.insert(key, canonical_catalog_id.to_string());
    Ok(())
}

fn is_canonical_catalog_id(catalog_id: &str) -> bool {
    (3..=128).contains(&catalog_id.len())
        && catalog_id.is_ascii()
        && catalog_id
            .bytes()
            .enumerate()
            .all(|(index, byte)| match byte {
                b'a'..=b'z' | b'0'..=b'9' => true,
                b'.' | b'_' | b'-' => index > 0,
                _ => false,
            })
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

fn apply_auth_version_seven(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS runtime_settings (
             key        TEXT PRIMARY KEY,
             value      TEXT NOT NULL DEFAULT '',
             updated_at REAL NOT NULL
         );",
    )?;
    let detail = legacy_provider_order_row(transaction, "article_detail_provider_order")?;
    let abstract_page = legacy_provider_order_row(transaction, "article_abstract_provider_order")?;
    let full_text = legacy_provider_order_row(transaction, "article_fulltext_provider_order")?;

    if let Some((providers, updated_at)) = abstract_page.or(detail) {
        upsert_provider_order_configuration(
            transaction,
            "article_abstract_provider_orders",
            &providers,
            updated_at,
        )?;
    }
    if let Some((providers, updated_at)) = full_text {
        upsert_provider_order_configuration(
            transaction,
            "article_fulltext_provider_orders",
            &providers,
            updated_at,
        )?;
    }
    transaction.execute(
        "DELETE FROM runtime_settings
         WHERE key IN (
             'article_detail_provider_order',
             'article_abstract_provider_order',
             'article_fulltext_provider_order'
         )",
        [],
    )?;
    Ok(())
}

fn apply_auth_version_eight(
    transaction: &Transaction<'_>,
    from_version: i64,
) -> Result<(), MigrationError> {
    if (1..=7).contains(&from_version) {
        let updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        for (key, value) in [
            (
                "index_provider_routes",
                r#"{"ccf_computer_journals":"scholarly","chinese_journals":"cnki","english_journals":"scholarly"}"#,
            ),
            (
                "article_abstract_provider_orders",
                r#"{"default":["scholarly","cnki"],"catalogs":{}}"#,
            ),
            (
                "article_fulltext_provider_orders",
                r#"{"default":["zjlib_cnki"],"catalogs":{}}"#,
            ),
        ] {
            transaction.execute(
                "INSERT OR IGNORE INTO runtime_settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)",
                params![key, value, updated_at],
            )?;
        }
    }
    rewrite_runtime_provider_name_tokens(transaction)?;
    Ok(())
}

fn apply_auth_version_nine(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    transaction.execute_batch(
        "CREATE TABLE security_audit_events (
             id                  INTEGER PRIMARY KEY AUTOINCREMENT,
             actor_id            INTEGER CHECK (actor_id IS NULL OR actor_id > 0),
             target_id           INTEGER CHECK (target_id IS NULL OR target_id > 0),
             action              TEXT NOT NULL CHECK (
                 length(action) BETWEEN 1 AND 64 AND action NOT GLOB '*[^a-z0-9_]*'
             ),
             outcome             TEXT NOT NULL CHECK (
                 length(outcome) BETWEEN 1 AND 64 AND outcome NOT GLOB '*[^a-z0-9_]*'
             ),
             reason              TEXT NOT NULL DEFAULT '' CHECK (
                 length(reason) <= 64 AND reason NOT GLOB '*[^a-z0-9_]*'
             ),
             request_id          TEXT NOT NULL DEFAULT '' CHECK (
                 length(request_id) <= 128 AND request_id NOT GLOB '*[^A-Za-z0-9_.:-]*'
             ),
             source_class        TEXT NOT NULL DEFAULT '' CHECK (
                 length(source_class) <= 64 AND source_class NOT GLOB '*[^a-z0-9_]*'
             ),
             bucket              TEXT NOT NULL DEFAULT '' CHECK (
                 length(bucket) <= 64 AND bucket NOT GLOB '*[^a-z0-9_]*'
             ),
             rejected_count      INTEGER NOT NULL DEFAULT 0 CHECK (rejected_count >= 0),
             retry_after_seconds INTEGER NOT NULL DEFAULT 0 CHECK (retry_after_seconds >= 0),
             occurred_at         REAL NOT NULL
         );

         CREATE INDEX idx_security_audit_events_occurred
             ON security_audit_events(occurred_at, id);
         CREATE INDEX idx_security_audit_events_action_outcome
             ON security_audit_events(action, outcome, occurred_at DESC);
         CREATE INDEX idx_security_audit_events_actor
             ON security_audit_events(actor_id, occurred_at DESC);
         CREATE INDEX idx_security_audit_events_request
             ON security_audit_events(request_id) WHERE request_id <> '';

         CREATE TRIGGER security_audit_events_no_update
         BEFORE UPDATE ON security_audit_events
         BEGIN
             SELECT RAISE(ABORT, 'security audit events are append-only');
         END;

         CREATE TABLE security_audit_maintenance (
             id                INTEGER PRIMARY KEY CHECK (id = 1),
             last_retention_at REAL
         );
         INSERT INTO security_audit_maintenance (id, last_retention_at) VALUES (1, NULL);",
    )?;
    Ok(())
}

fn apply_auth_version_ten(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    transaction.execute_batch(
        "CREATE TABLE delivery_checkpoints (
             id                    INTEGER PRIMARY KEY AUTOINCREMENT,
             workflow              TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             db_name               TEXT NOT NULL CHECK (length(db_name) BETWEEN 1 AND 255),
             status                TEXT NOT NULL DEFAULT 'idle' CHECK (
                 status IN ('idle', 'running', 'completed', 'failed', 'skipped', 'unknown')
             ),
             legacy_status         TEXT CHECK (
                 legacy_status IS NULL OR length(legacy_status) BETWEEN 1 AND 128
             ),
             snapshot_json         TEXT NOT NULL,
             last_completed_run_at TEXT,
             revision              INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             legacy_source_hash    TEXT CHECK (
                 legacy_source_hash IS NULL OR (
                     length(legacy_source_hash) = 64
                     AND legacy_source_hash NOT GLOB '*[^0-9a-f]*'
                 )
             ),
             legacy_source_name    TEXT CHECK (
                 legacy_source_name IS NULL OR length(legacy_source_name) BETWEEN 1 AND 255
             ),
             legacy_imported_at    REAL,
             created_at            REAL NOT NULL,
             updated_at            REAL NOT NULL,
             CHECK (
                 (legacy_source_hash IS NULL AND legacy_source_name IS NULL
                  AND legacy_imported_at IS NULL)
                 OR (legacy_source_hash IS NOT NULL AND legacy_source_name IS NOT NULL
                     AND legacy_imported_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX idx_delivery_checkpoints_scope
             ON delivery_checkpoints(workflow, db_name);

         CREATE TABLE delivery_runs (
             id                     INTEGER PRIMARY KEY AUTOINCREMENT,
             external_id            TEXT NOT NULL CHECK (length(external_id) BETWEEN 1 AND 128),
             workflow               TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             scope_key              TEXT NOT NULL CHECK (length(scope_key) BETWEEN 1 AND 255),
             db_name                TEXT CHECK (db_name IS NULL OR length(db_name) BETWEEN 1 AND 255),
             trigger_kind           TEXT NOT NULL CHECK (
                 trigger_kind IN ('scheduled', 'manual', 'legacy')
             ),
             mode                   TEXT NOT NULL CHECK (mode IN ('dry_run', 'execute')),
             user_id                INTEGER CHECK (user_id IS NULL OR user_id > 0),
             status                 TEXT NOT NULL CHECK (
                 status IN (
                     'queued', 'claimed', 'running', 'cancelling', 'completed', 'failed',
                     'cancelled', 'timed_out', 'skipped', 'unknown'
                 )
             ),
             legacy_status          TEXT CHECK (
                 legacy_status IS NULL OR length(legacy_status) BETWEEN 1 AND 128
             ),
             owner_id               TEXT CHECK (
                 owner_id IS NULL OR length(owner_id) BETWEEN 1 AND 128
             ),
             lease_expires_at       REAL,
             deadline_at            REAL,
             cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK (
                 cancellation_requested IN (0, 1)
             ),
             result_json            TEXT,
             error_code             TEXT CHECK (
                 error_code IS NULL OR length(error_code) BETWEEN 1 AND 64
             ),
             revision               INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             created_at             REAL NOT NULL,
             started_at             REAL,
             updated_at             REAL NOT NULL,
             finished_at            REAL,
             CHECK (
                 (status = 'queued' AND owner_id IS NULL AND lease_expires_at IS NULL
                  AND finished_at IS NULL)
                 OR (status IN ('claimed', 'running', 'cancelling')
                     AND owner_id IS NOT NULL AND lease_expires_at IS NOT NULL
                     AND finished_at IS NULL)
                 OR (status IN ('completed', 'failed', 'cancelled', 'timed_out', 'skipped', 'unknown')
                     AND owner_id IS NULL AND lease_expires_at IS NULL
                     AND finished_at IS NOT NULL)
             ),
             CHECK (
                 (trigger_kind = 'manual' AND user_id IS NOT NULL)
                 OR (trigger_kind <> 'manual' AND db_name IS NOT NULL)
             ),
             CHECK (deadline_at IS NULL OR deadline_at > created_at)
         );
         CREATE UNIQUE INDEX idx_delivery_runs_external_scope
             ON delivery_runs(workflow, scope_key, external_id);
         CREATE UNIQUE INDEX idx_delivery_runs_active_scope
             ON delivery_runs(workflow, db_name)
             WHERE db_name IS NOT NULL
               AND status IN ('claimed', 'running', 'cancelling');
         CREATE UNIQUE INDEX idx_delivery_runs_active_manual_user
             ON delivery_runs(user_id)
             WHERE trigger_kind = 'manual'
               AND user_id IS NOT NULL
               AND status IN ('queued', 'claimed', 'running', 'cancelling');
         CREATE INDEX idx_delivery_runs_queue
             ON delivery_runs(status, created_at, id);
         CREATE INDEX idx_delivery_runs_owner_lease
             ON delivery_runs(owner_id, lease_expires_at)
             WHERE owner_id IS NOT NULL;

         CREATE TABLE delivery_run_items (
             id               INTEGER PRIMARY KEY AUTOINCREMENT,
             delivery_run_id  INTEGER NOT NULL REFERENCES delivery_runs(id) ON DELETE CASCADE,
             item_kind        TEXT NOT NULL CHECK (
                 item_kind IN ('issue', 'inpress', 'article', 'subscriber')
             ),
             item_key         TEXT NOT NULL CHECK (length(item_key) BETWEEN 1 AND 512),
             user_id          INTEGER CHECK (user_id IS NULL OR user_id > 0),
             article_id       INTEGER CHECK (article_id IS NULL OR article_id > 0),
             status           TEXT NOT NULL CHECK (
                 status IN (
                     'pending', 'claimed', 'sending', 'succeeded', 'failed', 'skipped',
                     'cancelled', 'unknown'
                 )
             ),
             legacy_status    TEXT CHECK (
                 legacy_status IS NULL OR length(legacy_status) BETWEEN 1 AND 128
             ),
             owner_id         TEXT CHECK (
                 owner_id IS NULL OR length(owner_id) BETWEEN 1 AND 128
             ),
             lease_expires_at REAL,
             attempt_count    INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
             result_json      TEXT,
             error_code       TEXT CHECK (
                 error_code IS NULL OR length(error_code) BETWEEN 1 AND 64
             ),
             revision         INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             created_at       REAL NOT NULL,
             started_at       REAL,
             updated_at       REAL NOT NULL,
             finished_at      REAL,
             UNIQUE(delivery_run_id, item_kind, item_key),
             CHECK (
                 (status = 'pending' AND owner_id IS NULL AND lease_expires_at IS NULL
                  AND finished_at IS NULL)
                 OR (status IN ('claimed', 'sending')
                     AND owner_id IS NOT NULL AND lease_expires_at IS NOT NULL
                     AND finished_at IS NULL)
                 OR (status IN ('succeeded', 'failed', 'skipped', 'cancelled', 'unknown')
                     AND owner_id IS NULL AND lease_expires_at IS NULL
                     AND finished_at IS NOT NULL)
             )
         );
         CREATE INDEX idx_delivery_run_items_claim
             ON delivery_run_items(delivery_run_id, status, lease_expires_at, id);

         CREATE TABLE delivery_dedupe (
             id                  INTEGER PRIMARY KEY AUTOINCREMENT,
             workflow            TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             db_name             TEXT NOT NULL CHECK (length(db_name) BETWEEN 1 AND 255),
             user_id             INTEGER NOT NULL CHECK (user_id > 0),
             article_id          INTEGER NOT NULL CHECK (article_id > 0),
             delivery_run_id     INTEGER REFERENCES delivery_runs(id) ON DELETE SET NULL,
             status              TEXT NOT NULL CHECK (status IN ('reserved', 'confirmed', 'unknown')),
             message_id          TEXT CHECK (
                 message_id IS NULL OR length(message_id) BETWEEN 1 AND 256
             ),
             reservation_owner   TEXT CHECK (
                 reservation_owner IS NULL OR length(reservation_owner) BETWEEN 1 AND 128
             ),
             legacy_delivered_at TEXT,
             revision            INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             reserved_at         REAL NOT NULL,
             delivered_at        REAL,
             updated_at          REAL NOT NULL,
             CHECK (
                 (status = 'reserved' AND delivery_run_id IS NOT NULL
                  AND reservation_owner IS NOT NULL AND delivered_at IS NULL)
                 OR (status IN ('confirmed', 'unknown')
                     AND reservation_owner IS NULL AND delivered_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX idx_delivery_dedupe_identity
             ON delivery_dedupe(workflow, db_name, user_id, article_id);
         CREATE INDEX idx_delivery_dedupe_status_time
             ON delivery_dedupe(status, updated_at, id);

         CREATE TABLE delivery_leases (
             id                  INTEGER PRIMARY KEY AUTOINCREMENT,
             workflow            TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             db_name             TEXT NOT NULL CHECK (length(db_name) BETWEEN 1 AND 255),
             delivery_run_id     INTEGER REFERENCES delivery_runs(id) ON DELETE SET NULL,
             owner_id            TEXT CHECK (
                 owner_id IS NULL OR length(owner_id) BETWEEN 1 AND 128
             ),
             revision            INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             acquired_at         REAL,
             heartbeat_at        REAL,
             expires_at          REAL,
             updated_at          REAL NOT NULL,
             CHECK (
                 (owner_id IS NULL AND delivery_run_id IS NULL AND acquired_at IS NULL
                  AND heartbeat_at IS NULL AND expires_at IS NULL)
                 OR (owner_id IS NOT NULL AND delivery_run_id IS NOT NULL
                     AND acquired_at IS NOT NULL AND heartbeat_at IS NOT NULL
                     AND expires_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX idx_delivery_leases_scope
             ON delivery_leases(workflow, db_name);
         CREATE INDEX idx_delivery_leases_expiration
             ON delivery_leases(expires_at)
             WHERE owner_id IS NOT NULL;",
    )?;
    Ok(())
}

fn apply_auth_version_eleven(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    transaction.execute_batch(
        "UPDATE folders
         SET is_tracking = 0
         WHERE is_tracking = 1
           AND id NOT IN (
               SELECT MIN(id) FROM folders WHERE is_tracking = 1 GROUP BY user_id
           );
         CREATE UNIQUE INDEX idx_folders_one_tracking_per_user
             ON folders(user_id) WHERE is_tracking = 1;",
    )?;
    Ok(())
}

fn apply_auth_version_twelve(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let invite_columns = table_columns(transaction, "invite_codes")?;
    if invite_columns.iter().any(|column| column == "expires_at") {
        let expected_invite_columns = [
            "id",
            "code",
            "created_by",
            "used_by",
            "used_at",
            "created_at",
            "expires_at",
            "revoked_at",
            "max_uses",
            "use_count",
        ];
        let expected_use_columns = ["id", "invite_code_id", "user_id", "used_at"];
        if invite_columns != expected_invite_columns
            || table_columns(transaction, "invite_code_uses")? != expected_use_columns
        {
            return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
        }
        transaction.execute_batch(INVITE_LIFECYCLE_INDEXES_SQL)?;
        return Ok(());
    }

    let migrated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let default_ttl = litradar_domain::DEFAULT_INVITE_CODE_TTL_SECONDS as f64;
    transaction.execute_batch(INVITE_LIFECYCLE_TABLES_SQL)?;
    transaction.execute(
        "INSERT INTO invite_codes_v12 (
             id, code, created_by, used_by, used_at, created_at, expires_at,
             revoked_at, max_uses, use_count
         )
         SELECT id, code, created_by, used_by,
                CASE WHEN used_by IS NOT NULL THEN COALESCE(used_at, created_at) ELSE used_at END,
                created_at,
                MAX(created_at + ?1, ?2), NULL, 1,
                CASE WHEN used_by IS NOT NULL OR used_at IS NOT NULL THEN 1 ELSE 0 END
         FROM invite_codes",
        params![default_ttl, migrated_at + default_ttl],
    )?;
    transaction.execute(
        "UPDATE invite_codes_v12
         SET revoked_at = MAX(?1, created_at)
         WHERE created_by IS NOT NULL
           AND revoked_at IS NULL
           AND id NOT IN (
               SELECT MAX(id) FROM invite_codes_v12
               WHERE created_by IS NOT NULL GROUP BY created_by
           )",
        [migrated_at],
    )?;
    transaction.execute(
        "INSERT INTO invite_code_uses (invite_code_id, user_id, used_at)
         SELECT id, used_by, COALESCE(used_at, created_at)
         FROM invite_codes_v12
         WHERE used_by IS NOT NULL OR used_at IS NOT NULL",
        [],
    )?;
    transaction.execute_batch(
        "DROP TABLE invite_codes;
         ALTER TABLE invite_codes_v12 RENAME TO invite_codes;",
    )?;
    transaction.execute_batch(INVITE_LIFECYCLE_INDEXES_SQL)?;
    let foreign_key_violation_count =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })?;
    if foreign_key_violation_count != 0 {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    Ok(())
}

fn apply_auth_version_thirteen(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let cnki_columns = table_columns(transaction, "cnki_sessions")?;
    if !cnki_columns.is_empty() && !cnki_columns.iter().any(|column| column == "generation") {
        transaction.execute(
            "ALTER TABLE cnki_sessions ADD COLUMN generation INTEGER NOT NULL DEFAULT 1 CHECK (generation > 0)",
            [],
        )?;
    }
    Ok(())
}

fn apply_auth_version_fourteen(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let has_notification_settings =
        !table_columns(transaction, "notification_settings")?.is_empty();
    if has_notification_settings {
        validate_notification_string_lists(transaction)?;
    }
    transaction.execute_batch(NOTIFICATION_SETTINGS_V14_TABLE_SQL)?;
    if has_notification_settings {
        transaction.execute_batch(
            "INSERT INTO notification_settings_v14 (
             id, user_id, keywords, directions, selected_databases, delivery_method,
             pushplus_token, pushplus_template, pushplus_topic, pushplus_channel,
             sync_to_tracking_folder, ai_base_url, ai_api_key, ai_model, ai_system_prompt,
             ai_backup_base_url, ai_backup_api_key, ai_backup_model, ai_backup_system_prompt,
             ai_retry_attempts, enabled, created_at, updated_at
         )
         SELECT
             id, user_id, keywords, directions, selected_databases, delivery_method,
             pushplus_token, pushplus_template, pushplus_topic, pushplus_channel,
             sync_to_tracking_folder, ai_base_url, ai_api_key, ai_model, ai_system_prompt,
             ai_backup_base_url, ai_backup_api_key, ai_backup_model, ai_backup_system_prompt,
             ai_retry_attempts, enabled, created_at, updated_at
         FROM notification_settings;
         DROP TABLE notification_settings;
         ALTER TABLE notification_settings_v14 RENAME TO notification_settings;
         CREATE INDEX idx_notification_settings_user ON notification_settings(user_id);",
        )?;
    } else {
        transaction.execute_batch(
            "ALTER TABLE notification_settings_v14 RENAME TO notification_settings;
             CREATE INDEX idx_notification_settings_user ON notification_settings(user_id);",
        )?;
    }
    transaction.execute_batch(NOTIFICATION_SETTINGS_V14_TRIGGERS_SQL)?;
    let foreign_key_violation_count =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })?;
    if foreign_key_violation_count != 0 {
        return Err(MigrationError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    Ok(())
}

fn apply_auth_version_fifteen(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let user_columns = table_columns(transaction, "users")?;
    if !user_columns.is_empty()
        && !user_columns
            .iter()
            .any(|column| column == "token_generation")
    {
        transaction.execute(
            "ALTER TABLE users ADD COLUMN token_generation INTEGER NOT NULL DEFAULT 0 CHECK (token_generation >= 0)",
            [],
        )?;
    }
    Ok(())
}

fn validate_notification_string_lists(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let mut statement = transaction.prepare(
        "SELECT keywords, directions, selected_databases FROM notification_settings ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (keywords, directions, selected_databases) = row?;
        for value in [keywords, directions, selected_databases] {
            serde_json::from_str::<Vec<String>>(&value)
                .map_err(|_| MigrationError::InvalidNotificationSettingsState)?;
        }
    }
    Ok(())
}

fn rewrite_runtime_provider_name_tokens(
    transaction: &Transaction<'_>,
) -> Result<(), MigrationError> {
    let mut statement = transaction.prepare(
        "SELECT key, value, updated_at FROM runtime_settings
         WHERE key IN (
             'index_provider_routes',
             'article_abstract_provider_orders',
             'article_fulltext_provider_orders'
         )",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    for (key, value, updated_at) in rows {
        let rewritten = match key.as_str() {
            "index_provider_routes" => rewrite_index_provider_route_tokens(&value)?,
            "article_abstract_provider_orders" | "article_fulltext_provider_orders" => {
                rewrite_provider_order_tokens(&value)?
            }
            _ => continue,
        };
        if rewritten == value {
            continue;
        }
        transaction.execute(
            "UPDATE runtime_settings SET value = ?1 WHERE key = ?2 AND updated_at = ?3",
            params![rewritten, key, updated_at],
        )?;
    }
    Ok(())
}

fn rewrite_index_provider_route_tokens(value: &str) -> Result<String, MigrationError> {
    let parsed: serde_json::Value = serde_json::from_str(value)
        .map_err(|_| MigrationError::InvalidRuntimeProviderOrderState)?;
    let object = parsed
        .as_object()
        .ok_or(MigrationError::InvalidRuntimeProviderOrderState)?;
    let mut routes = BTreeMap::new();
    for (catalog, provider) in object {
        let provider = provider
            .as_str()
            .ok_or(MigrationError::InvalidRuntimeProviderOrderState)?;
        routes.insert(catalog.clone(), rewrite_provider_runtime_name(provider));
    }
    serde_json::to_string(&routes).map_err(|_| MigrationError::InvalidRuntimeProviderOrderState)
}

fn rewrite_provider_order_tokens(value: &str) -> Result<String, MigrationError> {
    let mut configuration: ProviderOrderConfiguration = serde_json::from_str(value)
        .map_err(|_| MigrationError::InvalidRuntimeProviderOrderState)?;
    configuration.default = configuration
        .default
        .into_iter()
        .map(|name| rewrite_provider_runtime_name(&name))
        .collect();
    configuration.catalogs = configuration
        .catalogs
        .into_iter()
        .map(|(catalog, providers)| {
            (
                catalog,
                providers
                    .into_iter()
                    .map(|name| rewrite_provider_runtime_name(&name))
                    .collect(),
            )
        })
        .collect();
    serde_json::to_string(&configuration)
        .map_err(|_| MigrationError::InvalidRuntimeProviderOrderState)
}

fn rewrite_provider_runtime_name(name: &str) -> String {
    match name {
        "cnki" => "cnki_oversea".to_string(),
        "zjlib_cnki" => "zjlib".to_string(),
        other => other.to_string(),
    }
}

fn legacy_provider_order_row(
    transaction: &Transaction<'_>,
    field: &str,
) -> Result<Option<(Vec<String>, f64)>, MigrationError> {
    let row = transaction
        .query_row(
            "SELECT value, updated_at FROM runtime_settings WHERE key = ?1",
            [field],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()?;
    row.map(|(value, updated_at)| {
        parse_legacy_provider_order(&value).map(|providers| (providers, updated_at))
    })
    .transpose()
}

fn parse_legacy_provider_order(value: &str) -> Result<Vec<String>, MigrationError> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut providers = Vec::new();
    let mut seen = BTreeSet::new();
    for part in value.split(',') {
        let provider = part.trim();
        if !is_runtime_name(provider) || !seen.insert(provider.to_string()) {
            return Err(MigrationError::InvalidRuntimeProviderOrderState);
        }
        providers.push(provider.to_string());
    }
    Ok(providers)
}

fn upsert_provider_order_configuration(
    transaction: &Transaction<'_>,
    field: &str,
    providers: &[String],
    updated_at: f64,
) -> Result<(), MigrationError> {
    let value = serde_json::to_string(&ProviderOrderConfiguration {
        default: providers.to_vec(),
        catalogs: BTreeMap::new(),
    })
    .map_err(|_| MigrationError::InvalidRuntimeProviderOrderState)?;
    transaction.execute(
        "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![field, value, updated_at],
    )?;
    Ok(())
}

fn is_runtime_name(value: &str) -> bool {
    (2..=128).contains(&value.len())
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'.' | b'_' | b'-' => index > 0,
            _ => false,
        })
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
        updated_at    REAL    NOT NULL,
        token_generation INTEGER NOT NULL DEFAULT 0 CHECK (token_generation >= 0)
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

const NOTIFICATION_SETTINGS_V14_TABLE_SQL: &str = "
    CREATE TABLE notification_settings_v14 (
        id                      INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id                 INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
        keywords                TEXT    NOT NULL DEFAULT '[]' CHECK (
            CASE WHEN json_valid(keywords)
                 THEN json_type(keywords) = 'array'
                 ELSE 0
            END
        ),
        directions              TEXT    NOT NULL DEFAULT '[]' CHECK (
            CASE WHEN json_valid(directions)
                 THEN json_type(directions) = 'array'
                 ELSE 0
            END
        ),
        selected_databases      TEXT    NOT NULL DEFAULT '[]' CHECK (
            CASE WHEN json_valid(selected_databases)
                 THEN json_type(selected_databases) = 'array'
                 ELSE 0
            END
        ),
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
";

const NOTIFICATION_SETTINGS_V14_TRIGGERS_SQL: &str = "
    CREATE TRIGGER notification_settings_json_strings_insert
    AFTER INSERT ON notification_settings
    WHEN EXISTS (SELECT 1 FROM json_each(NEW.keywords) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.directions) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.selected_databases) WHERE type <> 'text')
    BEGIN
        SELECT RAISE(ABORT, 'notification settings JSON arrays must contain strings');
    END;

    CREATE TRIGGER notification_settings_json_strings_update
    AFTER UPDATE OF keywords, directions, selected_databases ON notification_settings
    WHEN EXISTS (SELECT 1 FROM json_each(NEW.keywords) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.directions) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.selected_databases) WHERE type <> 'text')
    BEGIN
        SELECT RAISE(ABORT, 'notification settings JSON arrays must contain strings');
    END;
";

const INVITE_LIFECYCLE_TABLES_SQL: &str = "
    CREATE TABLE invite_codes_v12 (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        code        TEXT    NOT NULL UNIQUE,
        created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_at     REAL,
        created_at  REAL    NOT NULL,
        expires_at  REAL    NOT NULL CHECK (expires_at > created_at),
        revoked_at  REAL    CHECK (revoked_at IS NULL OR revoked_at >= created_at),
        max_uses    INTEGER NOT NULL CHECK (max_uses BETWEEN 1 AND 1000),
        use_count   INTEGER NOT NULL DEFAULT 0 CHECK (
            use_count >= 0 AND use_count <= max_uses
        ),
        CHECK (used_by IS NULL OR used_at IS NOT NULL)
    );

    CREATE TABLE invite_code_uses (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        invite_code_id INTEGER NOT NULL REFERENCES invite_codes_v12(id) ON DELETE RESTRICT,
        user_id        INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_at        REAL NOT NULL
    );
";

const INVITE_LIFECYCLE_INDEXES_SQL: &str = "
    CREATE INDEX IF NOT EXISTS idx_invite_codes_code ON invite_codes(code);
    CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by ON invite_codes(created_by);
    CREATE UNIQUE INDEX IF NOT EXISTS idx_invite_codes_one_unrevoked_creator
        ON invite_codes(created_by)
        WHERE created_by IS NOT NULL AND revoked_at IS NULL;
    CREATE INDEX IF NOT EXISTS idx_invite_codes_lifecycle
        ON invite_codes(revoked_at, expires_at, use_count, max_uses);
    CREATE INDEX IF NOT EXISTS idx_invite_code_uses_code_time
        ON invite_code_uses(invite_code_id, used_at, id);
    CREATE INDEX IF NOT EXISTS idx_invite_code_uses_user
        ON invite_code_uses(user_id, used_at, id);
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

const INDEX_VERSION_FIVE_SQL: &str = "
    CREATE TABLE journal_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
        identity_value TEXT NOT NULL,
        canonical_catalog_id TEXT NOT NULL,
        PRIMARY KEY (identity_kind, identity_value)
    );
    CREATE INDEX idx_journal_identity_keys_catalog
        ON journal_identity_keys(canonical_catalog_id);
";

const INDEX_VERSION_SIX_SQL: &str = "
    CREATE TABLE articles_v6 (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        title TEXT NOT NULL,
        publication_year INTEGER,
        date TEXT,
        authors_json TEXT NOT NULL,
        start_page TEXT,
        end_page TEXT,
        abstract_text TEXT,
        doi TEXT,
        pmid TEXT,
        open_access INTEGER,
        in_press INTEGER,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    INSERT INTO articles_v6 (
        article_id, journal_id, issue_id, title, publication_year, date, authors_json,
        start_page, end_page, abstract_text, doi, pmid, open_access, in_press
    )
    SELECT
        article_id, journal_id, issue_id, title, publication_year, date, authors_json,
        start_page, end_page, abstract_text, doi, pmid, open_access, in_press
    FROM articles;

    DROP TABLE articles;
    ALTER TABLE articles_v6 RENAME TO articles;

    CREATE TABLE article_retraction_dois (
        article_id INTEGER NOT NULL,
        retraction_doi TEXT NOT NULL,
        PRIMARY KEY (article_id, retraction_doi),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE INDEX idx_articles_journal ON articles(journal_id);
    CREATE INDEX idx_articles_issue ON articles(issue_id);
    CREATE INDEX idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX idx_articles_doi ON articles(doi);
    CREATE INDEX idx_articles_pmid ON articles(pmid);
    CREATE INDEX idx_article_retraction_dois_doi
        ON article_retraction_dois(retraction_doi);
";

pub(crate) const INDEX_CONTENT_TABLES_SQL: &str = "
    CREATE TABLE journals (
        journal_id INTEGER PRIMARY KEY,
        catalog_id TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        title_aliases_json TEXT NOT NULL,
        issns_json TEXT NOT NULL,
        issn TEXT,
        eissn TEXT,
        area TEXT,
        utd_rank TEXT,
        utd_rating TEXT,
        abs_rank TEXT,
        abs_rating TEXT,
        fms_rank TEXT,
        fms_rating TEXT,
        fmscn_rank TEXT,
        fmscn_rating TEXT
    );

    CREATE TABLE journal_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
        identity_value TEXT NOT NULL,
        canonical_catalog_id TEXT NOT NULL,
        PRIMARY KEY (identity_kind, identity_value)
    );

    CREATE TABLE issues (
        issue_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        publication_year INTEGER,
        title TEXT,
        volume TEXT,
        number TEXT,
        date TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
    );

    CREATE TABLE articles (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        title TEXT NOT NULL,
        publication_year INTEGER,
        date TEXT,
        authors_json TEXT NOT NULL,
        start_page TEXT,
        end_page TEXT,
        abstract_text TEXT,
        doi TEXT,
        pmid TEXT,
        open_access INTEGER,
        in_press INTEGER,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE TABLE article_retraction_dois (
        article_id INTEGER NOT NULL,
        retraction_doi TEXT NOT NULL,
        PRIMARY KEY (article_id, retraction_doi),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('doi', 'pmid', 'bibliographic')),
        identity_value TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        PRIMARY KEY (identity_kind, identity_value),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_listing (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        publication_year INTEGER,
        date TEXT,
        open_access INTEGER,
        in_press INTEGER,
        doi TEXT,
        pmid TEXT,
        area TEXT,
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

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

    CREATE TABLE article_change_events (
        event_id INTEGER PRIMARY KEY,
        content_revision TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        change_kind TEXT NOT NULL CHECK (change_kind IN ('upsert', 'remove')),
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        in_press INTEGER NOT NULL CHECK (in_press IN (0, 1)),
        created_at TEXT NOT NULL
    );

    CREATE INDEX idx_journals_issn ON journals(issn);
    CREATE INDEX idx_journals_eissn ON journals(eissn);
    CREATE INDEX idx_journal_identity_keys_catalog
        ON journal_identity_keys(canonical_catalog_id);
    CREATE INDEX idx_issues_journal_year ON issues(journal_id, publication_year);
    CREATE INDEX idx_articles_journal ON articles(journal_id);
    CREATE INDEX idx_articles_issue ON articles(issue_id);
    CREATE INDEX idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX idx_articles_doi ON articles(doi);
    CREATE INDEX idx_articles_pmid ON articles(pmid);
    CREATE INDEX idx_article_retraction_dois_doi
        ON article_retraction_dois(retraction_doi);
    CREATE INDEX idx_article_identity_keys_article ON article_identity_keys(article_id);
    CREATE INDEX idx_article_listing_date_id ON article_listing(date, article_id);
    CREATE INDEX idx_article_listing_journal_date_id
        ON article_listing(journal_id, date, article_id);
    CREATE INDEX idx_article_listing_issue ON article_listing(issue_id);
    CREATE UNIQUE INDEX idx_article_change_events_revision
        ON article_change_events(
            content_revision, article_id, change_kind, journal_id,
            COALESCE(issue_id, -1), in_press
        );
    CREATE INDEX idx_article_change_events_order ON article_change_events(event_id);
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
