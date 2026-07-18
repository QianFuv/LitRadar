//! Managed synchronization from an immutable metadata bundle to persistent storage.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;

use crate::{open_sqlite_connection, StorageConfig};

const BUNDLE_FORMAT: &str = "litradar-meta-bundle";
const BUNDLE_MANIFEST_FILENAME: &str = "bundle-manifest.json";
const CATALOG_V2_HEADER: &str = "catalog_id,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating";

/// Action taken for one persistent metadata catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedMetaAction {
    /// A missing persistent catalog was copied from the bundle.
    Created,
    /// A current bundled catalog was accepted without rewriting its bytes.
    Adopted,
    /// A proven managed or official legacy catalog was replaced.
    Updated,
    /// A user-modified or unknown catalog was preserved without changing state.
    Customized,
    /// The persistent catalog and its managed state were already current.
    Unchanged,
}

/// Deterministic preparation result for one bundled catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedMetaCatalogReport {
    /// Safe catalog basename from the validated bundle manifest.
    pub filename: String,
    /// Action taken for the persistent catalog.
    pub action: ManagedMetaAction,
}

/// Structured result returned after preparing every bundled catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedMetaPreparationReport {
    /// Positive immutable bundle version that was prepared.
    pub bundle_version: i64,
    /// Per-catalog results sorted by filename.
    pub catalogs: Vec<ManagedMetaCatalogReport>,
}

/// Errors returned while validating or preparing managed metadata catalogs.
#[derive(Debug)]
pub enum ManagedMetaError {
    /// Filesystem access or replacement failed.
    Io(std::io::Error),
    /// The bundle manifest is not valid JSON.
    Json(serde_json::Error),
    /// SQLite state access or transaction handling failed.
    Sqlite(rusqlite::Error),
    /// The manifest or a declared bundle entry violates the bundle contract.
    InvalidBundle(String),
    /// A bundled file does not match its declared canonical digest.
    BundleHashMismatch {
        /// Catalog basename whose digest did not match.
        filename: String,
        /// Canonical digest declared by the manifest.
        expected: String,
        /// Canonical digest computed from the bundled file.
        actual: String,
    },
    /// Persistent state was written by a newer immutable bundle.
    Downgrade {
        /// Newer bundle version stored in the auth database.
        stored_version: i64,
        /// Older bundle version supplied by the running image.
        bundle_version: i64,
    },
    /// An in-process rollback could not restore every prior target.
    Rollback(String),
}

impl fmt::Display for ManagedMetaError {
    /// Format a preparation error without exposing catalog contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "invalid metadata bundle manifest: {error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::InvalidBundle(message) => {
                write!(formatter, "invalid metadata bundle: {message}")
            }
            Self::BundleHashMismatch {
                filename,
                expected,
                actual,
            } => write!(
                formatter,
                "metadata bundle hash mismatch for {filename}: expected {expected}, found {actual}"
            ),
            Self::Downgrade {
                stored_version,
                bundle_version,
            } => write!(
                formatter,
                "metadata bundle downgrade refused: persistent state uses version {stored_version}, but this image provides version {bundle_version}"
            ),
            Self::Rollback(message) => {
                write!(formatter, "metadata replacement rollback failed: {message}")
            }
        }
    }
}

impl Error for ManagedMetaError {
    /// Return the underlying error when one is available.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::InvalidBundle(_)
            | Self::BundleHashMismatch { .. }
            | Self::Downgrade { .. }
            | Self::Rollback(_) => None,
        }
    }
}

