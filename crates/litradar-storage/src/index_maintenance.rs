//! Offline copy-on-write maintenance for article index databases.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::article_authors::decode_article_author_names;
use crate::backup::{has_recent_service_heartbeat, ACTIVE_HEARTBEAT_MAX_AGE_SECONDS};
use crate::migrations::{
    migrate_index_database, preflight_index_database, INDEX_SCHEMA_VERSION,
    MIN_SUPPORTED_INDEX_SCHEMA_VERSION,
};
use crate::StorageConfig;

const MAINTENANCE_MARKER_NAME: &str = ".litradar-index-maintenance.json";
const MAINTENANCE_STAGING_NAME: &str = ".litradar-index-staging";
const MAINTENANCE_ROLLBACK_NAME: &str = ".litradar-index-rollback";
const TEMPORARY_SPACE_OVERHEAD_BYTES: u64 = 64 * 1024 * 1024;
const BUSY_TIMEOUT_SECONDS: u64 = 30;

const SEARCH_CORPUS: [&str; 10] = [
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
];

const AUTHORITATIVE_TABLES: [TableSpec; 8] = [
    TableSpec {
        name: "journals",
        columns: "journal_id, catalog_id, title, title_aliases_json, issns_json, issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating, fms_rank, fms_rating, fmscn_rank, fmscn_rating",
        key_columns: "journal_id",
    },
    TableSpec {
        name: "journal_identity_keys",
        columns: "identity_kind, identity_value, canonical_catalog_id",
        key_columns: "identity_kind, identity_value",
    },
    TableSpec {
        name: "issues",
        columns: "issue_id, journal_id, publication_year, title, volume, number, date",
        key_columns: "issue_id",
    },
    TableSpec {
        name: "articles",
        columns: "article_id, journal_id, issue_id, title, publication_year, date, authors_json, start_page, end_page, abstract_text, doi, pmid, open_access, in_press",
        key_columns: "article_id",
    },
    TableSpec {
        name: "article_retraction_dois",
        columns: "article_id, retraction_doi",
        key_columns: "article_id, retraction_doi",
    },
    TableSpec {
        name: "article_identity_keys",
        columns: "identity_kind, identity_value, article_id",
        key_columns: "identity_kind, identity_value",
    },
    TableSpec {
        name: "article_listing",
        columns: "article_id, journal_id, issue_id, publication_year, date, open_access, in_press, doi, pmid, area",
        key_columns: "article_id",
    },
    TableSpec {
        name: "article_change_events",
        columns: "event_id, content_revision, article_id, change_kind, journal_id, issue_id, in_press, created_at",
        key_columns: "event_id",
    },
];

#[derive(Debug, Clone, Copy)]
struct TableSpec {
    name: &'static str,
    columns: &'static str,
    key_columns: &'static str,
}

/// Inputs for one confirmed offline index storage optimization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStorageOptimizationOptions {
    /// Project-derived storage paths for the inactive target.
    pub storage_config: StorageConfig,
    /// Explicit operator confirmation required before maintenance starts.
    pub confirmed: bool,
}

/// Exact maintenance paths retained when interrupted recovery is required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexStorageRecoveryPaths {
    /// Durable operation marker outside the replaceable index directory.
    pub marker: PathBuf,
    /// Complete candidate index directory built before replacement.
    pub staging: PathBuf,
    /// Original index directory retained across replacement validation.
    pub rollback: PathBuf,
}

/// Physical SQLite storage measurements that exclude row contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexDatabaseStorageMeasurement {
    /// Database file size in bytes.
    pub file_bytes: u64,
    /// SQLite page size in bytes.
    pub page_size: u64,
    /// Allocated SQLite page count.
    pub page_count: u64,
    /// SQLite freelist page count.
    pub freelist_count: u64,
    /// Bytes allocated to freelist pages.
    pub freelist_bytes: u64,
    /// Bytes allocated to FTS5 tables and indexes.
    pub fts_bytes: u64,
    /// Whether the duplicate stored-content FTS shadow table exists.
    pub has_content_shadow: bool,
}

/// Per-database result from a successful offline optimization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexDatabaseOptimizationReport {
    /// Safe database filename without its project path.
    pub database: String,
    /// Supported schema version read from the source database.
    pub source_schema_version: i64,
    /// Exact schema version built in the replacement database.
    pub target_schema_version: i64,
    /// Source storage measurements captured before copying.
    pub before: IndexDatabaseStorageMeasurement,
    /// Replacement storage measurements captured after validation.
    pub after: IndexDatabaseStorageMeasurement,
    /// Authoritative row counts keyed by table name.
    pub row_counts: BTreeMap<String, u64>,
}

/// Outcome kind for a successful offline optimization invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexStorageOptimizationOutcome {
    /// No index databases existed, so no filesystem state changed.
    Noop,
    /// Every database was rebuilt, validated, and replaced atomically by directory.
    Optimized,
}

/// Successful report for an offline index storage optimization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexStorageOptimizationReport {
    /// Whether the invocation optimized databases or completed as an empty no-op.
    pub outcome: IndexStorageOptimizationOutcome,
    /// Number of rebuilt index databases.
    pub database_count: usize,
    /// Aggregate source database bytes before maintenance.
    pub source_bytes: u64,
    /// Conservative temporary-space requirement reported before copying.
    pub temporary_bytes_required: u64,
    /// Aggregate replacement database bytes after maintenance.
    pub optimized_bytes: u64,
    /// Aggregate bytes reclaimed without underflow when a small fixture grows.
    pub reclaimed_bytes: u64,
    /// Redacted per-database measurements and row counts.
    pub databases: Vec<IndexDatabaseOptimizationReport>,
}

