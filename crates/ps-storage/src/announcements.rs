//! Announcement repository queries for the auth database.

use std::error::Error;
use std::fmt;
use std::path::Path;

use ps_domain::AnnouncementInfo;
use rusqlite::Connection;

/// Repository error for announcement queries.
#[derive(Debug)]
pub enum AnnouncementRepositoryError {
    /// SQLite returned an error while reading announcements.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for AnnouncementRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for AnnouncementRepositoryError {
    /// Return the underlying error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
        }
    }
}

impl From<rusqlite::Error> for AnnouncementRepositoryError {
    /// Convert SQLite errors into repository errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// List enabled announcements using Python-compatible ordering.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the existing auth SQLite database.
///
/// # Returns
///
/// Enabled announcement payloads ordered by priority and recency.
pub fn list_active_announcements(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<AnnouncementInfo>, AnnouncementRepositoryError> {
    let connection = Connection::open(auth_db_path)?;
    let mut statement = connection.prepare(
        "
        SELECT id, title, message, priority, enabled, created_at, updated_at
        FROM announcements
        WHERE enabled = 1
        ORDER BY CASE priority
            WHEN 'high' THEN 0
            WHEN 'normal' THEN 1
            ELSE 2
        END, created_at DESC
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AnnouncementInfo {
            id: row.get(0)?,
            title: row.get(1)?,
            message: row.get(2)?,
            priority: row.get(3)?,
            enabled: row.get::<_, i64>(4)? != 0,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;

    let mut announcements = Vec::new();
    for row in rows {
        announcements.push(row?);
    }
    Ok(announcements)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    use super::list_active_announcements;

    #[test]
    fn lists_enabled_announcements_with_python_ordering() {
        let db_file = NamedTempFile::new().expect("database file should be created");
        let connection = Connection::open(db_file.path()).expect("connection should open");
        connection
            .execute_batch(
                "
                CREATE TABLE announcements (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    message TEXT NOT NULL,
                    priority TEXT NOT NULL DEFAULT 'normal',
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at REAL NOT NULL,
                    updated_at REAL NOT NULL
                );
                INSERT INTO announcements
                    (title, message, priority, enabled, created_at, updated_at)
                VALUES
                    ('Normal newer', 'normal message', 'normal', 1, 20.0, 21.0),
                    ('High older', 'high message', 'high', 1, 10.0, 11.0),
                    ('Low newest', 'low message', 'low', 1, 30.0, 31.0),
                    ('Disabled', 'hidden message', 'high', 0, 40.0, 41.0);
                ",
            )
            .expect("announcements should be inserted");
        drop(connection);

        let announcements =
            list_active_announcements(db_file.path()).expect("announcements should load");
        let titles = announcements
            .iter()
            .map(|announcement| announcement.title.as_str())
            .collect::<Vec<_>>();

        assert_eq!(titles, ["High older", "Normal newer", "Low newest"]);
        assert!(announcements
            .iter()
            .all(|announcement| announcement.enabled));
    }
}
