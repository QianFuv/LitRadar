//! Disk-backed change-event manifest generation.

use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

/// One provider-neutral canonical content outbox event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentChangeEvent {
    /// Monotonic content database event identifier.
    pub event_id: i64,
    /// Core-owned content revision label.
    pub content_revision: String,
    /// Immutable article identifier.
    pub article_id: i64,
    /// Canonical change kind, `upsert` or `remove`.
    pub change_kind: String,
    /// Immutable journal identifier.
    pub journal_id: i64,
    /// Canonical issue membership when present.
    pub issue_id: Option<i64>,
    /// Whether the event refers to in-press membership.
    pub in_press: bool,
    /// Safe event creation timestamp.
    pub created_at: String,
}

/// Read a bounded page of provider-neutral content outbox events.
///
/// # Arguments
///
/// * `connection` - Open content database connection.
/// * `after_event_id` - Exclusive event cursor.
/// * `limit` - Positive maximum page size.
///
/// # Returns
///
/// Events in monotonic identifier order.
pub fn list_content_change_events(
    connection: &Connection,
    after_event_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<ContentChangeEvent>> {
    if limit == 0 || limit > 10_000 {
        return Err(rusqlite::Error::InvalidParameterName(
            "content change limit must be between 1 and 10000".to_string(),
        ));
    }
    let mut statement = connection.prepare(
        "SELECT
             event_id, content_revision, article_id, change_kind, journal_id,
             issue_id, in_press, created_at
         FROM article_change_events
         WHERE event_id > ?1
         ORDER BY event_id
         LIMIT ?2",
    )?;
    let limit = i64::try_from(limit).expect("bounded content change limit fits i64");
    let events = statement
        .query_map(params![after_event_id, limit], |row| {
            Ok(ContentChangeEvent {
                event_id: row.get(0)?,
                content_revision: row.get(1)?,
                article_id: row.get(2)?,
                change_kind: row.get(3)?,
                journal_id: row.get(4)?,
                issue_id: row.get(5)?,
                in_press: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?
        .collect();
    events
}

/// Acknowledge provider-neutral content events through one inclusive cursor.
///
/// # Arguments
///
/// * `connection` - Open content database connection.
/// * `through_event_id` - Inclusive event identifier to remove after publication.
///
/// # Returns
///
/// Number of acknowledged outbox rows.
pub fn acknowledge_content_change_events(
    connection: &Connection,
    through_event_id: i64,
) -> rusqlite::Result<usize> {
    connection.execute(
        "DELETE FROM article_change_events WHERE event_id <= ?1",
        [through_event_id],
    )
}

/// Errors returned while streaming a change manifest.
#[derive(Debug)]
pub enum ChangeWriteError {
    /// SQLite query or cleanup failed.
    Sqlite(rusqlite::Error),
    /// Manifest file IO failed.
    Io(std::io::Error),
    /// JSON serialization failed.
    Json(serde_json::Error),
}

impl fmt::Display for ChangeWriteError {
    /// Format a change-manifest write error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for ChangeWriteError {
    /// Return the underlying error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
        }
    }
}

impl From<rusqlite::Error> for ChangeWriteError {
    /// Convert SQLite errors into change-write errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for ChangeWriteError {
    /// Convert IO errors into change-write errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ChangeWriteError {
    /// Convert JSON errors into change-write errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Stream one run's normalized change events into an atomic manifest file.
///
/// The output keeps the existing manifest JSON shape while querying ordered
/// event rows directly from SQLite. Successfully published events are deleted
/// only after the temporary file is flushed, synced, and renamed.
///
/// # Arguments
///
/// * `connection` - Open index database connection.
/// * `db_name` - Index database filename.
/// * `run_id` - Completed index run identifier.
/// * `generated_at` - Manifest generation timestamp.
/// * `path` - Final manifest path.
pub fn write_change_manifest_from_events(
    connection: &Connection,
    db_name: &str,
    run_id: &str,
    generated_at: &str,
    path: &Path,
) -> Result<(), ChangeWriteError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = manifest_temp_path(path);
    let mut pending = PendingManifest::new(temp_path.clone());
    let file = File::create(&temp_path)?;
    let mut writer = BufWriter::new(file);
    stream_manifest(&mut writer, connection, db_name, run_id, generated_at)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    fs::rename(&temp_path, path)?;
    pending.did_publish = true;
    connection.execute(
        "DELETE FROM index_change_events WHERE run_id = ?1",
        [run_id],
    )?;
    Ok(())
}

struct PendingManifest {
    path: PathBuf,
    did_publish: bool,
}

impl PendingManifest {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            did_publish: false,
        }
    }
}

impl Drop for PendingManifest {
    fn drop(&mut self) {
        if !self.did_publish {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn manifest_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("changes.json");
    path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()))
}

fn stream_manifest(
    writer: &mut impl Write,
    connection: &Connection,
    db_name: &str,
    run_id: &str,
    generated_at: &str,
) -> Result<(), ChangeWriteError> {
    writer.write_all(b"{\"run_id\":")?;
    write_json(writer, &run_id)?;
    writer.write_all(b",\"generated_at\":")?;
    write_json(writer, &generated_at)?;
    writer.write_all(b",\"db_name\":")?;
    write_json(writer, &db_name)?;
    writer.write_all(b",\"changed_issue_keys\":")?;
    write_issue_keys(writer, connection, run_id, false)?;
    writer.write_all(b",\"changed_inpress_journal_ids\":")?;
    write_inpress_journal_ids(writer, connection, run_id, false)?;
    writer.write_all(b",\"notifiable_article_ids\":")?;
    write_article_ids(writer, connection, run_id, false)?;
    writer.write_all(b",\"backfill_issue_keys\":")?;
    write_issue_keys(writer, connection, run_id, true)?;
    writer.write_all(b",\"backfill_inpress_journal_ids\":")?;
    write_inpress_journal_ids(writer, connection, run_id, true)?;
    writer.write_all(b",\"backfill_article_ids\":")?;
    write_article_ids(writer, connection, run_id, true)?;
    writer.write_all(b",\"summary\":{")?;
    write_summary(writer, connection, run_id)?;
    writer.write_all(b"}}\n")?;
    Ok(())
}

fn write_summary(
    writer: &mut impl Write,
    connection: &Connection,
    run_id: &str,
) -> Result<(), ChangeWriteError> {
    let changed_issue_count = membership_group_count(connection, run_id, "issue", false)?;
    let changed_inpress_count = membership_group_count(connection, run_id, "inpress", false)?;
    let raw_changed_issue_count = changed_issue_count;
    let raw_changed_inpress_count = changed_inpress_count;
    let added_article_count = event_article_count(connection, run_id, "add", false)?;
    let removed_article_count = event_article_count(connection, run_id, "remove", false)?;
    let backfill_article_count = event_article_count(connection, run_id, "add", true)?;

    writer.write_all(b"\"changed_issue_count\":")?;
    write_json(writer, &changed_issue_count)?;
    writer.write_all(b",\"changed_inpress_count\":")?;
    write_json(writer, &changed_inpress_count)?;
    writer.write_all(b",\"added_article_count\":")?;
    write_json(writer, &added_article_count)?;
    writer.write_all(b",\"removed_article_count\":")?;
    write_json(writer, &removed_article_count)?;
    writer.write_all(b",\"issues\":")?;
    write_issue_details(writer, connection, run_id)?;
    writer.write_all(b",\"inpress\":")?;
    write_inpress_details(writer, connection, run_id)?;
    writer.write_all(b",\"raw_changed_issue_count\":")?;
    write_json(writer, &raw_changed_issue_count)?;
    writer.write_all(b",\"raw_changed_inpress_count\":")?;
    write_json(writer, &raw_changed_inpress_count)?;
    writer.write_all(b",\"backfill_article_count\":")?;
    write_json(writer, &backfill_article_count)?;
    writer.write_all(b",\"backfill_issue_keys\":")?;
    write_issue_keys(writer, connection, run_id, true)?;
    writer.write_all(b",\"backfill_inpress_journal_ids\":")?;
    write_inpress_journal_ids(writer, connection, run_id, true)?;
    Ok(())
}

fn write_issue_keys(
    writer: &mut impl Write,
    connection: &Connection,
    run_id: &str,
    backfill_only: bool,
) -> Result<(), ChangeWriteError> {
    let backfill_clause = if backfill_only {
        "AND event_type = 'add' AND is_backfill = 1"
    } else {
        ""
    };
    let sql = format!(
        "
        SELECT journal_id, issue_id
        FROM index_change_events
        WHERE run_id = ?1 AND membership_type = 'issue' {backfill_clause}
        GROUP BY journal_id, issue_id
        ORDER BY CAST(journal_id AS TEXT) || ':' || CAST(issue_id AS TEXT)
        "
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([run_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    writer.write_all(b"[")?;
    let mut is_first = true;
    for row in rows {
        let (journal_id, issue_id) = row?;
        write_separator(writer, &mut is_first)?;
        write_json(writer, &format!("{journal_id}:{issue_id}"))?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_inpress_journal_ids(
    writer: &mut impl Write,
    connection: &Connection,
    run_id: &str,
    backfill_only: bool,
) -> Result<(), ChangeWriteError> {
    let backfill_clause = if backfill_only {
        "AND event_type = 'add' AND is_backfill = 1"
    } else {
        ""
    };
    let sql = format!(
        "
        SELECT journal_id
        FROM index_change_events
        WHERE run_id = ?1 AND membership_type = 'inpress' {backfill_clause}
        GROUP BY journal_id
        ORDER BY journal_id
        "
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([run_id], |row| row.get::<_, i64>(0))?;
    writer.write_all(b"[")?;
    let mut is_first = true;
    for row in rows {
        write_separator(writer, &mut is_first)?;
        write_json(writer, &row?)?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_article_ids(
    writer: &mut impl Write,
    connection: &Connection,
    run_id: &str,
    is_backfill: bool,
) -> Result<(), ChangeWriteError> {
    let mut statement = connection.prepare(
        "
        SELECT article_id
        FROM index_change_events
        WHERE run_id = ?1 AND event_type = 'add' AND is_backfill = ?2
        GROUP BY article_id
        ORDER BY article_id
        ",
    )?;
    let rows = statement.query_map(params![run_id, i64::from(is_backfill)], |row| {
        row.get::<_, i64>(0)
    })?;
    writer.write_all(b"[")?;
    let mut is_first = true;
    for row in rows {
        write_separator(writer, &mut is_first)?;
        write_json(writer, &row?)?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_issue_details(
    writer: &mut impl Write,
    connection: &Connection,
    run_id: &str,
) -> Result<(), ChangeWriteError> {
    let mut statement = connection.prepare(
        "
        SELECT events.journal_id,
               events.issue_id,
               (
                   SELECT COUNT(*)
                   FROM articles
                   WHERE articles.journal_id = events.journal_id
                     AND articles.issue_id = events.issue_id
               ) AS after_count,
               SUM(CASE WHEN events.event_type = 'add' THEN 1 ELSE 0 END),
               SUM(CASE WHEN events.event_type = 'remove' THEN 1 ELSE 0 END)
        FROM index_change_events AS events
        WHERE events.run_id = ?1 AND events.membership_type = 'issue'
        GROUP BY events.journal_id, events.issue_id
        ORDER BY CAST(events.journal_id AS TEXT) || ':' || CAST(events.issue_id AS TEXT)
        ",
    )?;
    let rows = statement.query_map([run_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    writer.write_all(b"[")?;
    let mut is_first = true;
    for row in rows {
        let (journal_id, issue_id, after_count, added_count, removed_count) = row?;
        let before_count = after_count - added_count + removed_count;
        write_separator(writer, &mut is_first)?;
        writer.write_all(b"{\"issue_key\":")?;
        write_json(writer, &format!("{journal_id}:{issue_id}"))?;
        writer.write_all(b",\"before_count\":")?;
        write_json(writer, &before_count.max(0))?;
        writer.write_all(b",\"after_count\":")?;
        write_json(writer, &after_count)?;
        writer.write_all(b"}")?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_inpress_details(
    writer: &mut impl Write,
    connection: &Connection,
    run_id: &str,
) -> Result<(), ChangeWriteError> {
    let mut statement = connection.prepare(
        "
        SELECT events.journal_id,
               (
                   SELECT COUNT(*)
                   FROM articles
                   WHERE articles.journal_id = events.journal_id
                     AND articles.issue_id IS NULL
                     AND COALESCE(articles.in_press, 0) = 1
               ) AS after_count,
               SUM(CASE WHEN events.event_type = 'add' THEN 1 ELSE 0 END),
               SUM(CASE WHEN events.event_type = 'remove' THEN 1 ELSE 0 END)
        FROM index_change_events AS events
        WHERE events.run_id = ?1 AND events.membership_type = 'inpress'
        GROUP BY events.journal_id
        ORDER BY events.journal_id
        ",
    )?;
    let rows = statement.query_map([run_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    writer.write_all(b"[")?;
    let mut is_first = true;
    for row in rows {
        let (journal_id, after_count, added_count, removed_count) = row?;
        let before_count = after_count - added_count + removed_count;
        write_separator(writer, &mut is_first)?;
        writer.write_all(b"{\"journal_id\":")?;
        write_json(writer, &journal_id)?;
        writer.write_all(b",\"before_count\":")?;
        write_json(writer, &before_count.max(0))?;
        writer.write_all(b",\"after_count\":")?;
        write_json(writer, &after_count)?;
        writer.write_all(b"}")?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn membership_group_count(
    connection: &Connection,
    run_id: &str,
    membership_type: &str,
    backfill_only: bool,
) -> rusqlite::Result<i64> {
    let backfill_clause = if backfill_only {
        "AND event_type = 'add' AND is_backfill = 1"
    } else {
        ""
    };
    let group_columns = if membership_type == "issue" {
        "journal_id, issue_id"
    } else {
        "journal_id"
    };
    connection.query_row(
        &format!(
            "
            SELECT COUNT(*)
            FROM (
                SELECT {group_columns}
                FROM index_change_events
                WHERE run_id = ?1 AND membership_type = ?2 {backfill_clause}
                GROUP BY {group_columns}
            )
            "
        ),
        params![run_id, membership_type],
        |row| row.get(0),
    )
}

fn event_article_count(
    connection: &Connection,
    run_id: &str,
    event_type: &str,
    backfill_only: bool,
) -> rusqlite::Result<i64> {
    let backfill_clause = if backfill_only {
        "AND is_backfill = 1"
    } else {
        "AND is_backfill = 0"
    };
    connection.query_row(
        &format!(
            "
            SELECT COUNT(DISTINCT article_id)
            FROM index_change_events
            WHERE run_id = ?1 AND event_type = ?2 {backfill_clause}
            "
        ),
        params![run_id, event_type],
        |row| row.get(0),
    )
}

fn write_separator(writer: &mut impl Write, is_first: &mut bool) -> std::io::Result<()> {
    if *is_first {
        *is_first = false;
    } else {
        writer.write_all(b",")?;
    }
    Ok(())
}

fn write_json(writer: &mut impl Write, value: &impl Serialize) -> Result<(), ChangeWriteError> {
    serde_json::to_writer(writer, value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::schema::{
        apply_article_changes, init_index_db, upsert_articles, with_immediate_index_transaction,
        ChangeEventContext,
    };
    use crate::transforms::ArticleRecord;

    use super::{
        acknowledge_content_change_events, list_content_change_events,
        write_change_manifest_from_events,
    };

    #[test]
    fn pages_and_acknowledges_provider_neutral_content_events() {
        let connection = Connection::open_in_memory().expect("content database should open");
        connection
            .execute_batch(
                "CREATE TABLE article_change_events (
                     event_id INTEGER PRIMARY KEY,
                     content_revision TEXT NOT NULL,
                     article_id INTEGER NOT NULL,
                     change_kind TEXT NOT NULL,
                     journal_id INTEGER NOT NULL,
                     issue_id INTEGER,
                     in_press INTEGER NOT NULL,
                     created_at TEXT NOT NULL
                 );
                 INSERT INTO article_change_events VALUES
                     (1, 'revision-a', 10, 'upsert', 100, 1000, 0, 'first'),
                     (2, 'revision-b', 11, 'remove', 100, NULL, 1, 'second');",
            )
            .expect("content event fixture should initialize");

        let events = list_content_change_events(&connection, 0, 1)
            .expect("first content event page should load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content_revision, "revision-a");
        assert_eq!(events[0].issue_id, Some(1000));
        assert!(!events[0].in_press);
        assert!(list_content_change_events(&connection, 0, 0).is_err());
        assert!(list_content_change_events(&connection, 0, 10_001).is_err());

        assert_eq!(
            acknowledge_content_change_events(&connection, events[0].event_id)
                .expect("first content event should acknowledge"),
            1
        );
        let remaining = list_content_change_events(&connection, events[0].event_id, 10)
            .expect("remaining content events should load");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].change_kind, "remove");
        assert!(remaining[0].in_press);
    }

    #[test]
    fn streams_snapshot_compatible_manifest_and_cleans_published_events() {
        let temp = tempdir().expect("temporary directory should create");
        let manifest_path = temp.path().join("index.changes.json");
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        connection
            .execute_batch(
                "
                INSERT INTO journals (journal_id, library_id) VALUES (1, 'scholarly');
                INSERT INTO journals (journal_id, library_id) VALUES (2, 'scholarly');
                INSERT INTO issues (issue_id, journal_id) VALUES (10, 1);
                INSERT INTO issues (issue_id, journal_id) VALUES (11, 1);
                ",
            )
            .expect("journal and issue parents should insert");
        upsert_articles(
            &connection,
            &[article(1, 1, Some(10), 0), article(2, 2, None, 1)],
        )
        .expect("before articles should insert");
        let context =
            ChangeEventContext::new("run-stream", "worker-0", "2026-07-13T00:00:00Z", false);
        with_immediate_index_transaction(&connection, |transaction| {
            apply_article_changes(
                transaction,
                &[
                    article(1, 1, Some(11), 0),
                    article(3, 1, Some(11), 0),
                    article(4, 2, None, 1),
                ],
                &[2],
                "Journal",
                Some(&context),
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("changes should commit");

        write_change_manifest_from_events(
            &connection,
            "index.sqlite",
            "run-stream",
            "2026-07-13T00:00:00Z",
            &manifest_path,
        )
        .expect("manifest should stream");

        let payload: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&manifest_path).expect("manifest should read"),
        )
        .expect("manifest should parse");
        assert_eq!(
            payload,
            json!({
                "run_id": "run-stream",
                "generated_at": "2026-07-13T00:00:00Z",
                "db_name": "index.sqlite",
                "changed_issue_keys": ["1:10", "1:11"],
                "changed_inpress_journal_ids": [2],
                "notifiable_article_ids": [1, 3, 4],
                "backfill_issue_keys": [],
                "backfill_inpress_journal_ids": [],
                "backfill_article_ids": [],
                "summary": {
                    "changed_issue_count": 2,
                    "changed_inpress_count": 1,
                    "added_article_count": 3,
                    "removed_article_count": 2,
                    "issues": [
                        {"issue_key": "1:10", "before_count": 1, "after_count": 0},
                        {"issue_key": "1:11", "before_count": 0, "after_count": 2}
                    ],
                    "inpress": [
                        {"journal_id": 2, "before_count": 1, "after_count": 1}
                    ],
                    "raw_changed_issue_count": 2,
                    "raw_changed_inpress_count": 1,
                    "backfill_article_count": 0,
                    "backfill_issue_keys": [],
                    "backfill_inpress_journal_ids": []
                }
            })
        );
        let remaining_events: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM index_change_events WHERE run_id = 'run-stream'",
                [],
                |row| row.get(0),
            )
            .expect("event count should query");
        assert_eq!(remaining_events, 0);
    }

    #[test]
    fn streams_large_article_arrays_without_materializing_a_manifest() {
        let temp = tempdir().expect("temporary directory should create");
        let manifest_path = temp.path().join("large.changes.json");
        let mut connection =
            Connection::open(temp.path().join("large.sqlite")).expect("test db should open");
        init_index_db(&connection).expect("schema should initialize");
        connection
            .execute(
                "INSERT INTO journals (journal_id, library_id) VALUES (9, 'scholarly')",
                [],
            )
            .expect("journal parent should insert");
        let transaction = connection
            .transaction()
            .expect("event transaction should start");
        {
            let mut article_statement = transaction
                .prepare(
                    "INSERT INTO articles (article_id, journal_id, in_press) VALUES (?1, 9, 1)",
                )
                .expect("article insert should prepare");
            let mut event_statement = transaction
                .prepare(
                    "
                    INSERT INTO index_change_events (
                        run_id, worker_id, article_id, event_type, membership_type,
                        journal_id, issue_id, is_backfill, created_at
                    ) VALUES ('run-large', 'worker-0', ?1, 'add', 'inpress', 9, NULL, 0, 'now')
                    ",
                )
                .expect("event insert should prepare");
            for article_id in 1..=100_000_i64 {
                article_statement
                    .execute([article_id])
                    .expect("article should insert");
                event_statement
                    .execute([article_id])
                    .expect("event should insert");
            }
        }
        transaction.commit().expect("events should commit");

        write_change_manifest_from_events(
            &connection,
            "large.sqlite",
            "run-large",
            "now",
            &manifest_path,
        )
        .expect("large manifest should stream");

        let payload: serde_json::Value = serde_json::from_reader(
            fs::File::open(&manifest_path).expect("large manifest should open"),
        )
        .expect("large manifest should parse");
        assert_eq!(
            payload["notifiable_article_ids"]
                .as_array()
                .expect("article ids should be an array")
                .len(),
            100_000
        );
        assert_eq!(payload["summary"]["added_article_count"], 100_000);
    }

    fn article(
        article_id: i64,
        journal_id: i64,
        issue_id: Option<i64>,
        in_press: i64,
    ) -> ArticleRecord {
        ArticleRecord {
            article_id,
            journal_id,
            issue_id,
            title: Some(format!("Article {article_id}")),
            date: None,
            authors: Some("Author".to_string()),
            start_page: None,
            end_page: None,
            abstract_text: None,
            doi: None,
            pmid: None,
            permalink: None,
            suppressed: Some(0),
            in_press: Some(in_press),
            open_access: None,
            platform_id: Some(format!("article-{article_id}")),
            retraction_doi: None,
            within_library_holdings: None,
            content_location: None,
            full_text_file: None,
        }
    }
}