/// Errors returned by the fail-closed offline index optimizer.
#[derive(Debug)]
pub enum IndexStorageOptimizationError {
    /// The destructive maintenance confirmation flag was absent.
    ConfirmationRequired,
    /// A recent API, worker, or scheduler heartbeat marks the target active.
    ActiveTarget,
    /// An unexpired index batch or Provider lease marks the target active.
    ActiveLease {
        /// Redacted lease family.
        lease_kind: &'static str,
        /// Safe control database filename.
        database: String,
        /// Stored lease expiry as Unix epoch seconds.
        expires_at: i64,
    },
    /// A marker, staging directory, or rollback directory requires manual recovery.
    InterruptedState(Box<IndexStorageRecoveryPaths>),
    /// The index directory contains material that cannot be copied safely.
    InvalidLayout(String),
    /// One source database is outside the supported v6/v7 rollout window.
    UnsupportedDatabase {
        /// Safe database filename.
        database: String,
        /// Schema version found in the source database.
        found: i64,
    },
    /// A required integrity, equivalence, or storage check failed.
    Validation {
        /// Safe database filename or component label.
        database: String,
        /// Redacted validation failure description.
        check: String,
    },
    /// Source metadata changed between initial inspection and replacement.
    SourceChanged(String),
    /// Filesystem access failed before a durable marker was acquired.
    Io(std::io::Error),
    /// SQLite access failed before a durable marker was acquired.
    Sqlite(rusqlite::Error),
    /// A caught failure after marker acquisition retained recovery evidence.
    OperationFailed {
        /// Redacted operation phase or original error code.
        phase: &'static str,
        /// Redacted underlying failure text.
        detail: String,
        /// Exact marker, staging, and rollback recovery paths.
        recovery_paths: Box<IndexStorageRecoveryPaths>,
    },
    /// The original directory could not be restored after a switch failure.
    RollbackFailed {
        /// Redacted primary and rollback failure text.
        detail: String,
        /// Exact marker, staging, and rollback recovery paths.
        recovery_paths: Box<IndexStorageRecoveryPaths>,
    },
}

impl IndexStorageOptimizationError {
    /// Return a stable machine-readable error code.
    ///
    /// # Returns
    ///
    /// Snake-case error code suitable for structured CLI output.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConfirmationRequired => "confirmation_required",
            Self::ActiveTarget => "active_target",
            Self::ActiveLease { .. } => "active_lease",
            Self::InterruptedState(_) => "interrupted_state",
            Self::InvalidLayout(_) => "invalid_layout",
            Self::UnsupportedDatabase { .. } => "unsupported_schema",
            Self::Validation { .. } => "validation_failed",
            Self::SourceChanged(_) => "source_changed",
            Self::Io(_) => "io",
            Self::Sqlite(_) => "sqlite",
            Self::OperationFailed { .. } => "operation_failed",
            Self::RollbackFailed { .. } => "rollback_failed",
        }
    }

    /// Return exact recovery paths when maintenance evidence was retained.
    ///
    /// # Returns
    ///
    /// Marker, staging, and rollback paths for interrupted-state recovery.
    pub fn recovery_paths(&self) -> Option<&IndexStorageRecoveryPaths> {
        match self {
            Self::InterruptedState(paths)
            | Self::OperationFailed {
                recovery_paths: paths,
                ..
            }
            | Self::RollbackFailed {
                recovery_paths: paths,
                ..
            } => Some(paths.as_ref()),
            _ => None,
        }
    }

    fn with_recovery(self, paths: &IndexStorageRecoveryPaths) -> Self {
        match self {
            Self::RollbackFailed { .. } | Self::InterruptedState(_) => self,
            other => Self::OperationFailed {
                phase: other.code(),
                detail: other.to_string(),
                recovery_paths: Box::new(paths.clone()),
            },
        }
    }
}

impl fmt::Display for IndexStorageOptimizationError {
    /// Format an optimization error without exposing article contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfirmationRequired => formatter.write_str(
                "index storage optimization requires --confirm-index-maintenance",
            ),
            Self::ActiveTarget => formatter.write_str(
                "index storage optimization refused because a recent service heartbeat marks the target active",
            ),
            Self::ActiveLease {
                lease_kind,
                database,
                expires_at,
            } => write!(
                formatter,
                "index storage optimization refused because {lease_kind} in {database} remains leased until {expires_at}",
            ),
            Self::InterruptedState(paths) => write!(
                formatter,
                "interrupted index maintenance requires recovery; marker={}, staging={}, rollback={}",
                paths.marker.display(),
                paths.staging.display(),
                paths.rollback.display(),
            ),
            Self::InvalidLayout(message) => {
                write!(formatter, "unsafe index directory layout: {message}")
            }
            Self::UnsupportedDatabase { database, found } => write!(
                formatter,
                "index database {database} uses unsupported schema version {found}; supported versions are {MIN_SUPPORTED_INDEX_SCHEMA_VERSION} through {INDEX_SCHEMA_VERSION}",
            ),
            Self::Validation { database, check } => {
                write!(formatter, "index database {database} failed {check}")
            }
            Self::SourceChanged(database) => write!(
                formatter,
                "index source changed during maintenance: {database}",
            ),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::OperationFailed {
                phase,
                detail,
                recovery_paths,
            } => write!(
                formatter,
                "index maintenance failed during {phase}: {detail}; marker={}, staging={}, rollback={}",
                recovery_paths.marker.display(),
                recovery_paths.staging.display(),
                recovery_paths.rollback.display(),
            ),
            Self::RollbackFailed {
                detail,
                recovery_paths,
            } => write!(
                formatter,
                "index maintenance rollback failed: {detail}; marker={}, staging={}, rollback={}",
                recovery_paths.marker.display(),
                recovery_paths.staging.display(),
                recovery_paths.rollback.display(),
            ),
        }
    }
}

impl Error for IndexStorageOptimizationError {
    /// Return the underlying filesystem or SQLite error when available.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IndexStorageOptimizationError {
    /// Convert filesystem failures into optimization errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for IndexStorageOptimizationError {
    /// Convert SQLite failures into optimization errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceDatabase {
    name: String,
    path: PathBuf,
    file_bytes: u64,
    modified_at: Option<SystemTime>,
    schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexDirectorySnapshot {
    databases: Vec<SourceDatabase>,
    source_bytes: u64,
}

trait IndexOptimizationHook {
    fn before_database_copy(
        &mut self,
        _database: &str,
    ) -> Result<(), IndexStorageOptimizationError> {
        Ok(())
    }