impl From<std::io::Error> for ManagedMetaError {
    /// Convert filesystem failures into metadata preparation errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ManagedMetaError {
    /// Convert JSON failures into metadata preparation errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<rusqlite::Error> for ManagedMetaError {
    /// Convert SQLite failures into metadata preparation errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Prepare persistent metadata catalogs from one validated immutable bundle.
///
/// # Arguments
///
/// * `storage_config` - Persistent storage paths and auth database location.
/// * `bundle_dir` - Immutable directory containing the manifest and official CSV files.
///
/// # Returns
///
/// A deterministic report after all file and managed-state changes commit.
pub fn prepare_managed_meta(
    storage_config: &StorageConfig,
    bundle_dir: impl AsRef<Path>,
) -> Result<ManagedMetaPreparationReport, ManagedMetaError> {
    let mut hook = NoopPreparationHook;
    prepare_managed_meta_with_hook(storage_config, bundle_dir.as_ref(), &mut hook)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    format: String,
    version: i64,
    catalogs: Vec<BundleCatalog>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleCatalog {
    filename: String,
    sha256: String,
    legacy_sha256: Vec<String>,
}

#[derive(Debug)]
struct ValidatedBundle {
    version: i64,
    catalogs: Vec<ValidatedCatalog>,
}

#[derive(Debug)]
struct ValidatedCatalog {
    filename: String,
    sha256: String,
    legacy_sha256: BTreeSet<String>,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct ManagedState {
    bundle_version: i64,
    applied_sha256: String,
}

#[derive(Debug)]
struct CatalogDecision {
    action: ManagedMetaAction,
    should_replace: bool,
    should_store_state: bool,
}

#[derive(Debug)]
enum DestinationState {
    Missing,
    Present(Option<String>),
}

trait PreparationHook {
    fn before_replacement(&mut self, _filename: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn before_state_write(&mut self, _filename: &str) -> std::io::Result<()> {
        Ok(())
    }
}

struct NoopPreparationHook;

impl PreparationHook for NoopPreparationHook {}

fn prepare_managed_meta_with_hook(
    storage_config: &StorageConfig,
    bundle_dir: &Path,
    hook: &mut impl PreparationHook,
) -> Result<ManagedMetaPreparationReport, ManagedMetaError> {
    let bundle = validate_bundle(bundle_dir)?;
    let mut connection = open_sqlite_connection(storage_config.auth_db_path())?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    reject_bundle_downgrade(&transaction, bundle.version)?;
    prepare_target_directory(storage_config.meta_dir())?;

    let mut replacements = Vec::new();
    let catalogs = match apply_catalogs(
        &transaction,
        storage_config.meta_dir(),
        &bundle,
        hook,
        &mut replacements,
    ) {
        Ok(catalogs) => catalogs,
        Err(error) => {
            drop(transaction);
            return Err(rollback_after_error(error, &mut replacements));
        }
    };

    if let Err(error) = transaction.commit() {
        return Err(rollback_after_error(
            ManagedMetaError::Sqlite(error),
            &mut replacements,
        ));
    }
    finish_replacements(&mut replacements)?;

    Ok(ManagedMetaPreparationReport {
        bundle_version: bundle.version,
        catalogs,
    })
}

fn validate_bundle(bundle_dir: &Path) -> Result<ValidatedBundle, ManagedMetaError> {
    let manifest_path = bundle_dir.join(BUNDLE_MANIFEST_FILENAME);
    validate_bundle_file(&manifest_path, BUNDLE_MANIFEST_FILENAME)?;
    let manifest: BundleManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.format != BUNDLE_FORMAT {
        return Err(ManagedMetaError::InvalidBundle(format!(
            "unsupported format {}",
            manifest.format
        )));
    }
    if manifest.version <= 0 {
        return Err(ManagedMetaError::InvalidBundle(
            "version must be a positive integer".to_string(),
        ));
    }
    if manifest.catalogs.is_empty() {
        return Err(ManagedMetaError::InvalidBundle(
            "catalog inventory must not be empty".to_string(),
        ));
    }

    let mut filenames = BTreeSet::new();
    let mut catalogs = Vec::with_capacity(manifest.catalogs.len());
    for catalog in manifest.catalogs {
        validate_catalog_filename(&catalog.filename)?;
        if !filenames.insert(catalog.filename.clone()) {
            return Err(ManagedMetaError::InvalidBundle(format!(
                "duplicate catalog filename {}",
                catalog.filename
            )));
        }
        validate_sha256(&catalog.sha256, &catalog.filename)?;
        let mut legacy_sha256 = BTreeSet::new();
        for digest in catalog.legacy_sha256 {
            validate_sha256(&digest, &catalog.filename)?;
            if digest == catalog.sha256 || !legacy_sha256.insert(digest.clone()) {
                return Err(ManagedMetaError::InvalidBundle(format!(
                    "duplicate current or legacy hash for {}",
                    catalog.filename
                )));
            }
        }

        let catalog_path = bundle_dir.join(&catalog.filename);
        validate_bundle_file(&catalog_path, &catalog.filename)?;
        let bytes = fs::read(&catalog_path)?;
        if manifest.version >= 2 {
            validate_catalog_v2_header(&bytes, &catalog.filename)?;
        }
        let actual = canonical_sha256(&bytes).map_err(|_| {
            ManagedMetaError::InvalidBundle(format!(
                "catalog {} is not valid UTF-8",
                catalog.filename
            ))
        })?;
        if actual != catalog.sha256 {
            return Err(ManagedMetaError::BundleHashMismatch {
                filename: catalog.filename,
                expected: catalog.sha256,
                actual,
            });
        }
        catalogs.push(ValidatedCatalog {
            filename: catalog.filename,
            sha256: actual,
            legacy_sha256,
            bytes,
        });
    }
    catalogs.sort_by(|left, right| left.filename.cmp(&right.filename));
    Ok(ValidatedBundle {
        version: manifest.version,
        catalogs,
    })
}

fn validate_catalog_v2_header(bytes: &[u8], filename: &str) -> Result<(), ManagedMetaError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ManagedMetaError::InvalidBundle(format!("catalog {filename} is not valid UTF-8"))
    })?;
    let header = text
        .lines()
        .next()
        .map(|value| value.trim_end_matches('\r'))
        .unwrap_or_default();
    if header != CATALOG_V2_HEADER {
        return Err(ManagedMetaError::InvalidBundle(format!(
            "catalog {filename} must use the exact canonical v2 header"
        )));
    }
    Ok(())
}

fn validate_bundle_file(path: &Path, filename: &str) -> Result<(), ManagedMetaError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ManagedMetaError::InvalidBundle(format!("missing bundled file {filename}"))
        } else {
            ManagedMetaError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(ManagedMetaError::InvalidBundle(format!(
            "bundled path {filename} must be a regular file"
        )));
    }
    Ok(())
}

