//! Verified backup creation, inspection, service heartbeats, and offline restore.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::backup::Backup;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;

use crate::migrations::{AUTH_SCHEMA_VERSION, INDEX_SCHEMA_VERSION};
use crate::{open_sqlite_connection, DatabaseResolutionError, StorageConfig};

/// Current on-disk backup manifest format version.
pub const BACKUP_FORMAT_VERSION: u32 = 1;

/// Maximum heartbeat age that prevents an offline restore.
pub const ACTIVE_HEARTBEAT_MAX_AGE_SECONDS: f64 = 90.0;

const BACKUP_FORMAT_NAME: &str = "litradar-backup";
const MANIFEST_FILENAME: &str = "manifest.json";
const AUTH_BACKUP_PATH: &str = "auth.sqlite";
const INDEX_BACKUP_DIR: &str = "index";
const PUSH_STATE_DIRS: [&str; 2] = ["push_state", "folder_push_state"];
const BACKUP_PAGES_PER_STEP: i32 = 128;
const BACKUP_STEP_PAUSE_MILLISECONDS: u64 = 25;

/// Service kinds that publish restore-safety heartbeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// HTTP API process.
    Api,
    /// Scheduler worker process.
    Worker,
}

impl ServiceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Worker => "worker",
        }
    }
}

/// Optional data groups included in one backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSelection {
    /// Whether all discovered index databases were included.
    pub index_databases: bool,
    /// Whether notification and folder delivery state files were included.
    pub push_state: bool,
}

/// Logical kind of one file recorded in the backup manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupComponentKind {
    /// Auth and business SQLite database.
    AuthDatabase,
    /// One journal index SQLite database.
    IndexDatabase,
    /// One notification or folder-delivery state file.
    PushState,
}

/// One integrity-checked file recorded in a backup manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupComponent {
    /// Component kind.
    pub kind: BackupComponentKind,
    /// Portable forward-slash path relative to the backup root.
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// Lowercase SHA-256 digest.
    pub sha256: String,
    /// SQLite user version for database components.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<i64>,
}

/// Versioned manifest stored at the root of every backup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Stable product backup format identifier.
    pub format: String,
    /// Manifest format version.
    pub version: u32,
    /// Unix creation timestamp in seconds.
    pub created_at: f64,
    /// Optional groups included by the operator.
    pub selection: BackupSelection,
    /// Sorted integrity metadata for every included file.
    pub components: Vec<BackupComponent>,
}

/// Inputs for creating one verified backup directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupCreateOptions {
    /// Storage paths used to discover index and push-state data.
    pub storage_config: StorageConfig,
    /// Auth database path, including an explicit CLI override when supplied.
    pub auth_db_path: PathBuf,
    /// New output directory that must not already exist.
    pub output_dir: PathBuf,
    /// Whether to include every discovered index database.
    pub include_index_databases: bool,
    /// Whether to include both delivery state directories.
    pub include_push_state: bool,
}

/// Inputs for restoring a verified backup into an offline target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRestoreOptions {
    /// Target storage paths.
    pub storage_config: StorageConfig,
    /// Target auth database path, including an explicit CLI override when supplied.
    pub auth_db_path: PathBuf,
    /// Existing backup directory to verify and restore.
    pub backup_dir: PathBuf,
}

/// Successful offline restore summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRestoreReport {
    /// Number of manifest files restored.
    pub restored_files: usize,
    /// Number of SQLite databases restored.
    pub restored_databases: usize,
    /// Whether the index directory was replaced from the backup.
    pub restored_index_databases: bool,
    /// Whether both push-state directories were replaced from the backup.
    pub restored_push_state: bool,
}

/// Errors returned by backup, verification, heartbeat, and restore operations.
#[derive(Debug)]
pub enum BackupError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// SQLite backup or validation failed.
    Sqlite(rusqlite::Error),
    /// Manifest JSON could not be read or written.
    Json(serde_json::Error),
    /// Index database discovery failed.
    DatabaseResolution(DatabaseResolutionError),
    /// Operator input is invalid.
    InvalidInput(String),
    /// Manifest structure or component layout is invalid.
    InvalidManifest(String),
    /// Backup format or database schema is newer than this binary supports.
    Unsupported(String),
    /// A component is missing, changed, corrupt, or hash-mismatched.
    Integrity(String),
    /// A recent API or worker heartbeat makes the target active.
    ActiveTarget,
    /// A failed restore could not fully roll back its applied replacements.
    Rollback(String),
}

impl fmt::Display for BackupError {
    /// Format a backup error without exposing database or secret contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::DatabaseResolution(error) => write!(formatter, "{error}"),
            Self::InvalidInput(message) => write!(formatter, "invalid backup input: {message}"),
            Self::InvalidManifest(message) => {
                write!(formatter, "invalid backup manifest: {message}")
            }
            Self::Unsupported(message) => write!(formatter, "unsupported backup: {message}"),
            Self::Integrity(message) => write!(formatter, "backup integrity error: {message}"),
            Self::ActiveTarget => formatter.write_str(
                "restore refused because a recent API or worker heartbeat marks the target active",
            ),
            Self::Rollback(message) => write!(formatter, "restore rollback failed: {message}"),
        }
    }
}

impl Error for BackupError {
    /// Return the underlying filesystem, SQLite, JSON, or discovery error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::DatabaseResolution(error) => Some(error),
            Self::InvalidInput(_)
            | Self::InvalidManifest(_)
            | Self::Unsupported(_)
            | Self::Integrity(_)
            | Self::ActiveTarget
            | Self::Rollback(_) => None,
        }
    }
}

impl From<std::io::Error> for BackupError {
    /// Convert filesystem errors into backup errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for BackupError {
    /// Convert SQLite errors into backup errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for BackupError {
    /// Convert manifest JSON errors into backup errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<DatabaseResolutionError> for BackupError {
    /// Convert index discovery errors into backup errors.
    fn from(error: DatabaseResolutionError) -> Self {
        Self::DatabaseResolution(error)
    }
}