    fn before_staging_validation(&mut self) -> Result<(), IndexStorageOptimizationError> {
        Ok(())
    }

    fn after_source_rename(&mut self) -> Result<(), IndexStorageOptimizationError> {
        Ok(())
    }

    fn after_switch(&mut self) -> Result<(), IndexStorageOptimizationError> {
        Ok(())
    }

    fn before_rollback(&mut self) -> Result<(), IndexStorageOptimizationError> {
        Ok(())
    }
}

struct NoopIndexOptimizationHook;

impl IndexOptimizationHook for NoopIndexOptimizationHook {}

/// Optimize every inactive index database through a validated directory replacement.
///
/// # Arguments
///
/// * `options` - Project storage paths and explicit maintenance confirmation.
///
/// # Returns
///
/// Redacted size, schema, and row-count report after a no-op or successful replacement.
pub fn optimize_index_storage(
    options: &IndexStorageOptimizationOptions,
) -> Result<IndexStorageOptimizationReport, IndexStorageOptimizationError> {
    let mut hook = NoopIndexOptimizationHook;
    optimize_index_storage_with_hook(options, current_epoch_seconds(), &mut hook)
}

pub(crate) fn interrupted_index_maintenance_state(
    config: &StorageConfig,
) -> Result<Option<IndexStorageRecoveryPaths>, std::io::Error> {
    let paths = maintenance_paths(config)?;
    if path_exists(&paths.marker)? || path_exists(&paths.staging)? || path_exists(&paths.rollback)?
    {
        Ok(Some(paths))
    } else {
        Ok(None)
    }
}

fn optimize_index_storage_with_hook(
    options: &IndexStorageOptimizationOptions,
    current_time: i64,
    hook: &mut impl IndexOptimizationHook,
) -> Result<IndexStorageOptimizationReport, IndexStorageOptimizationError> {
    if !options.confirmed {
        return Err(IndexStorageOptimizationError::ConfirmationRequired);
    }
    let paths = maintenance_paths(&options.storage_config)?;
    if let Some(existing) = interrupted_index_maintenance_state(&options.storage_config)? {
        return Err(IndexStorageOptimizationError::InterruptedState(Box::new(
            existing,
        )));
    }
    validate_data_directory(&options.storage_config)?;
    let mut source = inspect_index_directory(&options.storage_config)?;
    if source.databases.is_empty() {
        return Ok(IndexStorageOptimizationReport {
            outcome: IndexStorageOptimizationOutcome::Noop,
            database_count: 0,
            source_bytes: 0,
            temporary_bytes_required: 0,
            optimized_bytes: 0,
            reclaimed_bytes: 0,
            databases: Vec::new(),
        });
    }
    ensure_target_inactive(&options.storage_config, current_time)?;
    for database in &mut source.databases {
        database.schema_version = validate_supported_source_schema(database)?;
    }
    let temporary_bytes_required = source
        .source_bytes
        .saturating_mul(2)
        .saturating_add(TEMPORARY_SPACE_OVERHEAD_BYTES);
    tracing::info!(
        event = "storage.index_optimization.estimate",
        component = "storage",
        database_count = source.databases.len(),
        source_bytes = source.source_bytes,
        temporary_bytes_required,
    );

    if let Err(error) = acquire_maintenance_marker(&paths, current_time) {
        return if path_exists(&paths.marker).unwrap_or(false) {
            Err(error.with_recovery(&paths))
        } else {
            Err(error)
        };
    }
    let operation = run_marked_optimization(
        options,
        &paths,
        &source,
        current_time,
        temporary_bytes_required,
        hook,
    );
    match operation {
        Ok(report) => Ok(report),
        Err(error) => {
            if !matches!(error, IndexStorageOptimizationError::RollbackFailed { .. })
                && options.storage_config.index_dir().exists()
                && !paths.rollback.exists()
            {
                let _ = remove_known_maintenance_directory(
                    &paths.staging,
                    options.storage_config.project_root(),
                    MAINTENANCE_STAGING_NAME,
                );
            }
            Err(error.with_recovery(&paths))
        }
    }
}

fn run_marked_optimization(
    options: &IndexStorageOptimizationOptions,
    paths: &IndexStorageRecoveryPaths,
    source: &IndexDirectorySnapshot,
    current_time: i64,
    temporary_bytes_required: u64,
    hook: &mut impl IndexOptimizationHook,
) -> Result<IndexStorageOptimizationReport, IndexStorageOptimizationError> {
    ensure_target_inactive(&options.storage_config, current_time)?;
    validate_source_integrity(source)?;
    fs::create_dir(&paths.staging)?;

    for database in &source.databases {
        hook.before_database_copy(&database.name)?;
        let staged_path = paths.staging.join(&database.name);
        build_staged_database(database, &staged_path)?;
    }
    fs::set_permissions(
        &paths.staging,
        fs::metadata(options.storage_config.index_dir())?.permissions(),
    )?;

    hook.before_staging_validation()?;
    let mut database_reports = Vec::with_capacity(source.databases.len());
    for database in &source.databases {
        database_reports.push(validate_rebuilt_database(
            database,
            &paths.staging.join(&database.name),
        )?);
    }

    let final_source = inspect_index_directory(&options.storage_config)?;
    ensure_source_unchanged(source, &final_source)?;
    ensure_target_inactive(&options.storage_config, current_time)?;
    switch_index_directory(&options.storage_config, paths, hook)?;

    let post_switch = hook.after_switch().and_then(|()| {
        for database in &source.databases {
            validate_rebuilt_database(
                &SourceDatabase {
                    path: paths.rollback.join(&database.name),
                    ..database.clone()
                },
                &options.storage_config.index_dir().join(&database.name),
            )?;
        }
        Ok(())
    });
    if let Err(error) = post_switch {
        rollback_switched_index_directory(&options.storage_config, paths, hook, &error)?;
        return Err(error);
    }

    remove_known_maintenance_directory(
        &paths.rollback,
        options.storage_config.project_root(),
        MAINTENANCE_ROLLBACK_NAME,
    )?;
    fs::remove_file(&paths.marker)?;

    let optimized_bytes = database_reports
        .iter()
        .map(|report| report.after.file_bytes)
        .sum::<u64>();
    Ok(IndexStorageOptimizationReport {
        outcome: IndexStorageOptimizationOutcome::Optimized,
        database_count: database_reports.len(),
        source_bytes: source.source_bytes,
        temporary_bytes_required,
        optimized_bytes,
        reclaimed_bytes: source.source_bytes.saturating_sub(optimized_bytes),
        databases: database_reports,
    })
}

fn maintenance_paths(config: &StorageConfig) -> Result<IndexStorageRecoveryPaths, std::io::Error> {
    let data_dir = std::path::absolute(config.project_root().join("data"))?;
    Ok(IndexStorageRecoveryPaths {
        marker: data_dir.join(MAINTENANCE_MARKER_NAME),
        staging: data_dir.join(MAINTENANCE_STAGING_NAME),
        rollback: data_dir.join(MAINTENANCE_ROLLBACK_NAME),
    })
}

fn path_exists(path: &Path) -> Result<bool, std::io::Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn validate_data_directory(config: &StorageConfig) -> Result<(), IndexStorageOptimizationError> {
    let data_dir = config.project_root().join("data");
    if !path_exists(&data_dir)? {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(data_dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "project data target must be a regular directory".to_string(),
        ));
    }
    Ok(())
}

