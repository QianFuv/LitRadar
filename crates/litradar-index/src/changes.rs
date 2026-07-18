//! Provider-neutral content outbox and atomic change-manifest publication.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_json::json;

const OUTBOX_PAGE_SIZE: usize = 1_000;

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

/// Discard every currently committed event after a non-update rebuild.
///
/// # Arguments
///
/// * `connection` - Open content database connection.
///
/// # Returns
///
/// Number of acknowledged rows.
pub fn discard_content_change_events(connection: &Connection) -> rusqlite::Result<usize> {
    connection.execute("DELETE FROM article_change_events", [])
}

/// Errors returned while publishing a content change manifest.
#[derive(Debug)]
pub enum ChangeWriteError {
    /// SQLite query or acknowledgement failed.
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
    /// Convert SQLite errors into manifest failures.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for ChangeWriteError {
    /// Convert filesystem errors into manifest failures.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ChangeWriteError {
    /// Convert JSON errors into manifest failures.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Publish all currently committed content events and acknowledge them atomically after rename.
///
/// # Arguments
///
/// * `connection` - Open provider-neutral content database.
/// * `db_name` - Stable catalog-derived content database filename.
/// * `run_id` - Current core-owned run identifier.
/// * `generated_at` - Safe manifest generation timestamp.
/// * `path` - Final manifest path.
///
/// # Returns
///
/// Number of events included and acknowledged after durable publication.
pub fn write_content_change_manifest(
    connection: &Connection,
    db_name: &str,
    run_id: &str,
    generated_at: &str,
    path: &Path,
) -> Result<usize, ChangeWriteError> {
    let events = read_all_pending_events(connection)?;
    let through_event_id = events.last().map(|event| event.event_id);
    let payload = build_manifest_payload(db_name, run_id, generated_at, &events);
    publish_json_atomically(path, &payload)?;
    if let Some(through_event_id) = through_event_id {
        acknowledge_content_change_events(connection, through_event_id)?;
    }
    Ok(events.len())
}

fn read_all_pending_events(
    connection: &Connection,
) -> Result<Vec<ContentChangeEvent>, ChangeWriteError> {
    let mut events = Vec::new();
    let mut cursor = 0;
    loop {
        let page = list_content_change_events(connection, cursor, OUTBOX_PAGE_SIZE)?;
        let Some(last) = page.last() else {
            break;
        };
        cursor = last.event_id;
        let is_last = page.len() < OUTBOX_PAGE_SIZE;
        events.extend(page);
        if is_last {
            break;
        }
    }
    Ok(events)
}

fn build_manifest_payload(
    db_name: &str,
    run_id: &str,
    generated_at: &str,
    events: &[ContentChangeEvent],
) -> serde_json::Value {
    let mut issue_keys = BTreeSet::new();
    let mut inpress_journal_ids = BTreeSet::new();
    let mut notifiable_article_ids = BTreeSet::new();
    let mut added_article_ids = BTreeSet::new();
    let mut removed_article_ids = BTreeSet::new();
    for event in events {
        if event.in_press {
            inpress_journal_ids.insert(event.journal_id);
        } else if let Some(issue_id) = event.issue_id {
            issue_keys.insert(format!("{}:{issue_id}", event.journal_id));
        }
        if event.change_kind == "upsert" {
            added_article_ids.insert(event.article_id);
            notifiable_article_ids.insert(event.article_id);
        } else if event.change_kind == "remove" {
            removed_article_ids.insert(event.article_id);
        }
    }
    json!({
        "run_id": run_id,
        "generated_at": generated_at,
        "db_name": db_name,
        "changed_issue_keys": issue_keys,
        "changed_inpress_journal_ids": inpress_journal_ids,
        "notifiable_article_ids": notifiable_article_ids,
        "backfill_issue_keys": [],
        "backfill_inpress_journal_ids": [],
        "backfill_article_ids": [],
        "summary": {
            "changed_issue_count": issue_keys.len(),
            "changed_inpress_count": inpress_journal_ids.len(),
            "added_article_count": added_article_ids.len(),
            "removed_article_count": removed_article_ids.len(),
            "added_article_ids": added_article_ids,
            "removed_article_ids": removed_article_ids,
            "issues": [],
            "inpress": [],
            "raw_changed_issue_count": issue_keys.len(),
            "raw_changed_inpress_count": inpress_journal_ids.len(),
            "backfill_article_count": 0,
            "backfill_issue_keys": [],
            "backfill_inpress_journal_ids": []
        }
    })
}

fn publish_json_atomically(
    path: &Path,
    payload: &serde_json::Value,
) -> Result<(), ChangeWriteError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = manifest_temp_path(path);
    let mut pending = PendingManifest::new(temp_path.clone());
    let file = File::create(&temp_path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, payload)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    fs::rename(&temp_path, path)?;
    pending.did_publish = true;
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

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    use super::{list_content_change_events, write_content_change_manifest, ContentChangeEvent};
    use crate::schema::init_content_db;

    fn insert_event(connection: &Connection, event: &ContentChangeEvent) {
        connection
            .execute(
                "INSERT INTO article_change_events (
                     event_id, content_revision, article_id, change_kind, journal_id,
                     issue_id, in_press, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    event.event_id,
                    event.content_revision,
                    event.article_id,
                    event.change_kind,
                    event.journal_id,
                    event.issue_id,
                    i64::from(event.in_press),
                    event.created_at,
                ],
            )
            .expect("event should insert");
    }

    #[test]
    fn manifest_uses_provider_neutral_events_and_acks_after_publish() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        insert_event(
            &connection,
            &ContentChangeEvent {
                event_id: 1,
                content_revision: "revision-1".to_string(),
                article_id: 101,
                change_kind: "upsert".to_string(),
                journal_id: 10,
                issue_id: Some(20),
                in_press: false,
                created_at: "2026-07-18T00:00:00Z".to_string(),
            },
        );
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("changes.json");
        let count = write_content_change_manifest(
            &connection,
            "catalog.sqlite",
            "run-1",
            "2026-07-18T00:00:01Z",
            &path,
        )
        .expect("manifest should publish");
        assert_eq!(count, 1);
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("manifest should read"))
                .expect("manifest should parse");
        assert_eq!(payload["changed_issue_keys"], json!(["10:20"]));
        assert_eq!(payload["notifiable_article_ids"], json!([101]));
        assert!(list_content_change_events(&connection, 0, 10)
            .expect("outbox should read")
            .is_empty());
    }

    #[test]
    fn failed_publish_does_not_ack_outbox() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        insert_event(
            &connection,
            &ContentChangeEvent {
                event_id: 1,
                content_revision: "revision-1".to_string(),
                article_id: 101,
                change_kind: "upsert".to_string(),
                journal_id: 10,
                issue_id: None,
                in_press: true,
                created_at: "2026-07-18T00:00:00Z".to_string(),
            },
        );
        let directory = tempdir().expect("temporary directory should create");
        let path = directory.path().join("parent-is-file");
        std::fs::write(&path, "occupied").expect("blocking file should write");
        let error = write_content_change_manifest(
            &connection,
            "catalog.sqlite",
            "run-1",
            "2026-07-18T00:00:01Z",
            &path.join("changes.json"),
        )
        .expect_err("publication should fail");
        assert!(matches!(error, super::ChangeWriteError::Io(_)));
        assert_eq!(
            list_content_change_events(&connection, 0, 10)
                .expect("outbox should read")
                .len(),
            1
        );
    }
}