/// Persist a process heartbeat used to guard offline restores.
///
/// # Arguments
///
/// * `auth_db_path` - Current auth database path.
/// * `service` - API or worker service kind.
/// * `instance_id` - Process-lifetime identifier.
/// * `heartbeat_at` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Empty result after the heartbeat is committed.
pub fn record_service_heartbeat(
    auth_db_path: impl AsRef<Path>,
    service: ServiceKind,
    instance_id: &str,
    heartbeat_at: f64,
) -> Result<(), BackupError> {
    if instance_id.trim().is_empty() || !heartbeat_at.is_finite() {
        return Err(BackupError::InvalidInput(
            "service heartbeat values are invalid".to_string(),
        ));
    }
    let connection = open_sqlite_connection(auth_db_path)?;
    connection.execute(
        "INSERT INTO service_heartbeats (service, instance_id, started_at, heartbeat_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(service, instance_id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
        params![service.as_str(), instance_id, heartbeat_at],
    )?;
    connection.execute(
        "DELETE FROM service_heartbeats WHERE heartbeat_at < ?1",
        [heartbeat_at - 604_800.0],
    )?;
    Ok(())
}

/// Delete one service heartbeat during graceful shutdown.
///
/// # Arguments
///
/// * `auth_db_path` - Current auth database path.
/// * `service` - API or worker service kind.
/// * `instance_id` - Process-lifetime identifier.
///
/// # Returns
///
/// Empty result after the heartbeat row is removed.
pub fn delete_service_heartbeat(
    auth_db_path: impl AsRef<Path>,
    service: ServiceKind,
    instance_id: &str,
) -> Result<(), BackupError> {
    let connection = open_sqlite_connection(auth_db_path)?;
    connection.execute(
        "DELETE FROM service_heartbeats WHERE service = ?1 AND instance_id = ?2",
        params![service.as_str(), instance_id],
    )?;
    Ok(())
}