fn inspect_index_directory(
    config: &StorageConfig,
) -> Result<IndexDirectorySnapshot, IndexStorageOptimizationError> {
    let index_dir = config.index_dir();
    if !path_exists(index_dir)? {
        return Ok(IndexDirectorySnapshot {
            databases: Vec::new(),
            source_bytes: 0,
        });
    }
    let directory_metadata = fs::symlink_metadata(index_dir)?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "index target must be a regular directory".to_string(),
        ));
    }
    if !cfg!(windows) && directory_metadata.permissions().readonly() {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "index target must be writable for directory replacement".to_string(),
        ));
    }

    let mut databases = Vec::new();
    let mut sidecars = Vec::new();
    for entry in fs::read_dir(index_dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(IndexStorageOptimizationError::InvalidLayout(
                "index directory entries must be regular files".to_string(),
            ));
        }
        if metadata.permissions().readonly() {
            return Err(IndexStorageOptimizationError::InvalidLayout(
                "index database and sidecar files must be writable for replacement".to_string(),
            ));
        }
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| {
                IndexStorageOptimizationError::InvalidLayout(
                    "index filenames must be valid UTF-8".to_string(),
                )
            })?;
        if name.ends_with(".sqlite") {
            databases.push(SourceDatabase {
                name,
                path,
                file_bytes: metadata.len(),
                modified_at: metadata.modified().ok(),
                schema_version: 0,
            });
        } else if let Some((database_name, kind)) = parse_sidecar_name(&name) {
            let database_name = database_name.to_string();
            sidecars.push((name, database_name, kind, metadata.len()));
        } else {
            return Err(IndexStorageOptimizationError::InvalidLayout(
                "unexpected file exists beside index databases".to_string(),
            ));
        }
    }
    databases.sort_by(|left, right| left.name.cmp(&right.name));
    for (sidecar, database_name, kind, size) in sidecars {
        if !databases
            .iter()
            .any(|database| database.name == database_name)
        {
            return Err(IndexStorageOptimizationError::InvalidLayout(format!(
                "orphaned SQLite sidecar {sidecar}"
            )));
        }
        if kind != "shm" && size != 0 {
            return Err(IndexStorageOptimizationError::InvalidLayout(format!(
                "non-empty SQLite {kind} sidecar {sidecar}"
            )));
        }
    }
    let source_bytes = databases.iter().map(|database| database.file_bytes).sum();
    Ok(IndexDirectorySnapshot {
        databases,
        source_bytes,
    })
}

fn parse_sidecar_name(name: &str) -> Option<(&str, &'static str)> {
    for (suffix, kind) in [("-wal", "wal"), ("-shm", "shm"), ("-journal", "journal")] {
        if let Some(database_name) = name.strip_suffix(suffix) {
            if database_name.ends_with(".sqlite") {
                return Some((database_name, kind));
            }
        }
    }
    None
}

fn validate_supported_source_schema(
    database: &SourceDatabase,
) -> Result<i64, IndexStorageOptimizationError> {
    if database.file_bytes == 0 {
        return Err(IndexStorageOptimizationError::UnsupportedDatabase {
            database: database.name.clone(),
            found: 0,
        });
    }
    let connection = open_read_only(&database.path)?;
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    drop(connection);
    if !(MIN_SUPPORTED_INDEX_SCHEMA_VERSION..=INDEX_SCHEMA_VERSION).contains(&version) {
        return Err(IndexStorageOptimizationError::UnsupportedDatabase {
            database: database.name.clone(),
            found: version,
        });
    }
    preflight_index_database(&database.path, None).map_err(|error| {
        IndexStorageOptimizationError::Validation {
            database: database.name.clone(),
            check: format!("exact schema preflight: {error}"),
        }
    })?;
    Ok(version)
}

fn ensure_target_inactive(
    config: &StorageConfig,
    current_time: i64,
) -> Result<(), IndexStorageOptimizationError> {
    if has_recent_service_heartbeat(
        config.auth_db_path(),
        current_time as f64,
        ACTIVE_HEARTBEAT_MAX_AGE_SECONDS,
    )
    .map_err(|error| IndexStorageOptimizationError::Validation {
        database: "auth.sqlite".to_string(),
        check: format!("activity gate: {error}"),
    })? {
        return Err(IndexStorageOptimizationError::ActiveTarget);
    }
    ensure_no_active_index_leases(config.index_control_dir(), current_time)
}