fn validate_catalog_filename(filename: &str) -> Result<(), ManagedMetaError> {
    let path = Path::new(filename);
    let is_safe_basename = !filename.is_empty()
        && !filename.contains('/')
        && !filename.contains('\\')
        && path.file_name().and_then(|value| value.to_str()) == Some(filename)
        && path.extension().and_then(|value| value.to_str()) == Some("csv");
    if !is_safe_basename {
        return Err(ManagedMetaError::InvalidBundle(format!(
            "catalog filename {filename:?} must be a portable CSV basename"
        )));
    }
    Ok(())
}

fn validate_sha256(digest: &str, filename: &str) -> Result<(), ManagedMetaError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ManagedMetaError::InvalidBundle(format!(
            "catalog {filename} contains an invalid SHA-256 digest"
        )));
    }
    Ok(())
}

fn canonical_sha256(bytes: &[u8]) -> Result<String, std::str::Utf8Error> {
    let text = std::str::from_utf8(bytes)?;
    let mut normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.ends_with('\n') {
        normalized.push('\n');
    }
    let digest = Sha256::digest(normalized.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn reject_bundle_downgrade(
    transaction: &Transaction<'_>,
    bundle_version: i64,
) -> Result<(), ManagedMetaError> {
    let stored_version: Option<i64> = transaction.query_row(
        "SELECT MAX(bundle_version) FROM managed_meta_catalogs",
        [],
        |row| row.get(0),
    )?;
    if stored_version.is_some_and(|version| version > bundle_version) {
        return Err(ManagedMetaError::Downgrade {
            stored_version: stored_version.expect("newer stored version should exist"),
            bundle_version,
        });
    }
    Ok(())
}

fn prepare_target_directory(meta_dir: &Path) -> Result<(), ManagedMetaError> {
    match fs::symlink_metadata(meta_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ManagedMetaError::InvalidBundle(format!(
                "persistent metadata path {} must be a directory",
                meta_dir.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(meta_dir)?;
            Ok(())
        }
        Err(error) => Err(ManagedMetaError::Io(error)),
    }
}

fn apply_catalogs(
    transaction: &Transaction<'_>,
    meta_dir: &Path,
    bundle: &ValidatedBundle,
    hook: &mut impl PreparationHook,
    replacements: &mut Vec<AppliedReplacement>,
) -> Result<Vec<ManagedMetaCatalogReport>, ManagedMetaError> {
    let mut reports = Vec::with_capacity(bundle.catalogs.len());
    for catalog in &bundle.catalogs {
        let state = load_managed_state(transaction, &catalog.filename)?;
        let target = meta_dir.join(&catalog.filename);
        let destination = load_destination_state(&target)?;
        let decision = classify_catalog(catalog, bundle.version, state.as_ref(), &destination);

        if decision.should_replace {
            hook.before_replacement(&catalog.filename)?;
            replacements.push(AppliedReplacement::apply(&target, &catalog.bytes)?);
        }
        if decision.should_store_state {
            hook.before_state_write(&catalog.filename)?;
            transaction.execute(
                "INSERT INTO managed_meta_catalogs
                    (filename, bundle_version, applied_sha256)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(filename) DO UPDATE SET
                    bundle_version = excluded.bundle_version,
                    applied_sha256 = excluded.applied_sha256",
                params![catalog.filename, bundle.version, catalog.sha256],
            )?;
        }
        reports.push(ManagedMetaCatalogReport {
            filename: catalog.filename.clone(),
            action: decision.action,
        });
    }
    Ok(reports)
}

fn load_managed_state(
    transaction: &Transaction<'_>,
    filename: &str,
) -> Result<Option<ManagedState>, ManagedMetaError> {
    transaction
        .query_row(
            "SELECT bundle_version, applied_sha256
             FROM managed_meta_catalogs WHERE filename = ?1",
            [filename],
            |row| {
                Ok(ManagedState {
                    bundle_version: row.get(0)?,
                    applied_sha256: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(ManagedMetaError::Sqlite)
}

fn load_destination_state(target: &Path) -> Result<DestinationState, ManagedMetaError> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DestinationState::Missing);
        }
        Err(error) => return Err(ManagedMetaError::Io(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(ManagedMetaError::InvalidBundle(format!(
            "persistent catalog {} must be a regular file",
            target.display()
        )));
    }
    let bytes = fs::read(target)?;
    Ok(DestinationState::Present(canonical_sha256(&bytes).ok()))
}

fn classify_catalog(
    catalog: &ValidatedCatalog,
    bundle_version: i64,
    state: Option<&ManagedState>,
    destination: &DestinationState,
) -> CatalogDecision {
    let DestinationState::Present(Some(destination_sha256)) = destination else {
        if matches!(destination, DestinationState::Present(None)) {
            return CatalogDecision {
                action: ManagedMetaAction::Customized,
                should_replace: false,
                should_store_state: false,
            };
        }
        return CatalogDecision {
            action: ManagedMetaAction::Created,
            should_replace: true,
            should_store_state: true,
        };
    };
    if destination_sha256 == catalog.sha256.as_str() {
        let is_current_state = state.is_some_and(|state| {
            state.applied_sha256 == catalog.sha256 && state.bundle_version == bundle_version
        });
        return CatalogDecision {
            action: if is_current_state {
                ManagedMetaAction::Unchanged
            } else {
                ManagedMetaAction::Adopted
            },
            should_replace: false,
            should_store_state: !is_current_state,
        };
    }
    let is_known_official = catalog.legacy_sha256.contains(destination_sha256);
    let is_managed = state.is_some_and(|state| state.applied_sha256.as_str() == destination_sha256);
    if is_known_official || is_managed {
        CatalogDecision {
            action: ManagedMetaAction::Updated,
            should_replace: true,
            should_store_state: true,
        }
    } else {
        CatalogDecision {
            action: ManagedMetaAction::Customized,
            should_replace: false,
            should_store_state: false,
        }
    }
}

#[derive(Debug)]
struct AppliedReplacement {
    target: PathBuf,
    rollback: Option<PathBuf>,
    is_applied: bool,
}

impl AppliedReplacement {
    fn apply(target: &Path, bytes: &[u8]) -> Result<Self, ManagedMetaError> {
        let parent = target.parent().ok_or_else(|| {
            ManagedMetaError::InvalidBundle(
                "persistent catalog has no parent directory".to_string(),
            )
        })?;
        let mut staged = Builder::new()
            .prefix(".litradar-meta-stage-")
            .tempfile_in(parent)?;
        staged.write_all(bytes)?;
        staged.as_file().sync_all()?;

        let rollback_file = Builder::new()
            .prefix(".litradar-meta-rollback-")
            .tempfile_in(parent)?;
        let rollback_path = rollback_file.path().to_path_buf();
        rollback_file.close()?;

        let rollback = match fs::symlink_metadata(target) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                    return Err(ManagedMetaError::InvalidBundle(format!(
                        "persistent catalog {} must be a regular file",
                        target.display()
                    )));
                }
                fs::rename(target, &rollback_path)?;
                Some(rollback_path)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ManagedMetaError::Io(error)),
        };

        if let Err(error) = staged.persist_noclobber(target) {
            if let Some(rollback_path) = rollback.as_ref() {
                fs::rename(rollback_path, target).map_err(|rollback_error| {
                    ManagedMetaError::Rollback(format!(
                        "replacement failed ({}); {} could not be restored ({rollback_error})",
                        error.error,
                        target.display()
                    ))
                })?;
            }
            return Err(ManagedMetaError::Io(error.error));
        }

        Ok(Self {
            target: target.to_path_buf(),
            rollback,
            is_applied: true,
        })
    }

    fn rollback(&mut self) -> Result<(), ManagedMetaError> {
        if !self.is_applied {
            return Ok(());
        }
        match fs::remove_file(&self.target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ManagedMetaError::Io(error)),
        }
        if let Some(rollback) = self.rollback.as_ref() {
            fs::rename(rollback, &self.target)?;
        }
        self.is_applied = false;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), ManagedMetaError> {
        if let Some(rollback) = self.rollback.take() {
            fs::remove_file(rollback)?;
        }
        self.is_applied = false;
        Ok(())
    }
}

fn rollback_after_error(
    error: ManagedMetaError,
    replacements: &mut [AppliedReplacement],
) -> ManagedMetaError {
    match rollback_replacements(replacements) {
        Ok(()) => error,
        Err(rollback_error) => ManagedMetaError::Rollback(format!("{error}; {rollback_error}")),
    }
}

fn rollback_replacements(replacements: &mut [AppliedReplacement]) -> Result<(), ManagedMetaError> {
    let mut failures = Vec::new();
    for replacement in replacements.iter_mut().rev() {
        if let Err(error) = replacement.rollback() {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(ManagedMetaError::Rollback(failures.join("; ")))
    }
}

fn finish_replacements(replacements: &mut [AppliedReplacement]) -> Result<(), ManagedMetaError> {
    for replacement in replacements {
        replacement.finish()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use rusqlite::Connection;
    use serde_json::{json, Value as JsonValue};
    use tempfile::{tempdir, TempDir};

    use super::{
        canonical_sha256, prepare_managed_meta, prepare_managed_meta_with_hook, validate_bundle,
        ManagedMetaAction, ManagedMetaError, ManagedMetaPreparationReport, PreparationHook,
    };
    use crate::{migrate_auth_database, StorageConfig};

    const ALPHA_CURRENT: &[u8] = b"name,value\nalpha,current\n";
    const ALPHA_LEGACY: &[u8] = b"name,value\nalpha,legacy\n";
    const ALPHA_UPDATED: &[u8] = b"catalog_id,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating\nalpha-journal,Alpha Journal,1234-5679,,1234-5679,,,,,,,,,,\n";
    const BETA_CURRENT: &[u8] = b"name,value\nbeta,current\n";
    const BETA_LEGACY: &[u8] = b"name,value\nbeta,legacy\n";

    struct CatalogFixture {
        filename: &'static str,
        current: &'static [u8],
        legacy: Vec<&'static [u8]>,
    }

    struct TestProject {
        root: TempDir,
        storage_config: StorageConfig,
    }

    impl TestProject {
        fn new() -> Self {
            let root = tempdir().expect("temporary project should be created");
            let storage_config = StorageConfig::from_project_root(root.path().join("project"));
            migrate_auth_database(storage_config.auth_db_path())
                .expect("auth database should migrate");
            Self {
                root,
                storage_config,
            }
        }

        fn bundle(
            &self,
            directory_name: &str,
            version: i64,
            catalogs: &[CatalogFixture],
        ) -> PathBuf {
            write_bundle(self.root.path(), directory_name, version, catalogs)
        }
    }

    struct FailingReplacementHook {
        filename: &'static str,
    }

    impl PreparationHook for FailingReplacementHook {
        fn before_replacement(&mut self, filename: &str) -> std::io::Result<()> {
            if filename == self.filename {
                return Err(std::io::Error::other("injected replacement failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn canonical_hash_normalizes_line_endings_and_terminal_lf() {
        let lf = canonical_sha256(b"name,value\nalpha,one\n").expect("LF should hash");
        let crlf = canonical_sha256(b"name,value\r\nalpha,one\r\n").expect("CRLF should hash");
        let cr = canonical_sha256(b"name,value\ralpha,one\r").expect("CR should hash");
        let no_terminal_lf =
            canonical_sha256(b"name,value\nalpha,one").expect("missing terminal LF should hash");
        let changed =
            canonical_sha256(b"name,value\nalpha,two\n").expect("changed field should hash");

        assert_eq!(lf, crlf);
        assert_eq!(lf, cr);
        assert_eq!(lf, no_terminal_lf);
        assert_ne!(lf, changed);
    }

    #[test]
    fn committed_bundle_manifest_matches_every_tracked_catalog() {
        let bundle_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/meta");
        let bundle = validate_bundle(&bundle_dir).expect("committed bundle should validate");
        let manifest_filenames = bundle
            .catalogs
            .iter()
            .map(|catalog| catalog.filename.as_str())
            .collect::<Vec<_>>();
        let mut tracked_filenames = fs::read_dir(&bundle_dir)
            .expect("metadata directory should open")
            .map(|entry| entry.expect("metadata entry should load").path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("csv"))
            .map(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .expect("catalog filename should be UTF-8")
                    .to_string()
            })
            .collect::<Vec<_>>();
        tracked_filenames.sort();

        assert_eq!(bundle.version, 2);
        assert_eq!(manifest_filenames, tracked_filenames);
        let ccf = bundle
            .catalogs
            .iter()
            .find(|catalog| catalog.filename == "ccf_computer_journals.csv")
            .expect("CCF catalog should be declared");
        assert!(ccf
            .legacy_sha256
            .contains("015927e3a57bdc3d99f82348f5af7079475579aa8d282005cfd70fd8113b12ed"));
        assert!(ccf
            .legacy_sha256
            .contains("550bb218f0d71be5e08486ee4a8ebcf1cdefebf076ac222a3583b857fe15e5a9"));
        let chinese = bundle
            .catalogs
            .iter()
            .find(|catalog| catalog.filename == "chinese_journals.csv")
            .expect("Chinese catalog should be declared");
        assert!(chinese
            .legacy_sha256
            .contains("d51d55dd23fd9db71db5be7d7df73e955df6480e7821d90429d3e21f3a3b0807"));
        let english = bundle
            .catalogs
            .iter()
            .find(|catalog| catalog.filename == "english_journals.csv")
            .expect("English catalog should be declared");
        assert!(english
            .legacy_sha256
            .contains("9c99d4c65dffbf1a026c15d1c8684a3b6520bbf601dba8344d71ded20846195d"));
    }

    #[test]
    fn preparation_covers_creation_adoption_upgrade_and_customization() {
        let project = TestProject::new();
        let bundle_v1 = project.bundle(
            "bundle-v1",
            1,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_CURRENT,
                legacy: vec![ALPHA_LEGACY],
            }],
        );

        let created = prepare_managed_meta(&project.storage_config, &bundle_v1)
            .expect("empty target should prepare");
        assert_action(&created, "alpha.csv", ManagedMetaAction::Created);
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("created catalog should read"),
            ALPHA_CURRENT
        );

        fs::write(
            project.storage_config.meta_dir().join("operator.csv"),
            b"operator,data\n",
        )
        .expect("operator catalog should be created");
        let unchanged = prepare_managed_meta(&project.storage_config, &bundle_v1)
            .expect("restart should prepare");
        assert_action(&unchanged, "alpha.csv", ManagedMetaAction::Unchanged);

        delete_managed_state(&project.storage_config, "alpha.csv");
        let current_crlf = String::from_utf8(ALPHA_CURRENT.to_vec())
            .expect("fixture should be UTF-8")
            .replace('\n', "\r\n")
            .into_bytes();
        fs::write(
            project.storage_config.meta_dir().join("alpha.csv"),
            &current_crlf,
        )
        .expect("CRLF catalog should be written");
        let adopted = prepare_managed_meta(&project.storage_config, &bundle_v1)
            .expect("current CRLF target should be adopted");
        assert_action(&adopted, "alpha.csv", ManagedMetaAction::Adopted);
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("adopted catalog should read"),
            current_crlf
        );

        delete_managed_state(&project.storage_config, "alpha.csv");
        fs::write(
            project.storage_config.meta_dir().join("alpha.csv"),
            ALPHA_LEGACY,
        )
        .expect("legacy catalog should be written");
        let upgraded_legacy = prepare_managed_meta(&project.storage_config, &bundle_v1)
            .expect("known legacy target should update");
        assert_action(&upgraded_legacy, "alpha.csv", ManagedMetaAction::Updated);
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("upgraded catalog should read"),
            ALPHA_CURRENT
        );

        let bundle_v2 = project.bundle(
            "bundle-v2",
            2,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_UPDATED,
                legacy: vec![ALPHA_CURRENT],
            }],
        );
        let updated = prepare_managed_meta(&project.storage_config, &bundle_v2)
            .expect("managed target should update to v2");
        assert_action(&updated, "alpha.csv", ManagedMetaAction::Updated);
        assert_eq!(managed_state(&project.storage_config, "alpha.csv").0, 2);

        let customized_bytes = b"name,value\nalpha,operator-change\n";
        fs::write(
            project.storage_config.meta_dir().join("alpha.csv"),
            customized_bytes,
        )
        .expect("customized catalog should be written");
        let customized = prepare_managed_meta(&project.storage_config, &bundle_v2)
            .expect("customized target should not fail preparation");
        assert_action(&customized, "alpha.csv", ManagedMetaAction::Customized);
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("customized catalog should read"),
            customized_bytes
        );
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("operator.csv"))
                .expect("operator catalog should remain"),
            b"operator,data\n"
        );

        fs::write(
            project.storage_config.meta_dir().join("alpha.csv"),
            [0xff, 0xfe, 0xfd],
        )
        .expect("non-UTF-8 catalog should be written");
        let non_utf8 = prepare_managed_meta(&project.storage_config, &bundle_v2)
            .expect("non-UTF-8 customization should be preserved");
        assert_action(&non_utf8, "alpha.csv", ManagedMetaAction::Customized);
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("non-UTF-8 catalog should read"),
            [0xff, 0xfe, 0xfd]
        );
    }

    #[test]
    fn invalid_bundles_fail_before_persistent_directory_writes() {
        let valid_digest = canonical_sha256(ALPHA_CURRENT).expect("fixture should hash");
        let cases = vec![
            json!({
                "format": "unsupported",
                "version": 1,
                "catalogs": []
            }),
            json!({
                "format": "litradar-meta-bundle",
                "version": 0,
                "catalogs": []
            }),
            manifest_with_catalog("../alpha.csv", &valid_digest),
            json!({
                "format": "litradar-meta-bundle",
                "version": 1,
                "catalogs": [
                    catalog_json("alpha.csv", &valid_digest),
                    catalog_json("alpha.csv", &valid_digest)
                ]
            }),
            manifest_with_catalog("alpha.txt", &valid_digest),
            manifest_with_catalog("alpha.csv", "not-a-hash"),
            manifest_with_catalog("missing.csv", &valid_digest),
            manifest_with_catalog(
                "alpha.csv",
                &canonical_sha256(BETA_CURRENT).expect("different fixture should hash"),
            ),
        ];

        for (index, manifest) in cases.into_iter().enumerate() {
            let project = TestProject::new();
            let bundle_dir = project.root.path().join(format!("invalid-bundle-{index}"));
            fs::create_dir_all(&bundle_dir).expect("bundle directory should be created");
            fs::write(bundle_dir.join("alpha.csv"), ALPHA_CURRENT)
                .expect("bundle catalog should be written");
            fs::write(
                bundle_dir.join("bundle-manifest.json"),
                serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
            )
            .expect("manifest should be written");

            prepare_managed_meta(&project.storage_config, &bundle_dir)
                .expect_err("invalid bundle should fail");
            assert!(
                !project.storage_config.meta_dir().exists(),
                "case {index} wrote the persistent metadata directory"
            );
        }
    }

    #[test]
    fn version_two_bundle_rejects_noncanonical_catalog_header() {
        let project = TestProject::new();
        let bundle = project.bundle(
            "invalid-v2-header",
            2,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_CURRENT,
                legacy: Vec::new(),
            }],
        );

        let error = prepare_managed_meta(&project.storage_config, bundle)
            .expect_err("noncanonical v2 catalog should fail");

        assert!(matches!(
            error,
            ManagedMetaError::InvalidBundle(message)
                if message.contains("exact canonical v2 header")
        ));
        assert!(!project.storage_config.meta_dir().exists());
    }

    #[test]
    fn downgrade_fails_before_catalog_writes() {
        let project = TestProject::new();
        let bundle_v2 = project.bundle(
            "bundle-v2",
            2,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_UPDATED,
                legacy: Vec::new(),
            }],
        );
        prepare_managed_meta(&project.storage_config, &bundle_v2)
            .expect("newer bundle should prepare");
        fs::remove_file(project.storage_config.meta_dir().join("alpha.csv"))
            .expect("managed catalog should be removed for the fixture");
        fs::write(
            project.storage_config.meta_dir().join("operator.csv"),
            b"preserve,me\n",
        )
        .expect("operator file should be written");
        let bundle_v1 = project.bundle(
            "bundle-v1",
            1,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_CURRENT,
                legacy: Vec::new(),
            }],
        );

        let error = prepare_managed_meta(&project.storage_config, &bundle_v1)
            .expect_err("older bundle should be rejected");

        assert!(matches!(
            error,
            ManagedMetaError::Downgrade {
                stored_version: 2,
                bundle_version: 1
            }
        ));
        assert!(!project.storage_config.meta_dir().join("alpha.csv").exists());
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("operator.csv"))
                .expect("operator file should remain"),
            b"preserve,me\n"
        );
    }

    #[test]
    fn concurrent_preparations_serialize_and_converge() {
        let project = TestProject::new();
        let bundle = project.bundle(
            "bundle",
            1,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_CURRENT,
                legacy: Vec::new(),
            }],
        );
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let storage_config = project.storage_config.clone();
            let bundle = bundle.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                prepare_managed_meta(&storage_config, &bundle)
                    .expect("concurrent preparation should succeed")
            }));
        }
        let mut actions = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("preparation thread should finish")
                    .catalogs[0]
                    .action
            })
            .collect::<Vec<_>>();
        actions.sort_by_key(|action| match action {
            ManagedMetaAction::Created => 0,
            ManagedMetaAction::Unchanged => 1,
            _ => 2,
        });

        assert_eq!(
            actions,
            [ManagedMetaAction::Created, ManagedMetaAction::Unchanged]
        );
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("converged catalog should read"),
            ALPHA_CURRENT
        );
        assert_eq!(managed_state(&project.storage_config, "alpha.csv").0, 1);
    }

    #[test]
    fn replacement_failure_rolls_back_prior_files_and_state() {
        let project = TestProject::new();
        let bundle = project.bundle(
            "bundle",
            1,
            &[
                CatalogFixture {
                    filename: "alpha.csv",
                    current: ALPHA_CURRENT,
                    legacy: vec![ALPHA_LEGACY],
                },
                CatalogFixture {
                    filename: "beta.csv",
                    current: BETA_CURRENT,
                    legacy: vec![BETA_LEGACY],
                },
            ],
        );
        fs::create_dir_all(project.storage_config.meta_dir())
            .expect("metadata directory should be created");
        fs::write(
            project.storage_config.meta_dir().join("alpha.csv"),
            ALPHA_LEGACY,
        )
        .expect("alpha legacy catalog should be written");
        fs::write(
            project.storage_config.meta_dir().join("beta.csv"),
            BETA_LEGACY,
        )
        .expect("beta legacy catalog should be written");
        let mut hook = FailingReplacementHook {
            filename: "beta.csv",
        };

        prepare_managed_meta_with_hook(&project.storage_config, &bundle, &mut hook)
            .expect_err("injected replacement failure should abort preparation");

        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("alpha catalog should be restored"),
            ALPHA_LEGACY
        );
        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("beta.csv"))
                .expect("beta catalog should be unchanged"),
            BETA_LEGACY
        );
        assert_eq!(managed_state_count(&project.storage_config), 0);
    }

    #[test]
    fn state_write_failure_restores_replaced_catalog() {
        let project = TestProject::new();
        let bundle = project.bundle(
            "bundle",
            1,
            &[CatalogFixture {
                filename: "alpha.csv",
                current: ALPHA_CURRENT,
                legacy: vec![ALPHA_LEGACY],
            }],
        );
        fs::create_dir_all(project.storage_config.meta_dir())
            .expect("metadata directory should be created");
        fs::write(
            project.storage_config.meta_dir().join("alpha.csv"),
            ALPHA_LEGACY,
        )
        .expect("legacy catalog should be written");
        let connection = Connection::open(project.storage_config.auth_db_path())
            .expect("auth database should open");
        connection
            .execute_batch(
                "CREATE TRIGGER reject_managed_meta_state
                 BEFORE INSERT ON managed_meta_catalogs
                 BEGIN
                    SELECT RAISE(FAIL, 'injected state failure');
                 END;",
            )
            .expect("failure trigger should be created");
        drop(connection);

        prepare_managed_meta(&project.storage_config, &bundle)
            .expect_err("state write failure should abort preparation");

        assert_eq!(
            fs::read(project.storage_config.meta_dir().join("alpha.csv"))
                .expect("legacy catalog should be restored"),
            ALPHA_LEGACY
        );
        assert_eq!(managed_state_count(&project.storage_config), 0);
    }

    fn write_bundle(
        root: &Path,
        directory_name: &str,
        version: i64,
        catalogs: &[CatalogFixture],
    ) -> PathBuf {
        let bundle_dir = root.join(directory_name);
        fs::create_dir_all(&bundle_dir).expect("bundle directory should be created");
        let manifest_catalogs = catalogs
            .iter()
            .map(|catalog| {
                fs::write(bundle_dir.join(catalog.filename), catalog.current)
                    .expect("bundled catalog should be written");
                json!({
                    "filename": catalog.filename,
                    "sha256": canonical_sha256(catalog.current)
                        .expect("current catalog should hash"),
                    "legacy_sha256": catalog
                        .legacy
                        .iter()
                        .map(|bytes| canonical_sha256(bytes).expect("legacy catalog should hash"))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let manifest = json!({
            "format": "litradar-meta-bundle",
            "version": version,
            "catalogs": manifest_catalogs
        });
        fs::write(
            bundle_dir.join("bundle-manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should be written");
        bundle_dir
    }

    fn manifest_with_catalog(filename: &str, sha256: &str) -> JsonValue {
        json!({
            "format": "litradar-meta-bundle",
            "version": 1,
            "catalogs": [catalog_json(filename, sha256)]
        })
    }

    fn catalog_json(filename: &str, sha256: &str) -> JsonValue {
        json!({
            "filename": filename,
            "sha256": sha256,
            "legacy_sha256": []
        })
    }

    fn assert_action(
        report: &ManagedMetaPreparationReport,
        filename: &str,
        expected: ManagedMetaAction,
    ) {
        let action = report
            .catalogs
            .iter()
            .find(|catalog| catalog.filename == filename)
            .expect("catalog report should exist")
            .action;
        assert_eq!(action, expected);
    }

    fn delete_managed_state(storage_config: &StorageConfig, filename: &str) {
        Connection::open(storage_config.auth_db_path())
            .expect("auth database should open")
            .execute(
                "DELETE FROM managed_meta_catalogs WHERE filename = ?1",
                [filename],
            )
            .expect("managed state should be deleted");
    }

    fn managed_state(storage_config: &StorageConfig, filename: &str) -> (i64, String) {
        Connection::open(storage_config.auth_db_path())
            .expect("auth database should open")
            .query_row(
                "SELECT bundle_version, applied_sha256
                 FROM managed_meta_catalogs WHERE filename = ?1",
                [filename],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("managed state should exist")
    }

    fn managed_state_count(storage_config: &StorageConfig) -> i64 {
        Connection::open(storage_config.auth_db_path())
            .expect("auth database should open")
            .query_row("SELECT COUNT(*) FROM managed_meta_catalogs", [], |row| {
                row.get(0)
            })
            .expect("managed state count should load")
    }
}
