//! Storage path configuration and database selection.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use litradar_domain::ProviderCatalogInfo;

/// Storage paths derived from a project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageConfig {
    project_root: PathBuf,
    index_dir: PathBuf,
    index_control_dir: PathBuf,
    meta_dir: PathBuf,
    auth_db_path: PathBuf,
}

impl StorageConfig {
    /// Build storage paths from the repository root.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Repository or deployment root path.
    ///
    /// # Returns
    ///
    /// Storage configuration using existing Python data paths.
    pub fn from_project_root(project_root: impl Into<PathBuf>) -> Self {
        let project_root = project_root.into();
        Self {
            index_dir: project_root.join("data").join("index"),
            index_control_dir: project_root.join("data").join("index-control"),
            meta_dir: project_root.join("data").join("meta"),
            auth_db_path: project_root.join("data").join("auth.sqlite"),
            project_root,
        }
    }

    /// Override the auth database path while retaining project-derived data directories.
    ///
    /// # Arguments
    ///
    /// * `auth_db_path` - Explicit auth database selected by a command caller.
    ///
    /// # Returns
    ///
    /// Storage configuration using the explicit auth database path.
    pub fn with_auth_db_path(mut self, auth_db_path: impl Into<PathBuf>) -> Self {
        self.auth_db_path = auth_db_path.into();
        self
    }

    /// Return the configured project root.
    ///
    /// # Returns
    ///
    /// Project root path.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Return the configured index database directory.
    ///
    /// # Returns
    ///
    /// Index database directory path.
    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }

    /// Return the disposable index control database directory.
    ///
    /// # Returns
    ///
    /// Control directory that is never exposed as an index database directory.
    pub fn index_control_dir(&self) -> &Path {
        &self.index_control_dir
    }

    /// Return the configured persistent metadata catalog directory.
    ///
    /// # Returns
    ///
    /// Metadata catalog directory path.
    pub fn meta_dir(&self) -> &Path {
        &self.meta_dir
    }

    /// Return the configured auth database path.
    ///
    /// # Returns
    ///
    /// Auth database path.
    pub fn auth_db_path(&self) -> &Path {
        &self.auth_db_path
    }

    /// Resolve the bundled SQLite `simple` tokenizer for this project root.
    ///
    /// # Returns
    ///
    /// Existing platform-specific extension path, or None when unavailable.
    pub fn simple_tokenizer_path(&self) -> Option<PathBuf> {
        let libs_dir = self.project_root.join("libs");
        if cfg!(windows) {
            Some(
                libs_dir
                    .join("simple-windows")
                    .join("libsimple-windows-x64")
                    .join("simple.dll"),
            )
            .filter(|path| path.exists())
        } else if cfg!(target_os = "linux") {
            Some(
                libs_dir
                    .join("simple-linux")
                    .join("libsimple-linux-ubuntu-latest")
                    .join("libsimple.so"),
            )
            .filter(|path| path.exists())
        } else {
            None
        }
    }

    /// Resolve one index database path with Python-compatible semantics.
    ///
    /// # Arguments
    ///
    /// * `db_name` - Optional database stem or filename.
    ///
    /// # Returns
    ///
    /// Resolved database path.
    pub fn resolve_index_db_path(
        &self,
        db_name: Option<&str>,
    ) -> Result<PathBuf, DatabaseResolutionError> {
        let normalized = db_name.and_then(normalize_database_name);
        if let Some(candidate) = normalized {
            let path = self.index_dir.join(candidate);
            return if path.exists() {
                Ok(path)
            } else {
                Err(DatabaseResolutionError::DatabaseNotFound)
            };
        }

        let sqlite_files = self.list_index_databases()?;
        match sqlite_files.len() {
            0 => Err(DatabaseResolutionError::NoSqliteDatabasesFound),
            1 => Ok(sqlite_files[0].clone()),
            _ => Err(DatabaseResolutionError::MultipleDatabasesFound),
        }
    }

    /// Resolve the canonical catalog stem for one selected index database.
    ///
    /// # Arguments
    ///
    /// * `db_name` - Optional database stem or filename.
    ///
    /// # Returns
    ///
    /// Safe lowercase catalog stem shared with its metadata CSV.
    pub fn resolve_index_catalog_stem(
        &self,
        db_name: Option<&str>,
    ) -> Result<String, DatabaseResolutionError> {
        let path = self.resolve_index_db_path(db_name)?;
        safe_catalog_stem(&path).ok_or(DatabaseResolutionError::InvalidDatabaseName)
    }

    /// List tracked SQLite database files under the index directory.
    ///
    /// # Returns
    ///
    /// Sorted SQLite database paths.
    pub fn list_index_databases(&self) -> Result<Vec<PathBuf>, DatabaseResolutionError> {
        if !self.index_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sqlite_files = Vec::new();
        for entry in fs::read_dir(&self.index_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("sqlite") {
                sqlite_files.push(path);
            }
        }
        sqlite_files.sort();
        Ok(sqlite_files)
    }

    /// Discover safe metadata and content catalogs for administrator configuration.
    ///
    /// # Returns
    ///
    /// Catalogs sorted by canonical stem without exposing filesystem paths.
    pub fn list_provider_catalogs(
        &self,
    ) -> Result<Vec<ProviderCatalogInfo>, DatabaseResolutionError> {
        let mut catalogs = BTreeMap::new();
        collect_catalog_files(&self.meta_dir, "csv", true, &mut catalogs)?;
        collect_catalog_files(&self.index_dir, "sqlite", false, &mut catalogs)?;
        Ok(catalogs.into_values().collect())
    }
}