fn ensure_no_active_index_leases(
    control_dir: &Path,
    current_time: i64,
) -> Result<(), IndexStorageOptimizationError> {
    if !path_exists(control_dir)? {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(control_dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "index control target must be a regular directory".to_string(),
        ));
    }
    let mut databases = Vec::new();
    for entry in fs::read_dir(control_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("sqlite") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(IndexStorageOptimizationError::InvalidLayout(
                "index control databases must be regular files".to_string(),
            ));
        }
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| {
                IndexStorageOptimizationError::InvalidLayout(
                    "index control filenames must be valid UTF-8".to_string(),
                )
            })?;
        databases.push((name, path));
    }
    databases.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, path) in databases {
        let connection = open_read_only(&path)?;
        if sqlite_table_exists(&connection, "index_batch_lease")? {
            if let Some(expires_at) = connection
                .query_row(
                    "SELECT expires_at FROM index_batch_lease
                     WHERE lease_key = 1 AND expires_at > ?1",
                    [current_time],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
            {
                return Err(IndexStorageOptimizationError::ActiveLease {
                    lease_kind: "index batch lease",
                    database: name,
                    expires_at,
                });
            }
        }
        if sqlite_table_exists(&connection, "provider_leases")? {
            if let Some(expires_at) = connection.query_row(
                "SELECT MAX(expires_at) FROM provider_leases WHERE expires_at > ?1",
                [current_time],
                |row| row.get::<_, Option<i64>>(0),
            )? {
                return Err(IndexStorageOptimizationError::ActiveLease {
                    lease_kind: "Provider lease",
                    database: name,
                    expires_at,
                });
            }
        }
    }
    Ok(())
}

fn acquire_maintenance_marker(
    paths: &IndexStorageRecoveryPaths,
    current_time: i64,
) -> Result<(), IndexStorageOptimizationError> {
    let parent = paths.marker.parent().ok_or_else(|| {
        IndexStorageOptimizationError::InvalidLayout(
            "maintenance marker has no parent directory".to_string(),
        )
    })?;
    fs::create_dir_all(parent)?;
    let mut marker = match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&paths.marker)
    {
        Ok(marker) => marker,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(IndexStorageOptimizationError::InterruptedState(Box::new(
                paths.clone(),
            )));
        }
        Err(error) => return Err(error.into()),
    };
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "format": "litradar-index-maintenance",
        "version": 1,
        "started_at_epoch_seconds": current_time,
        "process_id": std::process::id(),
        "staging": paths.staging,
        "rollback": paths.rollback,
    }))
    .map_err(|error| {
        IndexStorageOptimizationError::InvalidLayout(format!(
            "maintenance marker serialization failed: {error}"
        ))
    })?;
    marker.write_all(&payload)?;
    marker.write_all(b"\n")?;
    marker.sync_all()?;
    Ok(())
}

fn validate_source_integrity(
    source: &IndexDirectorySnapshot,
) -> Result<(), IndexStorageOptimizationError> {
    for database in &source.databases {
        let connection = open_read_only(&database.path)?;
        validate_sqlite_integrity(&connection, &database.name)?;
        validate_fts_membership(&connection, &database.name)?;
        for query in SEARCH_CORPUS {
            let _ = search_result_digest(&connection, query)?;
        }
    }
    Ok(())
}

fn build_staged_database(
    source: &SourceDatabase,
    staged_path: &Path,
) -> Result<(), IndexStorageOptimizationError> {
    migrate_index_database(staged_path, None).map_err(|error| {
        IndexStorageOptimizationError::Validation {
            database: source.name.clone(),
            check: format!("v7 staging initialization: {error}"),
        }
    })?;
    let mut connection = Connection::open_with_flags(
        staged_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI,
    )?;
    connection.busy_timeout(Duration::from_secs(BUSY_TIMEOUT_SECONDS))?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA temp_store = FILE;",
    )?;
    let indexes = explicit_indexes(&connection)?;
    for (name, _) in &indexes {
        connection.execute_batch(&format!("DROP INDEX {}", quote_identifier(name)))?;
    }
    let source_uri = sqlite_read_only_uri(&source.path)?;
    connection.execute("ATTACH DATABASE ?1 AS source", [source_uri.as_str()])?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for table in AUTHORITATIVE_TABLES {
        transaction.execute_batch(&format!(
            "INSERT INTO main.{table_name} ({columns})
             SELECT {columns} FROM source.{table_name}",
            table_name = table.name,
            columns = table.columns,
        ))?;
    }
    {
        let mut rows_statement = transaction.prepare(
            "SELECT
                 articles.article_id,
                 articles.title,
                 articles.abstract_text,
                 articles.doi,
                 articles.pmid,
                 articles.authors_json,
                 journals.title
             FROM source.articles AS articles
             JOIN source.journals AS journals
               ON journals.journal_id = articles.journal_id
             ORDER BY articles.article_id",
        )?;
        let mut rows = rows_statement.query([])?;
        let mut insert = transaction.prepare(
            "INSERT INTO main.article_search (
                 rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
             ) VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        while let Some(row) = rows.next()? {
            let article_id = row.get::<_, i64>(0)?;
            let authors_json = row.get::<_, String>(5)?;
            let authors = decode_article_author_names(&authors_json)
                .map_err(|_| IndexStorageOptimizationError::Validation {
                    database: source.name.clone(),
                    check: "canonical author JSON decoding".to_string(),
                })?
                .join("; ");
            insert.execute(params![
                article_id,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                authors,
                row.get::<_, String>(6)?,
            ])?;
        }
    }
    transaction.commit()?;
    connection.execute_batch("DETACH DATABASE source")?;
    for (_, sql) in indexes {
        connection.execute_batch(&sql)?;
    }
    connection.execute(
        "INSERT INTO article_search(article_search) VALUES('optimize')",
        [],
    )?;
    connection.execute_batch(
        "ANALYZE;
         VACUUM;
         PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = FULL;",
    )?;
    drop(connection);
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(staged_path)?
        .sync_all()?;
    fs::set_permissions(staged_path, fs::metadata(&source.path)?.permissions())?;
    Ok(())
}

fn explicit_indexes(
    connection: &Connection,
) -> Result<Vec<(String, String)>, IndexStorageOptimizationError> {
    let mut statement = connection.prepare(
        "SELECT name, sql FROM sqlite_schema
         WHERE type = 'index' AND sql IS NOT NULL
         ORDER BY name",
    )?;
    let indexes = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(indexes)
}

