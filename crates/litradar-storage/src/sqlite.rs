//! SQLite connection helpers shared by API, worker, and index code.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, LoadExtensionGuard, OpenFlags};

/// Result of a best-effort SQLite WAL sidecar cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqliteSidecarCleanup {
    /// No WAL or shared-memory sidecar existed when cleanup started.
    NotPresent,
    /// SQLite checkpointed the WAL and removed both sidecars on the final close.
    Cleaned,
    /// Another connection prevented a non-blocking checkpoint.
    Busy,
    /// SQLite completed the checkpoint but retained at least one sidecar.
    Retained,
}

/// Open a SQLite connection with baseline compatibility pragmas.
///
/// # Arguments
///
/// * `path` - SQLite database path.
///
/// # Returns
///
/// Open rusqlite connection.
pub fn open_sqlite_connection(path: impl AsRef<Path>) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(30))?;
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        ",
    )?;
    Ok(connection)
}

/// Ask SQLite to checkpoint and remove idle WAL sidecars without deleting active state.
///
/// Cleanup is non-blocking. The function never removes `-wal` or `-shm` directly; SQLite owns
/// both files and removes them only when this becomes the final connection. Active databases are
/// reported as busy or retained and are left untouched.
///
/// # Arguments
///
/// * `path` - Existing SQLite database path.
///
/// # Returns
///
/// Cleanup outcome or the SQLite failure that prevented a safe attempt.
pub fn cleanup_sqlite_sidecars(path: impl AsRef<Path>) -> rusqlite::Result<SqliteSidecarCleanup> {
    let path = path.as_ref();
    let sidecar_paths = sqlite_sidecar_paths(path);
    if !sidecar_paths.iter().any(|sidecar| sidecar.exists()) {
        return Ok(SqliteSidecarCleanup::NotPresent);
    }

    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::ZERO)?;
    let busy = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    })?;
    connection.close().map_err(|(_, error)| error)?;

    if !sidecar_paths.iter().any(|sidecar| sidecar.exists()) {
        Ok(SqliteSidecarCleanup::Cleaned)
    } else if busy != 0 {
        Ok(SqliteSidecarCleanup::Busy)
    } else {
        Ok(SqliteSidecarCleanup::Retained)
    }
}

fn sqlite_sidecar_paths(path: &Path) -> [PathBuf; 2] {
    [
        sqlite_sidecar_path(path, "-wal"),
        sqlite_sidecar_path(path, "-shm"),
    ]
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(path.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

/// Try to load an optional SQLite extension.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `extension_path` - Optional dynamic extension path.
///
/// # Returns
///
/// True when the extension loaded, or false when no path was configured.
pub fn try_load_extension(
    connection: &Connection,
    extension_path: Option<&Path>,
) -> rusqlite::Result<bool> {
    let Some(path) = extension_path else {
        return Ok(false);
    };
    let _guard = unsafe { LoadExtensionGuard::new(connection)? };
    unsafe { connection.load_extension(path, None::<&str>) }
        .map_err(|error| extension_load_error(path, error))?;
    Ok(true)
}

fn extension_load_error(path: &Path, error: rusqlite::Error) -> rusqlite::Error {
    let detail = error.to_string();
    match error {
        rusqlite::Error::SqliteFailure(code, _) => rusqlite::Error::SqliteFailure(
            code,
            Some(format!(
                "failed to load SQLite extension {}: {detail}",
                path.display()
            )),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::OpenFlags;
    use tempfile::tempdir;
    use tempfile::NamedTempFile;

    use super::{
        cleanup_sqlite_sidecars, open_sqlite_connection, sqlite_sidecar_paths, try_load_extension,
        SqliteSidecarCleanup,
    };

    #[test]
    fn opens_connection_and_executes_queries() {
        let db_file = NamedTempFile::new().expect("database file should be created");
        let connection = open_sqlite_connection(db_file.path()).expect("connection should open");
        let busy_timeout_ms: i64 = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy timeout should be readable");

        connection
            .execute("CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT)", [])
            .expect("table should be created");
        connection
            .execute("INSERT INTO item (name) VALUES (?1)", ["contract"])
            .expect("row should be inserted");
        let name: String = connection
            .query_row("SELECT name FROM item WHERE id = 1", [], |row| row.get(0))
            .expect("row should be queried");

        assert_eq!(busy_timeout_ms, 30_000);
        assert_eq!(name, "contract");
    }

    #[test]
    fn missing_extension_preserves_loader_error() {
        let connection = rusqlite::Connection::open_in_memory().expect("connection should open");
        let error =
            try_load_extension(&connection, Some(std::path::Path::new("missing-extension")))
                .expect_err("missing extension should preserve the loader failure");

        assert!(error.to_string().contains("missing-extension"));
    }

    #[test]
    fn cleanup_removes_sidecars_left_by_an_idle_read_only_connection() {
        let directory = tempdir().expect("temporary directory should be created");
        let database_path = directory.path().join("idle.sqlite");
        let connection = open_sqlite_connection(&database_path).expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE item (id INTEGER PRIMARY KEY);
                 INSERT INTO item DEFAULT VALUES;",
            )
            .expect("fixture data should be written");
        drop(connection);

        let reader =
            rusqlite::Connection::open_with_flags(&database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("read-only connection should open");
        reader
            .query_row("SELECT COUNT(*) FROM item", [], |row| row.get::<_, i64>(0))
            .expect("fixture data should be readable");
        drop(reader);
        let sidecars = sqlite_sidecar_paths(&database_path);
        assert!(sidecars.iter().all(|path| path.exists()));

        assert_eq!(
            cleanup_sqlite_sidecars(&database_path).expect("idle cleanup should succeed"),
            SqliteSidecarCleanup::Cleaned
        );
        assert!(sidecars.iter().all(|path| !path.exists()));

        let verification = rusqlite::Connection::open_with_flags(
            &database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .expect("cleaned database should reopen");
        let count = verification
            .query_row("SELECT COUNT(*) FROM item", [], |row| row.get::<_, i64>(0))
            .expect("checkpointed data should remain readable");
        let journal_mode = verification
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .expect("journal mode should be readable");
        assert_eq!(count, 1);
        assert_eq!(journal_mode, "wal");
        drop(verification);
        assert!(sidecars.iter().all(|path| !path.exists()));
    }

    #[test]
    fn cleanup_leaves_an_active_write_transaction_untouched() {
        let directory = tempdir().expect("temporary directory should be created");
        let database_path = directory.path().join("active.sqlite");
        let connection = open_sqlite_connection(&database_path).expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE item (id INTEGER PRIMARY KEY);
                 BEGIN IMMEDIATE;
                 INSERT INTO item DEFAULT VALUES;",
            )
            .expect("active transaction should start");
        let sidecars = sqlite_sidecar_paths(&database_path);
        assert!(sidecars.iter().all(|path| path.exists()));

        assert_eq!(
            cleanup_sqlite_sidecars(&database_path).expect("busy cleanup should return an outcome"),
            SqliteSidecarCleanup::Busy
        );
        assert!(sidecars.iter().all(|path| path.exists()));

        connection
            .execute_batch("ROLLBACK")
            .expect("fixture transaction should roll back");
        drop(connection);
        assert_eq!(
            cleanup_sqlite_sidecars(&database_path).expect("clean database should be skipped"),
            SqliteSidecarCleanup::NotPresent
        );
        assert!(sidecars.iter().all(|path| !path.exists()));
    }
}
