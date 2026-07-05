//! SQLite connection helpers shared by API, worker, and index code.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, LoadExtensionGuard};

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

/// Try to load an optional SQLite extension.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `extension_path` - Optional dynamic extension path.
///
/// # Returns
///
/// True when the extension loaded, false when no path or load failure occurred.
pub fn try_load_extension(
    connection: &Connection,
    extension_path: Option<&Path>,
) -> rusqlite::Result<bool> {
    let Some(path) = extension_path else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }

    let guard = unsafe { LoadExtensionGuard::new(connection)? };
    let result = unsafe { connection.load_extension(path, None::<&str>) };
    drop(guard);
    match result {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use super::{open_sqlite_connection, try_load_extension};

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
    fn missing_extension_is_a_nonfatal_false_result() {
        let connection = rusqlite::Connection::open_in_memory().expect("connection should open");
        let did_load =
            try_load_extension(&connection, Some(std::path::Path::new("missing-extension")))
                .expect("missing extension should not fail");

        assert!(!did_load);
    }
}