fn validate_rebuilt_database(
    source: &SourceDatabase,
    candidate_path: &Path,
) -> Result<IndexDatabaseOptimizationReport, IndexStorageOptimizationError> {
    preflight_index_database(candidate_path, None).map_err(|error| {
        IndexStorageOptimizationError::Validation {
            database: source.name.clone(),
            check: format!("v7 exact schema preflight: {error}"),
        }
    })?;
    let connection = open_read_only(candidate_path)?;
    validate_sqlite_integrity(&connection, &source.name)?;
    validate_fts_membership(&connection, &source.name)?;
    let source_uri = sqlite_read_only_uri(&source.path)?;
    connection.execute("ATTACH DATABASE ?1 AS source", [source_uri.as_str()])?;
    connection.execute_batch("PRAGMA query_only = ON")?;

    let mut row_counts = BTreeMap::new();
    for table in AUTHORITATIVE_TABLES {
        let candidate_count = table_count(&connection, "main", table.name)?;
        let source_count = table_count(&connection, "source", table.name)?;
        if candidate_count != source_count {
            return Err(IndexStorageOptimizationError::Validation {
                database: source.name.clone(),
                check: format!(
                    "{table_name} row-count equivalence",
                    table_name = table.name
                ),
            });
        }
        let key_mismatch = connection.query_row(
            &format!(
                "SELECT
                     EXISTS(
                         SELECT {keys} FROM main.{table_name}
                         EXCEPT
                         SELECT {keys} FROM source.{table_name}
                     )
                     OR EXISTS(
                         SELECT {keys} FROM source.{table_name}
                         EXCEPT
                         SELECT {keys} FROM main.{table_name}
                     )",
                keys = table.key_columns,
                table_name = table.name,
            ),
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if key_mismatch {
            return Err(IndexStorageOptimizationError::Validation {
                database: source.name.clone(),
                check: format!("{table_name} key equivalence", table_name = table.name),
            });
        }
        row_counts.insert(table.name.to_string(), candidate_count);
    }
    let fts_count = table_count(&connection, "main", "article_search")?;
    let source_fts_count = table_count(&connection, "source", "article_search")?;
    if fts_count != source_fts_count
        || attached_rowid_mismatch(&connection, "article_search", "article_search")?
    {
        return Err(IndexStorageOptimizationError::Validation {
            database: source.name.clone(),
            check: "FTS rowid equivalence".to_string(),
        });
    }
    row_counts.insert("article_search".to_string(), fts_count);
    drop(connection);

    let source_connection = open_read_only(&source.path)?;
    let candidate_connection = open_read_only(candidate_path)?;
    for query in SEARCH_CORPUS {
        if search_result_digest(&source_connection, query)?
            != search_result_digest(&candidate_connection, query)?
        {
            return Err(IndexStorageOptimizationError::Validation {
                database: source.name.clone(),
                check: "deterministic FTS result equivalence".to_string(),
            });
        }
    }
    let before = measure_database(&source.path)?;
    let after = measure_database(candidate_path)?;
    if after.has_content_shadow {
        return Err(IndexStorageOptimizationError::Validation {
            database: source.name.clone(),
            check: "contentless FTS shadow-table absence".to_string(),
        });
    }
    if after.page_count > 0 && after.freelist_count.saturating_mul(100) > after.page_count {
        return Err(IndexStorageOptimizationError::Validation {
            database: source.name.clone(),
            check: "freelist ratio at or below one percent".to_string(),
        });
    }
    Ok(IndexDatabaseOptimizationReport {
        database: source.name.clone(),
        source_schema_version: source.schema_version,
        target_schema_version: INDEX_SCHEMA_VERSION,
        before,
        after,
        row_counts,
    })
}

fn validate_sqlite_integrity(
    connection: &Connection,
    database: &str,
) -> Result<(), IndexStorageOptimizationError> {
    let quick_check =
        connection.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))?;
    if quick_check != "ok" {
        return Err(IndexStorageOptimizationError::Validation {
            database: database.to_string(),
            check: "SQLite quick_check".to_string(),
        });
    }
    let foreign_key_violations =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })?;
    if foreign_key_violations != 0 {
        return Err(IndexStorageOptimizationError::Validation {
            database: database.to_string(),
            check: "SQLite foreign_key_check".to_string(),
        });
    }
    Ok(())
}

fn validate_fts_membership(
    connection: &Connection,
    database: &str,
) -> Result<(), IndexStorageOptimizationError> {
    let mismatch = connection.query_row(
        "SELECT
             EXISTS(
                 SELECT article_id FROM articles
                 EXCEPT
                 SELECT rowid FROM article_search
             )
             OR EXISTS(
                 SELECT rowid FROM article_search
                 EXCEPT
                 SELECT article_id FROM articles
             )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if mismatch {
        return Err(IndexStorageOptimizationError::Validation {
            database: database.to_string(),
            check: "article-to-FTS rowid membership".to_string(),
        });
    }
    Ok(())
}

fn attached_rowid_mismatch(
    connection: &Connection,
    candidate_table: &str,
    source_table: &str,
) -> Result<bool, IndexStorageOptimizationError> {
    Ok(connection.query_row(
        &format!(
            "SELECT
                 EXISTS(
                     SELECT rowid FROM main.{candidate_table}
                     EXCEPT
                     SELECT rowid FROM source.{source_table}
                 )
                 OR EXISTS(
                     SELECT rowid FROM source.{source_table}
                     EXCEPT
                     SELECT rowid FROM main.{candidate_table}
                 )"
        ),
        [],
        |row| row.get::<_, bool>(0),
    )?)
}

fn table_count(
    connection: &Connection,
    schema: &str,
    table: &str,
) -> Result<u64, IndexStorageOptimizationError> {
    Ok(connection.query_row(
        &format!("SELECT COUNT(*) FROM {schema}.{table}"),
        [],
        |row| row.get::<_, u64>(0),
    )?)
}

fn search_result_digest(
    connection: &Connection,
    query: &str,
) -> Result<(u64, [u8; 32]), IndexStorageOptimizationError> {
    let mut statement = connection.prepare(
        "SELECT rowid FROM article_search
         WHERE article_search MATCH ?1
         ORDER BY rowid",
    )?;
    let mut rows = statement.query([query])?;
    let mut count = 0_u64;
    let mut digest = Sha256::new();
    while let Some(row) = rows.next()? {
        digest.update(row.get::<_, i64>(0)?.to_le_bytes());
        count = count.saturating_add(1);
    }
    Ok((count, digest.finalize().into()))
}

fn measure_database(
    path: &Path,
) -> Result<IndexDatabaseStorageMeasurement, IndexStorageOptimizationError> {
    let connection = open_read_only(path)?;
    let page_size = connection.query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))?;
    let page_count = connection.query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))?;
    let freelist_count =
        connection.query_row("PRAGMA freelist_count", [], |row| row.get::<_, u64>(0))?;
    let fts_bytes = connection.query_row(
        "SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name GLOB 'article_search*'",
        [],
        |row| row.get::<_, u64>(0),
    )?;
    let has_content_shadow = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema
             WHERE type = 'table' AND name = 'article_search_content'
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(IndexDatabaseStorageMeasurement {
        file_bytes: fs::metadata(path)?.len(),
        page_size,
        page_count,
        freelist_count,
        freelist_bytes: page_size.saturating_mul(freelist_count),
        fts_bytes,
        has_content_shadow,
    })
}