/// Database selection errors matching existing API detail strings.
#[derive(Debug)]
pub enum DatabaseResolutionError {
    /// No SQLite files exist under the index directory.
    NoSqliteDatabasesFound,
    /// The requested database does not exist.
    DatabaseNotFound,
    /// More than one database exists and no `db` value was provided.
    MultipleDatabasesFound,
    /// The selected database does not have a safe canonical catalog stem.
    InvalidDatabaseName,
    /// Filesystem access failed while reading database files.
    Io(std::io::Error),
}

impl fmt::Display for DatabaseResolutionError {
    /// Format the API-compatible database resolution detail.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSqliteDatabasesFound => formatter.write_str("No SQLite databases found"),
            Self::DatabaseNotFound => formatter.write_str("Database not found"),
            Self::MultipleDatabasesFound => {
                formatter.write_str("Multiple databases found, specify ?db=<name>")
            }
            Self::InvalidDatabaseName => {
                formatter.write_str("Database name is not a safe catalog stem")
            }
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for DatabaseResolutionError {
    /// Return the underlying IO error when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for DatabaseResolutionError {
    /// Convert filesystem errors into database resolution errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

fn normalize_database_name(db_name: &str) -> Option<PathBuf> {
    let trimmed = db_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let filename = Path::new(trimmed)
        .file_name()
        .and_then(|value| value.to_str())?;
    if filename.ends_with(".sqlite") {
        Some(PathBuf::from(filename))
    } else {
        Some(PathBuf::from(format!("{filename}.sqlite")))
    }
}

fn collect_catalog_files(
    directory: &Path,
    extension: &str,
    is_csv: bool,
    catalogs: &mut BTreeMap<String, ProviderCatalogInfo>,
) -> Result<(), DatabaseResolutionError> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some(extension) {
            continue;
        }
        let Some(stem) = safe_catalog_stem(&path) else {
            continue;
        };
        let Some(filename) = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let catalog = catalogs
            .entry(stem.clone())
            .or_insert_with(|| ProviderCatalogInfo {
                stem,
                csv_filename: None,
                database_filename: None,
            });
        if is_csv {
            catalog.csv_filename = Some(filename);
        } else {
            catalog.database_filename = Some(filename);
        }
    }
    Ok(())
}

