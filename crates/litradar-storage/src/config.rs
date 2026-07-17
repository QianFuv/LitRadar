//! Storage path configuration and database selection.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::{DatabaseResolutionError, StorageConfig};

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
}