fn ensure_source_unchanged(
    before: &IndexDirectorySnapshot,
    after: &IndexDirectorySnapshot,
) -> Result<(), IndexStorageOptimizationError> {
    if before.databases.len() != after.databases.len() {
        return Err(IndexStorageOptimizationError::SourceChanged(
            "index directory inventory".to_string(),
        ));
    }
    for (before_database, after_database) in before.databases.iter().zip(&after.databases) {
        if before_database.name != after_database.name
            || before_database.file_bytes != after_database.file_bytes
            || before_database.modified_at != after_database.modified_at
        {
            return Err(IndexStorageOptimizationError::SourceChanged(
                before_database.name.clone(),
            ));
        }
    }
    Ok(())
}

fn switch_index_directory(
    config: &StorageConfig,
    paths: &IndexStorageRecoveryPaths,
    hook: &mut impl IndexOptimizationHook,
) -> Result<(), IndexStorageOptimizationError> {
    fs::rename(config.index_dir(), &paths.rollback)?;
    if let Err(error) = hook.after_source_rename() {
        restore_source_rename(config, paths, hook, &error)?;
        return Err(error);
    }
    if let Err(error) = fs::rename(&paths.staging, config.index_dir()) {
        let primary = IndexStorageOptimizationError::Io(error);
        restore_source_rename(config, paths, hook, &primary)?;
        return Err(primary);
    }
    Ok(())
}

fn restore_source_rename(
    config: &StorageConfig,
    paths: &IndexStorageRecoveryPaths,
    hook: &mut impl IndexOptimizationHook,
    primary: &IndexStorageOptimizationError,
) -> Result<(), IndexStorageOptimizationError> {
    if let Err(error) = hook.before_rollback() {
        return Err(IndexStorageOptimizationError::RollbackFailed {
            detail: format!("{primary}; rollback hook failed: {error}"),
            recovery_paths: Box::new(paths.clone()),
        });
    }
    fs::rename(&paths.rollback, config.index_dir()).map_err(|rollback_error| {
        IndexStorageOptimizationError::RollbackFailed {
            detail: format!("{primary}; original directory restore failed: {rollback_error}"),
            recovery_paths: Box::new(paths.clone()),
        }
    })
}

fn rollback_switched_index_directory(
    config: &StorageConfig,
    paths: &IndexStorageRecoveryPaths,
    hook: &mut impl IndexOptimizationHook,
    primary: &IndexStorageOptimizationError,
) -> Result<(), IndexStorageOptimizationError> {
    if let Err(error) = hook.before_rollback() {
        return Err(IndexStorageOptimizationError::RollbackFailed {
            detail: format!("{primary}; rollback hook failed: {error}"),
            recovery_paths: Box::new(paths.clone()),
        });
    }
    fs::rename(config.index_dir(), &paths.staging).map_err(|rollback_error| {
        IndexStorageOptimizationError::RollbackFailed {
            detail: format!("{primary}; failed candidate retention failed: {rollback_error}"),
            recovery_paths: Box::new(paths.clone()),
        }
    })?;
    if let Err(rollback_error) = fs::rename(&paths.rollback, config.index_dir()) {
        let candidate_restore = fs::rename(&paths.staging, config.index_dir());
        return Err(IndexStorageOptimizationError::RollbackFailed {
            detail: format!(
                "{primary}; original directory restore failed: {rollback_error}; candidate restore result: {candidate_restore:?}"
            ),
            recovery_paths: Box::new(paths.clone()),
        });
    }
    Ok(())
}

fn remove_known_maintenance_directory(
    path: &Path,
    project_root: &Path,
    expected_name: &str,
) -> Result<(), IndexStorageOptimizationError> {
    if !path_exists(path)? {
        return Ok(());
    }
    let expected_parent = std::path::absolute(project_root.join("data"))?;
    if path.parent() != Some(expected_parent.as_path())
        || path.file_name().and_then(|value| value.to_str()) != Some(expected_name)
    {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "maintenance cleanup path escaped the project data directory".to_string(),
        ));
    }
    let parent_metadata = fs::symlink_metadata(&expected_parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "maintenance cleanup parent must be a regular directory".to_string(),
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(IndexStorageOptimizationError::InvalidLayout(
            "maintenance cleanup target must be a regular directory".to_string(),
        ));
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn open_read_only(path: &Path) -> Result<Connection, IndexStorageOptimizationError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    connection.busy_timeout(Duration::from_secs(BUSY_TIMEOUT_SECONDS))?;
    connection.execute_batch("PRAGMA temp_store = FILE")?;
    Ok(connection)
}