fn safe_catalog_stem(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    is_runtime_name(stem).then(|| stem.to_string())
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

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::{DatabaseResolutionError, ProviderCatalogInfo, StorageConfig};

    #[test]
    fn explicit_auth_database_keeps_project_data_directories() {
        let config =
            StorageConfig::from_project_root("project-root").with_auth_db_path("state/auth.sqlite");

        assert_eq!(
            config.auth_db_path(),
            std::path::Path::new("state/auth.sqlite")
        );
        assert_eq!(
            config.meta_dir(),
            std::path::Path::new("project-root/data/meta")
        );
        assert_eq!(
            config.index_dir(),
            std::path::Path::new("project-root/data/index")
        );
        assert_eq!(
            config.index_control_dir(),
            std::path::Path::new("project-root/data/index-control")
        );
    }

    #[test]
    fn resolves_single_database_when_name_is_omitted() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        std::fs::create_dir_all(config.index_dir()).expect("index dir should exist");
        let expected_path = config.index_dir().join("contract.sqlite");
        File::create(&expected_path).expect("fixture db should be created");

        let resolved = config
            .resolve_index_db_path(None)
            .expect("single database should resolve");

        assert_eq!(resolved, expected_path);
        assert_eq!(
            config
                .resolve_index_catalog_stem(None)
                .expect("single catalog stem should resolve"),
            "contract"
        );
        assert_eq!(
            config
                .resolve_index_catalog_stem(Some("contract.sqlite"))
                .expect("filename catalog stem should resolve"),
            "contract"
        );
    }

    #[test]
    fn disposable_control_databases_are_not_listed_as_content_indexes() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        std::fs::create_dir_all(config.index_dir()).expect("index dir should exist");
        std::fs::create_dir_all(config.index_control_dir()).expect("control dir should exist");
        let content = config.index_dir().join("catalog.sqlite");
        let control = config.index_control_dir().join("catalog.sqlite");
        File::create(&content).expect("content database should create");
        File::create(&control).expect("control database should create");

        assert_eq!(
            config
                .list_index_databases()
                .expect("content databases should list"),
            vec![content]
        );
        assert_ne!(control.parent(), Some(config.index_dir()));
    }

    #[test]
    fn keeps_python_database_resolution_error_messages() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        std::fs::create_dir_all(config.index_dir()).expect("index dir should exist");
        File::create(config.index_dir().join("alpha.sqlite")).expect("alpha db should be created");
        File::create(config.index_dir().join("beta.sqlite")).expect("beta db should be created");

        let multiple_error = config
            .resolve_index_db_path(None)
            .expect_err("multiple databases should be ambiguous");
        let missing_error = config
            .resolve_index_db_path(Some("../missing"))
            .expect_err("missing database should fail");

        assert!(matches!(
            multiple_error,
            DatabaseResolutionError::MultipleDatabasesFound
        ));
        assert_eq!(
            multiple_error.to_string(),
            "Multiple databases found, specify ?db=<name>"
        );
        assert!(matches!(
            missing_error,
            DatabaseResolutionError::DatabaseNotFound
        ));
        assert_eq!(missing_error.to_string(), "Database not found");
    }

    #[test]
    fn discovers_safe_csv_and_database_catalog_union() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        std::fs::create_dir_all(config.meta_dir()).expect("meta dir should exist");
        std::fs::create_dir_all(config.index_dir()).expect("index dir should exist");
        File::create(config.meta_dir().join("csv_only.csv")).expect("CSV should be created");
        File::create(config.meta_dir().join("paired.csv")).expect("paired CSV should be created");
        File::create(config.meta_dir().join("Unsafe.csv")).expect("unsafe CSV should be created");
        File::create(config.index_dir().join("paired.sqlite"))
            .expect("paired database should be created");
        File::create(config.index_dir().join("database_only.sqlite"))
            .expect("database should be created");

        let catalogs = config
            .list_provider_catalogs()
            .expect("catalogs should be discovered");

        assert_eq!(
            catalogs,
            vec![
                ProviderCatalogInfo {
                    stem: "csv_only".to_string(),
                    csv_filename: Some("csv_only.csv".to_string()),
                    database_filename: None,
                },
                ProviderCatalogInfo {
                    stem: "database_only".to_string(),
                    csv_filename: None,
                    database_filename: Some("database_only.sqlite".to_string()),
                },
                ProviderCatalogInfo {
                    stem: "paired".to_string(),
                    csv_filename: Some("paired.csv".to_string()),
                    database_filename: Some("paired.sqlite".to_string()),
                },
            ]
        );
    }
}