/// Check whether a target has a recent service or legacy worker heartbeat.
///
/// # Arguments
///
/// * `auth_db_path` - Target auth database path.
/// * `current_time` - Current Unix timestamp in seconds.
/// * `maximum_age_seconds` - Maximum heartbeat age considered active.
///
/// # Returns
///
/// True when an API or worker heartbeat is recent enough to block restore.
pub fn has_recent_service_heartbeat(
    auth_db_path: impl AsRef<Path>,
    current_time: f64,
    maximum_age_seconds: f64,
) -> Result<bool, BackupError> {
    let path = auth_db_path.as_ref();
    if !path.exists() {
        return Ok(false);
    }
    if !current_time.is_finite() || !maximum_age_seconds.is_finite() || maximum_age_seconds < 0.0 {
        return Err(BackupError::InvalidInput(
            "heartbeat time window is invalid".to_string(),
        ));
    }
    let connection = open_read_only_connection(path)?;
    let cutoff = current_time - maximum_age_seconds;
    if table_exists(&connection, "service_heartbeats")? {
        let is_active: bool = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM service_heartbeats
                 WHERE service IN ('api', 'worker') AND heartbeat_at >= ?1
             )",
            [cutoff],
            |row| row.get(0),
        )?;
        if is_active {
            return Ok(true);
        }
    }
    if table_exists(&connection, "scheduler_workers")? {
        let is_active: bool = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM scheduler_workers WHERE heartbeat_at >= ?1
             )",
            [cutoff],
            |row| row.get(0),
        )?;
        if is_active {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Create and verify a new backup directory.
///
/// # Arguments
///
/// * `options` - Source paths, output directory, and optional data groups.
///
/// # Returns
///
/// Written and verified manifest.
pub fn create_backup(options: &BackupCreateOptions) -> Result<BackupManifest, BackupError> {
    if options.output_dir.exists() {
        return Err(BackupError::InvalidInput(
            "output directory already exists".to_string(),
        ));
    }
    if !options.auth_db_path.is_file() {
        return Err(BackupError::InvalidInput(
            "auth database does not exist".to_string(),
        ));
    }
    let output_parent = parent_or_current(&options.output_dir);
    fs::create_dir_all(output_parent)?;
    validate_backup_output_outside_sources(options)?;
    let staging = Builder::new()
        .prefix(".litradar-backup-")
        .tempdir_in(output_parent)?;
    let mut components = Vec::new();

    let auth_destination = staging.path().join(AUTH_BACKUP_PATH);
    backup_sqlite_database(&options.auth_db_path, &auth_destination)?;
    components.push(database_component(
        BackupComponentKind::AuthDatabase,
        AUTH_BACKUP_PATH,
        &auth_destination,
    )?);

    if options.include_index_databases {
        let index_destination = staging.path().join(INDEX_BACKUP_DIR);
        fs::create_dir_all(&index_destination)?;
        for source in options.storage_config.list_index_databases()? {
            let filename = source.file_name().ok_or_else(|| {
                BackupError::InvalidInput("index database filename is invalid".to_string())
            })?;
            let destination = index_destination.join(filename);
            backup_sqlite_database(&source, &destination)?;
            let component_path = format!("{INDEX_BACKUP_DIR}/{}", filename.to_string_lossy());
            components.push(database_component(
                BackupComponentKind::IndexDatabase,
                &component_path,
                &destination,
            )?);
        }
    }

    if options.include_push_state {
        for directory in PUSH_STATE_DIRS {
            let source = options
                .storage_config
                .project_root()
                .join("data")
                .join(directory);
            copy_state_directory(&source, staging.path(), directory, &mut components)?;
        }
    }

    components.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = BackupManifest {
        format: BACKUP_FORMAT_NAME.to_string(),
        version: BACKUP_FORMAT_VERSION,
        created_at: unix_time_seconds()?,
        selection: BackupSelection {
            index_databases: options.include_index_databases,
            push_state: options.include_push_state,
        },
        components,
    };
    write_manifest(staging.path(), &manifest)?;
    let manifest = verify_backup(staging.path())?;
    fs::rename(staging.path(), &options.output_dir)?;
    Ok(manifest)
}

/// Verify one backup manifest, component set, hashes, and SQLite integrity.
///
/// # Arguments
///
/// * `backup_dir` - Backup directory containing `manifest.json`.
///
/// # Returns
///
/// Parsed manifest after every integrity check succeeds.
pub fn verify_backup(backup_dir: impl AsRef<Path>) -> Result<BackupManifest, BackupError> {
    let backup_dir = backup_dir.as_ref();
    if !backup_dir.is_dir() {
        return Err(BackupError::InvalidInput(
            "backup directory does not exist".to_string(),
        ));
    }
    let manifest_path = backup_dir.join(MANIFEST_FILENAME);
    let manifest: BackupManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    validate_manifest_header(&manifest)?;

    let mut expected_files = BTreeSet::from([MANIFEST_FILENAME.to_string()]);
    let mut component_paths = BTreeSet::new();
    let mut auth_components = 0_usize;
    for component in &manifest.components {
        let relative_path = parse_manifest_path(&component.path)?;
        validate_component_layout(component, &relative_path, &manifest.selection)?;
        if !component_paths.insert(component.path.clone()) {
            return Err(BackupError::InvalidManifest(
                "component paths must be unique".to_string(),
            ));
        }
        expected_files.insert(component.path.clone());
        if component.kind == BackupComponentKind::AuthDatabase {
            auth_components += 1;
        }
        let absolute_path = backup_dir.join(&relative_path);
        validate_component_file(component, &absolute_path)?;
    }
    if auth_components != 1 {
        return Err(BackupError::InvalidManifest(
            "exactly one auth database component is required".to_string(),
        ));
    }

    let actual_files = snapshot_directory(backup_dir)?
        .into_iter()
        .map(|file| portable_path(&file.relative_path))
        .collect::<BTreeSet<_>>();
    if actual_files != expected_files {
        return Err(BackupError::Integrity(
            "backup contains missing or unlisted files".to_string(),
        ));
    }
    Ok(manifest)
}

/// Verify and atomically restore a backup into an inactive target.
///
/// # Arguments
///
/// * `options` - Backup directory and target storage paths.
///
/// # Returns
///
/// Restore summary after target validation succeeds.
pub fn restore_backup(options: &BackupRestoreOptions) -> Result<BackupRestoreReport, BackupError> {
    restore_backup_at(options, unix_time_seconds()?)
}

fn restore_backup_at(
    options: &BackupRestoreOptions,
    current_time: f64,
) -> Result<BackupRestoreReport, BackupError> {
    let manifest = verify_backup(&options.backup_dir)?;
    if has_recent_service_heartbeat(
        &options.auth_db_path,
        current_time,
        ACTIVE_HEARTBEAT_MAX_AGE_SECONDS,
    )? {
        return Err(BackupError::ActiveTarget);
    }
    validate_backup_outside_targets(options, &manifest.selection)?;

    let auth_parent = parent_or_current(&options.auth_db_path);
    fs::create_dir_all(auth_parent)?;
    let data_dir = options.storage_config.project_root().join("data");
    fs::create_dir_all(&data_dir)?;
    let auth_workspace = Builder::new()
        .prefix(".litradar-auth-restore-")
        .tempdir_in(auth_parent)?;
    let data_workspace = Builder::new()
        .prefix(".litradar-data-restore-")
        .tempdir_in(&data_dir)?;

    let staged_auth = auth_workspace.path().join("staged-auth.sqlite");
    fs::copy(options.backup_dir.join(AUTH_BACKUP_PATH), &staged_auth)?;
    let mut replacements = vec![Replacement::new(
        options.auth_db_path.clone(),
        Some(staged_auth),
        auth_workspace.path().join("rollback-auth.sqlite"),
    )];
    for suffix in ["-wal", "-shm", "-journal"] {
        replacements.push(Replacement::new(
            sqlite_sidecar_path(&options.auth_db_path, suffix),
            None,
            auth_workspace
                .path()
                .join(format!("rollback-auth.sqlite{suffix}")),
        ));
    }

    if manifest.selection.index_databases {
        let staged_index = data_workspace.path().join("staged-index");
        copy_selected_group(
            &options.backup_dir,
            &manifest,
            BackupComponentKind::IndexDatabase,
            INDEX_BACKUP_DIR,
            &staged_index,
        )?;
        replacements.push(Replacement::new(
            options.storage_config.index_dir().to_path_buf(),
            Some(staged_index),
            data_workspace.path().join("rollback-index"),
        ));
    }

    if manifest.selection.push_state {
        for directory in PUSH_STATE_DIRS {
            let staged_state = data_workspace.path().join(format!("staged-{directory}"));
            copy_selected_group(
                &options.backup_dir,
                &manifest,
                BackupComponentKind::PushState,
                directory,
                &staged_state,
            )?;
            replacements.push(Replacement::new(
                data_dir.join(directory),
                Some(staged_state),
                data_workspace.path().join(format!("rollback-{directory}")),
            ));
        }
    }

    if has_recent_service_heartbeat(
        &options.auth_db_path,
        current_time,
        ACTIVE_HEARTBEAT_MAX_AGE_SECONDS,
    )? {
        return Err(BackupError::ActiveTarget);
    }
    apply_replacements(&mut replacements)?;
    if let Err(error) = validate_restored_components(options, &manifest) {
        rollback_replacements(&mut replacements)?;
        return Err(error);
    }

    Ok(BackupRestoreReport {
        restored_files: manifest.components.len(),
        restored_databases: manifest
            .components
            .iter()
            .filter(|component| {
                matches!(
                    component.kind,
                    BackupComponentKind::AuthDatabase | BackupComponentKind::IndexDatabase
                )
            })
            .count(),
        restored_index_databases: manifest.selection.index_databases,
        restored_push_state: manifest.selection.push_state,
    })
}

fn validate_manifest_header(manifest: &BackupManifest) -> Result<(), BackupError> {
    if manifest.format != BACKUP_FORMAT_NAME {
        return Err(BackupError::Unsupported(
            "manifest format identifier is unknown".to_string(),
        ));
    }
    if manifest.version != BACKUP_FORMAT_VERSION {
        return Err(BackupError::Unsupported(format!(
            "manifest version {} is not supported",
            manifest.version
        )));
    }
    if !manifest.created_at.is_finite() || manifest.created_at < 0.0 {
        return Err(BackupError::InvalidManifest(
            "creation timestamp is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_component_layout(
    component: &BackupComponent,
    relative_path: &Path,
    selection: &BackupSelection,
) -> Result<(), BackupError> {
    let portable = portable_path(relative_path);
    if is_key_file(relative_path) {
        return Err(BackupError::InvalidManifest(
            "key files are forbidden in backups".to_string(),
        ));
    }
    match component.kind {
        BackupComponentKind::AuthDatabase => {
            if portable != AUTH_BACKUP_PATH || component.schema_version.is_none() {
                return Err(BackupError::InvalidManifest(
                    "auth database component layout is invalid".to_string(),
                ));
            }
        }
        BackupComponentKind::IndexDatabase => {
            let parent = relative_path.parent().map(portable_path);
            let is_sqlite =
                relative_path.extension().and_then(|value| value.to_str()) == Some("sqlite");
            if !selection.index_databases
                || parent.as_deref() != Some(INDEX_BACKUP_DIR)
                || !is_sqlite
                || component.schema_version.is_none()
            {
                return Err(BackupError::InvalidManifest(
                    "index database component layout is invalid".to_string(),
                ));
            }
        }
        BackupComponentKind::PushState => {
            let first = portable.split('/').next().unwrap_or_default();
            if !selection.push_state
                || !PUSH_STATE_DIRS.contains(&first)
                || component.schema_version.is_some()
                || portable == first
            {
                return Err(BackupError::InvalidManifest(
                    "push-state component layout is invalid".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_component_file(
    component: &BackupComponent,
    absolute_path: &Path,
) -> Result<(), BackupError> {
    let metadata = fs::symlink_metadata(absolute_path)
        .map_err(|_| BackupError::Integrity(format!("component {} is missing", component.path)))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BackupError::Integrity(format!(
            "component {} is not a regular file",
            component.path
        )));
    }
    if metadata.len() != component.size || sha256_file(absolute_path)? != component.sha256 {
        return Err(BackupError::Integrity(format!(
            "component {} size or hash does not match",
            component.path
        )));
    }
    if matches!(
        component.kind,
        BackupComponentKind::AuthDatabase | BackupComponentKind::IndexDatabase
    ) {
        let found_version = validate_sqlite_database(absolute_path)?;
        let expected_version = component.schema_version.ok_or_else(|| {
            BackupError::InvalidManifest("database schema version is missing".to_string())
        })?;
        let supported_version = match component.kind {
            BackupComponentKind::AuthDatabase => AUTH_SCHEMA_VERSION,
            BackupComponentKind::IndexDatabase => INDEX_SCHEMA_VERSION,
            BackupComponentKind::PushState => unreachable!("database kind was checked"),
        };
        if expected_version < 0 || expected_version > supported_version {
            return Err(BackupError::Unsupported(format!(
                "database schema version {expected_version} exceeds supported version {supported_version}"
            )));
        }
        if found_version != expected_version {
            return Err(BackupError::Integrity(format!(
                "component {} schema version does not match",
                component.path
            )));
        }
    }
    Ok(())
}

fn validate_backup_outside_targets(
    options: &BackupRestoreOptions,
    selection: &BackupSelection,
) -> Result<(), BackupError> {
    let backup = fs::canonicalize(&options.backup_dir)?;
    let mut targets = vec![options.auth_db_path.clone()];
    if selection.index_databases {
        targets.push(options.storage_config.index_dir().to_path_buf());
    }
    if selection.push_state {
        for directory in PUSH_STATE_DIRS {
            targets.push(
                options
                    .storage_config
                    .project_root()
                    .join("data")
                    .join(directory),
            );
        }
    }
    for target in targets {
        if let Ok(canonical_target) = fs::canonicalize(target) {
            if backup.starts_with(&canonical_target) {
                return Err(BackupError::InvalidInput(
                    "backup directory must be outside restored targets".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_backup_output_outside_sources(
    options: &BackupCreateOptions,
) -> Result<(), BackupError> {
    let output_parent = fs::canonicalize(parent_or_current(&options.output_dir))?;
    let output_name = options
        .output_dir
        .file_name()
        .ok_or_else(|| BackupError::InvalidInput("output directory name is invalid".to_string()))?;
    let output = output_parent.join(output_name);
    let mut source_directories = Vec::new();
    if options.include_index_databases && options.storage_config.index_dir().exists() {
        source_directories.push(fs::canonicalize(options.storage_config.index_dir())?);
    }
    if options.include_push_state {
        for directory in PUSH_STATE_DIRS {
            let source = options
                .storage_config
                .project_root()
                .join("data")
                .join(directory);
            if source.exists() {
                source_directories.push(fs::canonicalize(source)?);
            }
        }
    }
    if source_directories
        .iter()
        .any(|source| output.starts_with(source))
    {
        return Err(BackupError::InvalidInput(
            "output directory must be outside included source directories".to_string(),
        ));
    }
    Ok(())
}

fn backup_sqlite_database(source: &Path, destination: &Path) -> Result<(), BackupError> {
    let source_metadata = fs::symlink_metadata(source).map_err(|_| {
        BackupError::InvalidInput("SQLite backup source does not exist".to_string())
    })?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err(BackupError::InvalidInput(
            "SQLite backup source is not a regular file".to_string(),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    {
        let source_connection = open_read_only_connection(source)?;
        let mut destination_connection = Connection::open(destination)?;
        let backup = Backup::new(&source_connection, &mut destination_connection)?;
        backup.run_to_completion(
            BACKUP_PAGES_PER_STEP,
            Duration::from_millis(BACKUP_STEP_PAUSE_MILLISECONDS),
            None,
        )?;
        drop(backup);
        destination_connection.execute_batch("PRAGMA journal_mode = DELETE;")?;
    }
    remove_sqlite_sidecars(destination)?;
    validate_sqlite_database(destination)?;
    Ok(())
}

fn database_component(
    kind: BackupComponentKind,
    path: &str,
    absolute_path: &Path,
) -> Result<BackupComponent, BackupError> {
    Ok(BackupComponent {
        kind,
        path: path.to_string(),
        size: fs::metadata(absolute_path)?.len(),
        sha256: sha256_file(absolute_path)?,
        schema_version: Some(validate_sqlite_database(absolute_path)?),
    })
}

fn validate_sqlite_database(path: &Path) -> Result<i64, BackupError> {
    let connection = open_read_only_connection(path)?;
    let quick_check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        return Err(BackupError::Integrity(
            "SQLite quick_check failed".to_string(),
        ));
    }
    Ok(connection.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

fn open_read_only_connection(path: &Path) -> Result<Connection, BackupError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(30))?;
    Ok(connection)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, BackupError> {
    Ok(connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )?)
}

fn write_manifest(backup_dir: &Path, manifest: &BackupManifest) -> Result<(), BackupError> {
    let mut file = File::create(backup_dir.join(MANIFEST_FILENAME))?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotFile {
    relative_path: PathBuf,
    size: u64,
    sha256: String,
}

fn snapshot_directory(root: &Path) -> Result<Vec<SnapshotFile>, BackupError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let root_metadata = fs::symlink_metadata(root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(BackupError::Integrity(
            "backup source directory is not a regular directory".to_string(),
        ));
    }
    let mut files = Vec::new();
    collect_snapshot_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn collect_snapshot_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<SnapshotFile>,
) -> Result<(), BackupError> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(BackupError::Integrity(
                "symbolic links are not allowed in backups".to_string(),
            ));
        }
        if metadata.is_dir() {
            collect_snapshot_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(SnapshotFile {
                relative_path: path
                    .strip_prefix(root)
                    .map_err(|_| {
                        BackupError::Integrity(
                            "backup file escaped its source directory".to_string(),
                        )
                    })?
                    .to_path_buf(),
                size: metadata.len(),
                sha256: sha256_file(&path)?,
            });
        } else {
            return Err(BackupError::Integrity(
                "special files are not allowed in backups".to_string(),
            ));
        }
    }
    Ok(())
}

fn copy_state_directory(
    source: &Path,
    staging_root: &Path,
    manifest_directory: &str,
    components: &mut Vec<BackupComponent>,
) -> Result<(), BackupError> {
    let before = snapshot_directory(source)?;
    let destination_root = staging_root.join(manifest_directory);
    fs::create_dir_all(&destination_root)?;
    for snapshot in &before {
        if is_key_file(&snapshot.relative_path) {
            return Err(BackupError::InvalidInput(
                "push-state directories contain a forbidden key file".to_string(),
            ));
        }
        let source_path = source.join(&snapshot.relative_path);
        let destination = destination_root.join(&snapshot.relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source_path, &destination)?;
        if fs::metadata(&destination)?.len() != snapshot.size
            || sha256_file(&destination)? != snapshot.sha256
        {
            return Err(BackupError::Integrity(
                "push-state copy changed while it was written".to_string(),
            ));
        }
    }
    if snapshot_directory(source)? != before {
        return Err(BackupError::Integrity(
            "push-state files changed during backup".to_string(),
        ));
    }
    for snapshot in before {
        components.push(BackupComponent {
            kind: BackupComponentKind::PushState,
            path: format!(
                "{manifest_directory}/{}",
                portable_path(&snapshot.relative_path)
            ),
            size: snapshot.size,
            sha256: snapshot.sha256,
            schema_version: None,
        });
    }
    Ok(())
}

fn copy_selected_group(
    backup_dir: &Path,
    manifest: &BackupManifest,
    kind: BackupComponentKind,
    manifest_directory: &str,
    destination_root: &Path,
) -> Result<(), BackupError> {
    fs::create_dir_all(destination_root)?;
    for component in manifest
        .components
        .iter()
        .filter(|component| component.kind == kind)
    {
        let relative_path = parse_manifest_path(&component.path)?;
        if !relative_path.starts_with(manifest_directory) {
            continue;
        }
        let group_relative = relative_path
            .strip_prefix(manifest_directory)
            .map_err(|_| {
                BackupError::InvalidManifest("component group path is invalid".to_string())
            })?;
        let destination = destination_root.join(group_relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(backup_dir.join(relative_path), destination)?;
    }
    Ok(())
}

fn validate_restored_components(
    options: &BackupRestoreOptions,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    for component in &manifest.components {
        let relative = parse_manifest_path(&component.path)?;
        let target = match component.kind {
            BackupComponentKind::AuthDatabase => options.auth_db_path.clone(),
            BackupComponentKind::IndexDatabase | BackupComponentKind::PushState => options
                .storage_config
                .project_root()
                .join("data")
                .join(relative),
        };
        validate_component_file(component, &target)?;
    }
    Ok(())
}

#[derive(Debug)]
struct Replacement {
    target: PathBuf,
    staged: Option<PathBuf>,
    rollback: PathBuf,
    had_original: bool,
    is_applied: bool,
}

impl Replacement {
    fn new(target: PathBuf, staged: Option<PathBuf>, rollback: PathBuf) -> Self {
        Self {
            target,
            staged,
            rollback,
            had_original: false,
            is_applied: false,
        }
    }

    fn apply(&mut self) -> Result<(), BackupError> {
        if self.target.exists() {
            let metadata = fs::symlink_metadata(&self.target)?;
            if metadata.file_type().is_symlink() {
                return Err(BackupError::InvalidInput(
                    "restore targets cannot be symbolic links".to_string(),
                ));
            }
            fs::rename(&self.target, &self.rollback)?;
            self.had_original = true;
        }
        if let Some(staged) = self.staged.as_ref() {
            if let Err(error) = fs::rename(staged, &self.target) {
                if self.had_original {
                    fs::rename(&self.rollback, &self.target).map_err(|rollback_error| {
                        BackupError::Rollback(format!(
                            "replacement failed ({error}); original could not be restored ({rollback_error})"
                        ))
                    })?;
                    self.had_original = false;
                }
                return Err(BackupError::Io(error));
            }
        }
        self.is_applied = true;
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), BackupError> {
        if !self.is_applied {
            return Ok(());
        }
        if self.staged.is_some() && self.target.exists() {
            remove_path(&self.target)?;
        }
        if self.had_original {
            fs::rename(&self.rollback, &self.target)?;
        }
        self.is_applied = false;
        self.had_original = false;
        Ok(())
    }
}

fn apply_replacements(replacements: &mut [Replacement]) -> Result<(), BackupError> {
    for index in 0..replacements.len() {
        if let Err(error) = replacements[index].apply() {
            rollback_replacements(&mut replacements[..index])?;
            return Err(error);
        }
    }
    Ok(())
}

fn rollback_replacements(replacements: &mut [Replacement]) -> Result<(), BackupError> {
    let mut failures = Vec::new();
    for replacement in replacements.iter_mut().rev() {
        if let Err(error) = replacement.rollback() {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(BackupError::Rollback(failures.join("; ")))
    }
}

fn remove_path(path: &Path) -> Result<(), BackupError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn parse_manifest_path(value: &str) -> Result<PathBuf, BackupError> {
    if value.is_empty() || value.contains('\\') {
        return Err(BackupError::InvalidManifest(
            "component path is not portable".to_string(),
        ));
    }
    let mut path = PathBuf::new();
    for part in value.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return Err(BackupError::InvalidManifest(
                "component path is unsafe".to_string(),
            ));
        }
        path.push(part);
    }
    if path.is_absolute() {
        return Err(BackupError::InvalidManifest(
            "component path must be relative".to_string(),
        ));
    }
    if path
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(BackupError::InvalidManifest(
            "component path must contain only normal relative segments".to_string(),
        ));
    }
    Ok(path)
}

fn is_key_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("key") || extension.eq_ignore_ascii_case("pem")
        })
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_file(path: &Path) -> Result<String, BackupError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn remove_sqlite_sidecars(path: &Path) -> Result<(), BackupError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = sqlite_sidecar_path(path, suffix);
        if sidecar.exists() {
            fs::remove_file(sidecar)?;
        }
    }
    Ok(())
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn parent_or_current(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn unix_time_seconds() -> Result<f64, BackupError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BackupError::InvalidInput("system time is before the Unix epoch".to_string()))?
        .as_secs_f64())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::Connection;
    use tempfile::{tempdir, TempDir};

    use super::{
        apply_replacements, create_backup, delete_service_heartbeat, has_recent_service_heartbeat,
        parse_manifest_path, record_service_heartbeat, restore_backup_at, verify_backup,
        BackupComponentKind, BackupCreateOptions, BackupError, BackupRestoreOptions, Replacement,
        ServiceKind,
    };
    use crate::{
        migrate_auth_database, migrate_index_database, open_sqlite_connection, StorageConfig,
    };

    #[test]
    fn manifest_paths_accept_only_portable_normal_segments() {
        assert_eq!(
            parse_manifest_path("index/fixture.sqlite").expect("normal relative path should parse"),
            PathBuf::from("index").join("fixture.sqlite")
        );
        for value in [
            "",
            "/absolute",
            "../escape",
            "index/../escape",
            "index\\escape",
            "C:/escape",
            "index//escape",
        ] {
            assert!(parse_manifest_path(value).is_err(), "{value} should fail");
        }
    }

    #[test]
    fn online_backup_round_trip_preserves_wal_rows_and_optional_data() {
        let fixture = BackupFixture::new("round-trip");
        let source_connection = fixture.open_auth_probe("source-row");
        let index_path = fixture.create_index_probe("journal-row");
        fixture.write_push_state("push_state", "alpha.json", "{\"value\":1}");
        fixture.write_push_state("folder_push_state", "beta.json", "{\"value\":2}");
        let secret_dir = fixture.source_root.join("secrets");
        fs::create_dir_all(&secret_dir).expect("secret directory should be created");
        fs::write(secret_dir.join("litradar.key"), [7_u8; 32])
            .expect("secret fixture should be written");

        let manifest = create_backup(&fixture.create_options(true, true))
            .expect("online backup should complete");
        let verified = verify_backup(&fixture.backup_dir).expect("backup should verify");

        assert_eq!(manifest, verified);
        assert_eq!(query_probe_count(&fixture.auth_db_path), 1);
        assert_eq!(query_probe_count(&index_path), 1);
        assert!(manifest
            .components
            .iter()
            .any(|component| component.kind == BackupComponentKind::IndexDatabase));
        assert!(!fixture.backup_dir.join("litradar.key").exists());
        assert!(!fixture.backup_dir.join("secrets").exists());

        let restore_root = fixture.root.path().join("restored");
        let restore_config = StorageConfig::from_project_root(&restore_root);
        fs::create_dir_all(restore_config.index_dir()).expect("stale index directory should exist");
        fs::write(restore_config.index_dir().join("stale.sqlite"), "stale")
            .expect("stale index should write");
        let stale_push_state = restore_root
            .join("data")
            .join("push_state")
            .join("stale.json");
        fs::create_dir_all(
            stale_push_state
                .parent()
                .expect("stale state should have a parent"),
        )
        .expect("stale push-state directory should exist");
        fs::write(&stale_push_state, "stale").expect("stale push state should write");
        let report = restore_backup_at(
            &BackupRestoreOptions {
                auth_db_path: restore_config.auth_db_path().to_path_buf(),
                storage_config: restore_config.clone(),
                backup_dir: fixture.backup_dir.clone(),
            },
            10_000.0,
        )
        .expect("verified backup should restore");

        assert_eq!(report.restored_databases, 2);
        assert!(report.restored_index_databases);
        assert!(report.restored_push_state);
        assert!(!restore_config.index_dir().join("stale.sqlite").exists());
        assert!(!stale_push_state.exists());
        assert_eq!(
            query_probe_value(restore_config.auth_db_path()),
            "source-row"
        );
        assert_eq!(
            query_probe_value(&restore_config.index_dir().join("fixture.sqlite")),
            "journal-row"
        );
        assert_eq!(
            fs::read_to_string(
                restore_root
                    .join("data")
                    .join("push_state")
                    .join("alpha.json")
            )
            .expect("restored push state should read"),
            "{\"value\":1}"
        );
        drop(source_connection);
    }

    #[test]
    fn omitted_optional_groups_leave_target_directories_unchanged() {
        let source = BackupFixture::new("auth-only");
        source.open_auth_probe("backup-row");
        create_backup(&source.create_options(false, false))
            .expect("auth-only backup should complete");

        let target_root = source.root.path().join("auth-only-target");
        let target_config = StorageConfig::from_project_root(&target_root);
        fs::create_dir_all(target_config.index_dir()).expect("target index should exist");
        let kept_index = target_config.index_dir().join("keep.sqlite");
        fs::write(&kept_index, "keep-index").expect("target index should write");
        let kept_state = target_root
            .join("data")
            .join("push_state")
            .join("keep.json");
        fs::create_dir_all(kept_state.parent().expect("state should have a parent"))
            .expect("target state should exist");
        fs::write(&kept_state, "keep-state").expect("target state should write");

        let report = restore_backup_at(
            &BackupRestoreOptions {
                storage_config: target_config.clone(),
                auth_db_path: target_config.auth_db_path().to_path_buf(),
                backup_dir: source.backup_dir.clone(),
            },
            10_000.0,
        )
        .expect("auth-only backup should restore");

        assert!(!report.restored_index_databases);
        assert!(!report.restored_push_state);
        assert_eq!(
            fs::read_to_string(kept_index).expect("kept index should read"),
            "keep-index"
        );
        assert_eq!(
            fs::read_to_string(kept_state).expect("kept state should read"),
            "keep-state"
        );
    }

    #[test]
    fn verification_rejects_corrupt_incomplete_unsupported_and_unlisted_backups() {
        let corrupt = BackupFixture::new("corrupt");
        corrupt.open_auth_probe("row");
        create_backup(&corrupt.create_options(false, false))
            .expect("fixture backup should complete");
        fs::write(corrupt.backup_dir.join("auth.sqlite"), b"not sqlite")
            .expect("database should be corrupted");
        assert!(matches!(
            verify_backup(&corrupt.backup_dir),
            Err(BackupError::Integrity(_))
        ));

        let incomplete = BackupFixture::new("incomplete");
        incomplete.open_auth_probe("row");
        create_backup(&incomplete.create_options(false, false))
            .expect("fixture backup should complete");
        fs::remove_file(incomplete.backup_dir.join("auth.sqlite"))
            .expect("component should be removed");
        assert!(matches!(
            verify_backup(&incomplete.backup_dir),
            Err(BackupError::Integrity(_))
        ));

        let unsupported = BackupFixture::new("unsupported");
        unsupported.open_auth_probe("row");
        create_backup(&unsupported.create_options(false, false))
            .expect("fixture backup should complete");
        let manifest_path = unsupported.backup_dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest should read"))
                .expect("manifest should parse");
        manifest["version"] = serde_json::json!(999);
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("manifest should encode"),
        )
        .expect("manifest should update");
        assert!(matches!(
            verify_backup(&unsupported.backup_dir),
            Err(BackupError::Unsupported(_))
        ));

        let hash_mismatch = BackupFixture::new("hash-mismatch");
        hash_mismatch.open_auth_probe("row");
        create_backup(&hash_mismatch.create_options(false, false))
            .expect("fixture backup should complete");
        let manifest_path = hash_mismatch.backup_dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest should read"))
                .expect("manifest should parse");
        manifest["components"][0]["sha256"] = serde_json::json!("0".repeat(64));
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("manifest should encode"),
        )
        .expect("manifest should update");
        assert!(matches!(
            verify_backup(&hash_mismatch.backup_dir),
            Err(BackupError::Integrity(_))
        ));

        let unlisted = BackupFixture::new("unlisted");
        unlisted.open_auth_probe("row");
        create_backup(&unlisted.create_options(false, false))
            .expect("fixture backup should complete");
        fs::write(unlisted.backup_dir.join("litradar.key"), [9_u8; 32])
            .expect("unlisted key fixture should be written");
        assert!(matches!(
            verify_backup(&unlisted.backup_dir),
            Err(BackupError::Integrity(_))
        ));
    }

    #[test]
    fn creation_rejects_key_files_and_outputs_nested_in_selected_state() {
        let key_file = BackupFixture::new("state-key");
        key_file.open_auth_probe("row");
        key_file.write_push_state("push_state", "forbidden.key", "key material");
        assert!(matches!(
            create_backup(&key_file.create_options(false, true)),
            Err(BackupError::InvalidInput(_))
        ));

        let nested = BackupFixture::new("nested-output");
        nested.open_auth_probe("row");
        nested.write_push_state("push_state", "state.json", "{}");
        let mut options = nested.create_options(false, true);
        options.output_dir = nested
            .source_root
            .join("data")
            .join("push_state")
            .join("backup");
        assert!(matches!(
            create_backup(&options),
            Err(BackupError::InvalidInput(_))
        ));
    }

    #[test]
    fn recent_api_and_worker_heartbeats_block_restore_until_stale_or_deleted() {
        let root = tempdir().expect("temp root should be created");
        let auth_db_path = root.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");

        record_service_heartbeat(&auth_db_path, ServiceKind::Api, "api-a", 1_000.0)
            .expect("API heartbeat should persist");
        assert!(has_recent_service_heartbeat(&auth_db_path, 1_050.0, 90.0)
            .expect("heartbeat should load"));
        delete_service_heartbeat(&auth_db_path, ServiceKind::Api, "api-a")
            .expect("API heartbeat should delete");
        record_service_heartbeat(&auth_db_path, ServiceKind::Worker, "worker-a", 1_000.0)
            .expect("worker heartbeat should persist");
        assert!(has_recent_service_heartbeat(&auth_db_path, 1_050.0, 90.0)
            .expect("heartbeat should load"));
        assert!(!has_recent_service_heartbeat(&auth_db_path, 1_100.0, 90.0)
            .expect("stale heartbeat should load"));
    }

    #[test]
    fn active_target_and_preflight_failures_leave_existing_rows_unchanged() {
        let source = BackupFixture::new("active-source");
        source.open_auth_probe("backup-row");
        create_backup(&source.create_options(false, false))
            .expect("fixture backup should complete");

        let target_root = source.root.path().join("active-target");
        let target_config = StorageConfig::from_project_root(&target_root);
        migrate_auth_database(target_config.auth_db_path())
            .expect("target auth database should migrate");
        write_probe(target_config.auth_db_path(), "target-row");
        record_service_heartbeat(
            target_config.auth_db_path(),
            ServiceKind::Api,
            "api-active",
            2_000.0,
        )
        .expect("active heartbeat should persist");

        let error = restore_backup_at(
            &BackupRestoreOptions {
                storage_config: target_config.clone(),
                auth_db_path: target_config.auth_db_path().to_path_buf(),
                backup_dir: source.backup_dir.clone(),
            },
            2_001.0,
        )
        .expect_err("active target should refuse restore");

        assert!(matches!(error, BackupError::ActiveTarget));
        assert_eq!(
            query_probe_value(target_config.auth_db_path()),
            "target-row"
        );

        delete_service_heartbeat(target_config.auth_db_path(), ServiceKind::Api, "api-active")
            .expect("heartbeat should delete");
        fs::write(source.backup_dir.join("auth.sqlite"), b"corrupt")
            .expect("backup should be corrupted");
        assert!(restore_backup_at(
            &BackupRestoreOptions {
                storage_config: target_config.clone(),
                auth_db_path: target_config.auth_db_path().to_path_buf(),
                backup_dir: source.backup_dir.clone(),
            },
            3_000.0,
        )
        .is_err());
        assert_eq!(
            query_probe_value(target_config.auth_db_path()),
            "target-row"
        );
    }

    #[test]
    fn replacement_failure_rolls_back_already_applied_targets() {
        let root = tempdir().expect("temp root should be created");
        let first_target = root.path().join("first.txt");
        let second_target = root.path().join("second.txt");
        let first_stage = root.path().join("first-stage.txt");
        fs::write(&first_target, "old-first").expect("first target should write");
        fs::write(&second_target, "old-second").expect("second target should write");
        fs::write(&first_stage, "new-first").expect("first stage should write");
        let missing_stage = root.path().join("missing-stage.txt");
        let mut replacements = vec![
            Replacement::new(
                first_target.clone(),
                Some(first_stage),
                root.path().join("first-rollback.txt"),
            ),
            Replacement::new(
                second_target.clone(),
                Some(missing_stage),
                root.path().join("second-rollback.txt"),
            ),
        ];

        assert!(apply_replacements(&mut replacements).is_err());
        assert_eq!(
            fs::read_to_string(first_target).expect("first target should read"),
            "old-first"
        );
        assert_eq!(
            fs::read_to_string(second_target).expect("second target should read"),
            "old-second"
        );
    }

    struct BackupFixture {
        root: TempDir,
        source_root: PathBuf,
        source_config: StorageConfig,
        auth_db_path: PathBuf,
        backup_dir: PathBuf,
    }

    impl BackupFixture {
        fn new(name: &str) -> Self {
            let root = tempdir().expect("temp root should be created");
            let source_root = root.path().join(format!("source-{name}"));
            let source_config = StorageConfig::from_project_root(&source_root);
            let auth_db_path = source_config.auth_db_path().to_path_buf();
            migrate_auth_database(&auth_db_path).expect("auth database should migrate");
            let backup_dir = root.path().join(format!("backup-{name}"));
            Self {
                root,
                source_root,
                source_config,
                auth_db_path,
                backup_dir,
            }
        }

        fn open_auth_probe(&self, value: &str) -> Connection {
            let connection = open_sqlite_connection(&self.auth_db_path)
                .expect("auth fixture connection should open");
            connection
                .execute_batch(
                    "PRAGMA wal_autocheckpoint = 0;
                     CREATE TABLE IF NOT EXISTS backup_probe (
                         id INTEGER PRIMARY KEY,
                         value TEXT NOT NULL
                     );",
                )
                .expect("auth probe table should exist");
            connection
                .execute(
                    "INSERT OR REPLACE INTO backup_probe (id, value) VALUES (1, ?1)",
                    [value],
                )
                .expect("auth probe row should write");
            connection
        }

        fn create_index_probe(&self, value: &str) -> PathBuf {
            let index_path = self.source_config.index_dir().join("fixture.sqlite");
            migrate_index_database(&index_path, None).expect("index database should migrate");
            write_probe(&index_path, value);
            index_path
        }

        fn write_push_state(&self, directory: &str, filename: &str, value: &str) {
            let path = self.source_root.join("data").join(directory).join(filename);
            fs::create_dir_all(path.parent().expect("state path should have a parent"))
                .expect("state directory should exist");
            fs::write(path, value).expect("state fixture should write");
        }

        fn create_options(
            &self,
            include_index_databases: bool,
            include_push_state: bool,
        ) -> BackupCreateOptions {
            BackupCreateOptions {
                storage_config: self.source_config.clone(),
                auth_db_path: self.auth_db_path.clone(),
                output_dir: self.backup_dir.clone(),
                include_index_databases,
                include_push_state,
            }
        }
    }

    fn write_probe(path: &Path, value: &str) {
        let connection = open_sqlite_connection(path).expect("probe database should open");
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS backup_probe (
                     id INTEGER PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .expect("probe table should exist");
        connection
            .execute(
                "INSERT OR REPLACE INTO backup_probe (id, value) VALUES (1, ?1)",
                [value],
            )
            .expect("probe row should write");
    }

    fn query_probe_count(path: &Path) -> i64 {
        Connection::open(path)
            .expect("probe database should open")
            .query_row("SELECT COUNT(*) FROM backup_probe", [], |row| row.get(0))
            .expect("probe count should load")
    }

    fn query_probe_value(path: &Path) -> String {
        Connection::open(path)
            .expect("probe database should open")
            .query_row("SELECT value FROM backup_probe WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("probe value should load")
    }
}