fn sqlite_read_only_uri(path: &Path) -> Result<Url, IndexStorageOptimizationError> {
    let absolute_path = std::path::absolute(path)?;
    let mut uri = Url::from_file_path(&absolute_path).map_err(|()| {
        IndexStorageOptimizationError::InvalidLayout(
            "SQLite source path cannot be represented as a file URI".to_string(),
        )
    })?;
    uri.query_pairs_mut()
        .append_pair("mode", "ro")
        .append_pair("immutable", "1");
    Ok(uri)
}

fn sqlite_table_exists(
    connection: &Connection,
    table: &str,
) -> Result<bool, IndexStorageOptimizationError> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )?)
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn current_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        optimize_index_storage_with_hook, IndexOptimizationHook, IndexStorageOptimizationError,
        IndexStorageOptimizationOptions,
    };
    use crate::{migrate_index_database, open_sqlite_connection, StorageConfig};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FailurePoint {
        Copy,
        Validation,
        SourceRename,
        PostSwitch,
        Rollback,
    }

    struct FailingHook {
        point: FailurePoint,
        should_fail_rollback: bool,
    }

    impl FailingHook {
        fn failure() -> IndexStorageOptimizationError {
            IndexStorageOptimizationError::Validation {
                database: "fixture.sqlite".to_string(),
                check: "injected maintenance failure".to_string(),
            }
        }

        fn storage_full_failure() -> IndexStorageOptimizationError {
            IndexStorageOptimizationError::Io(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "injected staging storage exhaustion",
            ))
        }
    }

    impl IndexOptimizationHook for FailingHook {
        fn before_database_copy(
            &mut self,
            _database: &str,
        ) -> Result<(), IndexStorageOptimizationError> {
            if self.point == FailurePoint::Copy {
                Err(Self::storage_full_failure())
            } else {
                Ok(())
            }
        }

        fn before_staging_validation(&mut self) -> Result<(), IndexStorageOptimizationError> {
            if self.point == FailurePoint::Validation {
                Err(Self::failure())
            } else {
                Ok(())
            }
        }

        fn after_source_rename(&mut self) -> Result<(), IndexStorageOptimizationError> {
            if self.point == FailurePoint::SourceRename {
                Err(Self::failure())
            } else {
                Ok(())
            }
        }

        fn after_switch(&mut self) -> Result<(), IndexStorageOptimizationError> {
            if self.point == FailurePoint::PostSwitch {
                Err(Self::failure())
            } else {
                Ok(())
            }
        }

        fn before_rollback(&mut self) -> Result<(), IndexStorageOptimizationError> {
            if self.should_fail_rollback || self.point == FailurePoint::Rollback {
                Err(Self::failure())
            } else {
                Ok(())
            }
        }
    }

    fn fixture() -> (tempfile::TempDir, StorageConfig, Vec<u8>) {
        let root = tempdir().expect("temporary root should create");
        let config = StorageConfig::from_project_root(root.path());
        fs::create_dir_all(config.index_dir()).expect("index directory should create");
        let path = config.index_dir().join("fixture.sqlite");
        migrate_index_database(&path, None).expect("index should initialize");
        let connection = open_sqlite_connection(&path).expect("index should open");
        connection
            .execute_batch(
                "INSERT INTO journals (
                     journal_id, catalog_id, title, title_aliases_json, issns_json
                 ) VALUES (1, 'fixture', 'Alpha Journal', '[]', '[]');
                 INSERT INTO articles (
                     article_id, journal_id, title, authors_json, in_press
                 ) VALUES (1, 1, 'Genome sequencing', '[\"Alice\"]', 0);
                 INSERT INTO article_listing (
                     article_id, journal_id, in_press
                 ) VALUES (1, 1, 0);
                 INSERT INTO article_search (
                     rowid, article_id, title, authors, journal_title
                 ) VALUES (1, 1, 'Genome sequencing', 'Alice', 'Alpha Journal');",
            )
            .expect("fixture rows should insert");
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")
            .expect("fixture should checkpoint");
        drop(connection);
        let bytes = fs::read(&path).expect("source bytes should read");
        (root, config, bytes)
    }

    #[test]
    fn injected_storage_full_and_validation_failures_leave_source_bytes_present() {
        for point in [FailurePoint::Copy, FailurePoint::Validation] {
            let (_root, config, source_bytes) = fixture();
            let mut hook = FailingHook {
                point,
                should_fail_rollback: false,
            };
            optimize_index_storage_with_hook(
                &IndexStorageOptimizationOptions {
                    storage_config: config.clone(),
                    confirmed: true,
                },
                1_000,
                &mut hook,
            )
            .expect_err("injected failure should stop maintenance");
            assert_eq!(
                fs::read(config.index_dir().join("fixture.sqlite"))
                    .expect("source should remain present"),
                source_bytes
            );
        }
    }

    #[test]
    fn injected_rename_and_post_switch_failures_restore_the_source() {
        for point in [FailurePoint::SourceRename, FailurePoint::PostSwitch] {
            let (_root, config, source_bytes) = fixture();
            let mut hook = FailingHook {
                point,
                should_fail_rollback: false,
            };
            optimize_index_storage_with_hook(
                &IndexStorageOptimizationOptions {
                    storage_config: config.clone(),
                    confirmed: true,
                },
                1_000,
                &mut hook,
            )
            .expect_err("injected failure should stop maintenance");
            assert_eq!(
                fs::read(config.index_dir().join("fixture.sqlite"))
                    .expect("source should be restored"),
                source_bytes
            );
        }
    }

    #[test]
    fn injected_rollback_failure_is_distinct_and_retains_recovery_paths() {
        let (_root, config, _source_bytes) = fixture();
        let mut hook = FailingHook {
            point: FailurePoint::PostSwitch,
            should_fail_rollback: true,
        };
        let error = optimize_index_storage_with_hook(
            &IndexStorageOptimizationOptions {
                storage_config: config,
                confirmed: true,
            },
            1_000,
            &mut hook,
        )
        .expect_err("rollback failure should be surfaced");

        assert_eq!(error.code(), "rollback_failed", "{error:?}");
        assert!(error.recovery_paths().is_some());
    }
}
