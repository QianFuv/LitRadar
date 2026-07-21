//! Disposable provider-scoped indexing control storage.

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::schema::ContentDatabaseError;

/// Current disposable control database schema version.
pub const CONTROL_SCHEMA_VERSION: i64 = 1;

const CONTROL_BUSY_TIMEOUT_SECONDS: u64 = 30;
const LEASE_DURATION_SECONDS: i64 = 300;

/// Provider checkpoint scope stored outside canonical content databases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointScope {
    /// Provider-wide listing or cursor checkpoint for one catalog.
    Listing,
    /// Checkpoint for one canonical journal.
    Journal {
        /// Immutable LitRadar catalog identifier.
        catalog_id: String,
    },
    /// Checkpoint for one canonical journal publication year.
    Year {
        /// Immutable LitRadar catalog identifier.
        catalog_id: String,
        /// Four-digit publication year.
        year: i64,
    },
}

impl CheckpointScope {
    fn parts(&self) -> (&'static str, String) {
        match self {
            Self::Listing => ("listing", String::new()),
            Self::Journal { catalog_id } => ("journal", catalog_id.clone()),
            Self::Year { catalog_id, year } => ("year", format!("{catalog_id}:{year}")),
        }
    }
}

/// Disposable control database operation failure.
#[derive(Debug)]
pub enum ControlDatabaseError {
    /// Filesystem setup failed.
    Io(std::io::Error),
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// A non-disposable newer control schema was opened by an older binary.
    UnsupportedVersion {
        /// Version stored by the control database.
        found: i64,
        /// Highest version supported by this binary.
        supported: i64,
    },
    /// Another run owns an unexpired provider-scoped lease.
    ActiveLease {
        /// Current owner run identifier.
        run_id: String,
        /// Lease expiry as Unix seconds.
        expires_at: i64,
    },
    /// The requested run no longer owns the provider-scoped lease.
    OwnershipLost {
        /// Run identifier that failed the ownership check.
        run_id: String,
    },
}

impl fmt::Display for ControlDatabaseError {
    /// Format a safe control database diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported index control schema version {found}; maximum supported is {supported}"
            ),
            Self::ActiveLease { run_id, expires_at } => write!(
                formatter,
                "index control scope is owned by active run {run_id} until {expires_at}"
            ),
            Self::OwnershipLost { run_id } => {
                write!(
                    formatter,
                    "index run {run_id} no longer owns its control lease"
                )
            }
        }
    }
}

impl Error for ControlDatabaseError {
    /// Return the underlying SQLite failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::UnsupportedVersion { .. }
            | Self::ActiveLease { .. }
            | Self::OwnershipLost { .. } => None,
        }
    }
}

impl From<std::io::Error> for ControlDatabaseError {
    /// Convert filesystem failures into control database errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for ControlDatabaseError {
    /// Convert SQLite failures into control database errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Open or recreate one disposable control database.
///
/// # Arguments
///
/// * `path` - Control database path outside the content index directory.
///
/// # Returns
///
/// Initialized control connection.
pub fn open_control_db(path: impl AsRef<Path>) -> Result<Connection, ControlDatabaseError> {
    if let Some(parent) = path
        .as_ref()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(CONTROL_BUSY_TIMEOUT_SECONDS))?;
    init_control_db(&connection)?;
    Ok(connection)
}

/// Failure while committing content before its disposable provider checkpoint.
#[derive(Debug)]
pub enum ContentCheckpointCommitError {
    /// The provider-neutral content transaction failed, so the checkpoint was not attempted.
    Content(ContentDatabaseError),
    /// Content committed, but the disposable checkpoint transaction failed.
    Control(ControlDatabaseError),
}

impl fmt::Display for ContentCheckpointCommitError {
    /// Format one ordered content/checkpoint commit failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(error) => write!(formatter, "content commit failed: {error}"),
            Self::Control(error) => write!(formatter, "checkpoint commit failed: {error}"),
        }
    }
}

impl Error for ContentCheckpointCommitError {
    /// Return the failed content or control operation.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Content(error) => Some(error),
            Self::Control(error) => Some(error),
        }
    }
}

impl From<ContentDatabaseError> for ContentCheckpointCommitError {
    /// Convert a content transaction failure into an ordered commit failure.
    fn from(error: ContentDatabaseError) -> Self {
        Self::Content(error)
    }
}

impl From<ControlDatabaseError> for ContentCheckpointCommitError {
    /// Convert a checkpoint transaction failure into an ordered commit failure.
    fn from(error: ControlDatabaseError) -> Self {
        Self::Control(error)
    }
}

/// Commit provider-neutral content before advancing one disposable checkpoint.
///
/// # Arguments
///
/// * `control_connection` - Open provider-scoped control database.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `scope` - Canonical checkpoint scope.
/// * `checkpoint` - Opaque provider checkpoint to commit after content.
/// * `updated_at` - Safe checkpoint timestamp.
/// * `write_content` - One atomic provider-neutral content operation.
///
/// # Returns
///
/// The content operation outcome after both ordered commits succeed. A control failure leaves
/// committed content for idempotent replay and never advances the checkpoint first.
pub fn commit_content_then_checkpoint<Outcome, WriteContent>(
    control_connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    scope: &CheckpointScope,
    checkpoint: &str,
    updated_at: &str,
    write_content: WriteContent,
) -> Result<Outcome, ContentCheckpointCommitError>
where
    WriteContent: FnOnce() -> Result<Outcome, ContentDatabaseError>,
{
    let outcome = write_content()?;
    write_checkpoint(
        control_connection,
        catalog_name,
        provider_name,
        scope,
        checkpoint,
        updated_at,
    )?;
    Ok(outcome)
}

/// Initialize one empty or current disposable control database.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
///
/// # Returns
///
/// Success after schema validation or initialization.
pub fn init_control_db(connection: &Connection) -> Result<(), ControlDatabaseError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if version > CONTROL_SCHEMA_VERSION {
        return Err(ControlDatabaseError::UnsupportedVersion {
            found: version,
            supported: CONTROL_SCHEMA_VERSION,
        });
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;

         CREATE TABLE IF NOT EXISTS provider_leases (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             run_id TEXT NOT NULL,
             heartbeat_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             PRIMARY KEY (catalog_name, provider_name)
         );

         CREATE TABLE IF NOT EXISTS provider_checkpoints (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             scope_kind TEXT NOT NULL
                 CHECK (scope_kind IN ('listing', 'journal', 'year')),
             scope_key TEXT NOT NULL,
             checkpoint TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             PRIMARY KEY (catalog_name, provider_name, scope_kind, scope_key)
         );

         CREATE INDEX IF NOT EXISTS idx_provider_checkpoints_catalog_provider
             ON provider_checkpoints(catalog_name, provider_name);
         PRAGMA user_version = 1;",
    )?;
    Ok(())
}

/// Read one opaque provider checkpoint.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `scope` - Canonical checkpoint scope.
///
/// # Returns
///
/// Opaque checkpoint when previously committed.
pub fn read_checkpoint(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    scope: &CheckpointScope,
) -> Result<Option<String>, ControlDatabaseError> {
    let (scope_kind, scope_key) = scope.parts();
    Ok(connection
        .query_row(
            "SELECT checkpoint
             FROM provider_checkpoints
             WHERE catalog_name = ?1 AND provider_name = ?2
               AND scope_kind = ?3 AND scope_key = ?4",
            params![catalog_name, provider_name, scope_kind, scope_key],
            |row| row.get(0),
        )
        .optional()?)
}

/// Check whether retired catalog aliases own any journal or year checkpoint.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `catalog_aliases` - Retired catalog identifiers claimed by canonical entries.
///
/// # Returns
///
/// Whether any provider namespace retains checkpoint state for an alias.
pub fn has_catalog_alias_checkpoints(
    connection: &Connection,
    catalog_name: &str,
    catalog_aliases: &[String],
) -> Result<bool, ControlDatabaseError> {
    for alias in catalog_aliases {
        let has_checkpoint = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1
                 FROM provider_checkpoints
                 WHERE catalog_name = ?1
                   AND (
                       (scope_kind = 'journal' AND scope_key = ?2)
                       OR (
                           scope_kind = 'year'
                           AND substr(scope_key, 1, length(?2) + 1) = ?2 || ':'
                       )
                   )
             )",
            params![catalog_name, alias],
            |row| row.get::<_, bool>(0),
        )?;
        if has_checkpoint {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Commit one opaque provider checkpoint after a content transaction succeeds.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `scope` - Canonical checkpoint scope.
/// * `checkpoint` - Opaque provider checkpoint.
/// * `updated_at` - Safe orchestration timestamp.
///
/// # Returns
///
/// Success after an independent immediate control transaction commits.
pub fn write_checkpoint(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    scope: &CheckpointScope,
    checkpoint: &str,
    updated_at: &str,
) -> Result<(), ControlDatabaseError> {
    let (scope_kind, scope_key) = scope.parts();
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    transaction.execute(
        "INSERT INTO provider_checkpoints (
             catalog_name, provider_name, scope_kind, scope_key, checkpoint, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(catalog_name, provider_name, scope_kind, scope_key) DO UPDATE SET
             checkpoint = excluded.checkpoint,
             updated_at = excluded.updated_at",
        params![
            catalog_name,
            provider_name,
            scope_kind,
            scope_key,
            checkpoint,
            updated_at
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Acquire or reclaim one provider-scoped control lease.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `run_id` - Unique orchestration run identifier.
/// * `now_epoch_seconds` - Current Unix timestamp.
///
/// # Returns
///
/// Success when this run owns the lease.
pub fn acquire_lease(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), ControlDatabaseError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let existing = transaction
        .query_row(
            "SELECT run_id, expires_at
             FROM provider_leases
             WHERE catalog_name = ?1 AND provider_name = ?2",
            params![catalog_name, provider_name],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    if let Some((owner, expires_at)) = existing {
        if owner != run_id && expires_at > now_epoch_seconds {
            return Err(ControlDatabaseError::ActiveLease {
                run_id: owner,
                expires_at,
            });
        }
    }
    transaction.execute(
        "INSERT INTO provider_leases (
             catalog_name, provider_name, run_id, heartbeat_at, expires_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(catalog_name, provider_name) DO UPDATE SET
             run_id = excluded.run_id,
             heartbeat_at = excluded.heartbeat_at,
             expires_at = excluded.expires_at",
        params![
            catalog_name,
            provider_name,
            run_id,
            now_epoch_seconds,
            now_epoch_seconds + LEASE_DURATION_SECONDS
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Renew a provider-scoped control lease owned by one run.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `run_id` - Expected lease owner.
/// * `now_epoch_seconds` - Current Unix timestamp.
///
/// # Returns
///
/// Success when the exact owner renewed an unexpired lease.
pub fn heartbeat_lease(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), ControlDatabaseError> {
    let changed = connection.execute(
        "UPDATE provider_leases
         SET heartbeat_at = ?4, expires_at = ?5
         WHERE catalog_name = ?1 AND provider_name = ?2 AND run_id = ?3
           AND expires_at > ?4",
        params![
            catalog_name,
            provider_name,
            run_id,
            now_epoch_seconds,
            now_epoch_seconds + LEASE_DURATION_SECONDS
        ],
    )?;
    if changed == 0 {
        return Err(ControlDatabaseError::OwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    Ok(())
}

/// Release a provider-scoped lease owned by one run.
///
/// # Arguments
///
/// * `connection` - Open control database connection.
/// * `catalog_name` - Stable maintained catalog stem.
/// * `provider_name` - Stable runtime provider name.
/// * `run_id` - Expected lease owner.
///
/// # Returns
///
/// Success when the exact owner removed its lease.
pub fn release_lease(
    connection: &Connection,
    catalog_name: &str,
    provider_name: &str,
    run_id: &str,
) -> Result<(), ControlDatabaseError> {
    let changed = connection.execute(
        "DELETE FROM provider_leases
         WHERE catalog_name = ?1 AND provider_name = ?2 AND run_id = ?3",
        params![catalog_name, provider_name, run_id],
    )?;
    if changed == 0 {
        return Err(ControlDatabaseError::OwnershipLost {
            run_id: run_id.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use litradar_domain::{
        ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft, JournalRankings, ProviderBatch,
    };
    use tempfile::tempdir;

    use crate::schema::{init_content_db, open_content_db, write_content_batch};

    use super::{
        acquire_lease, commit_content_then_checkpoint, has_catalog_alias_checkpoints,
        heartbeat_lease, open_control_db, read_checkpoint, release_lease, write_checkpoint,
        CheckpointScope, ContentCheckpointCommitError, ControlDatabaseError,
    };

    #[test]
    fn checkpoints_are_provider_scoped_and_loss_replays_from_empty_state() {
        let temp = tempdir().expect("temporary directory should create");
        let content_path = temp.path().join("catalog.sqlite");
        let content = open_content_db(&content_path).expect("content database should open");
        write_content_batch(
            &content,
            &canonical_catalog(),
            &canonical_batch(),
            "revision-a",
            "2026-07-18T00:00:00Z",
        )
        .expect("initial content should commit");
        content
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .expect("content WAL should checkpoint");
        drop(content);
        let content_before = std::fs::read(&content_path).expect("content bytes should read");
        let path = temp.path().join("catalog.control.sqlite");
        let connection = open_control_db(&path).expect("control database should open");
        let scope = CheckpointScope::Journal {
            catalog_id: "issn-1234-5679".to_string(),
        };
        write_checkpoint(
            &connection,
            "chinese_journals",
            "cnki",
            &scope,
            "opaque-a",
            "2026-07-18T00:00:00Z",
        )
        .expect("checkpoint should write");
        assert_eq!(
            read_checkpoint(&connection, "chinese_journals", "cnki", &scope)
                .expect("checkpoint should read")
                .as_deref(),
            Some("opaque-a")
        );
        assert_eq!(
            read_checkpoint(&connection, "chinese_journals", "replacement", &scope)
                .expect("replacement provider checkpoint should read"),
            None
        );
        drop(connection);
        std::fs::remove_file(&path).expect("disposable control database should delete");
        let recreated = open_control_db(&path).expect("control database should recreate");
        assert_eq!(
            read_checkpoint(&recreated, "chinese_journals", "cnki", &scope)
                .expect("recreated checkpoint should read"),
            None
        );
        assert_eq!(
            std::fs::read(&content_path).expect("content bytes should remain readable"),
            content_before
        );
        let content = open_content_db(&content_path).expect("content database should reopen");
        let replay = write_content_batch(
            &content,
            &canonical_catalog(),
            &canonical_batch(),
            "revision-a",
            "2026-07-18T00:00:00Z",
        )
        .expect("content should replay after control loss");
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(table_count(&content, "articles"), 1);
        assert_eq!(table_count(&content, "article_change_events"), 1);
    }

    #[test]
    fn content_precedes_checkpoint_and_both_failure_sides_are_replay_safe() {
        let content = rusqlite::Connection::open_in_memory().expect("content database should open");
        init_content_db(&content).expect("content schema should initialize");
        let control = rusqlite::Connection::open_in_memory().expect("control database should open");
        super::init_control_db(&control).expect("control schema should initialize");
        let catalog = canonical_catalog();
        let batch = canonical_batch();
        let scope = CheckpointScope::Listing;

        content
            .execute_batch(
                "CREATE TRIGGER fail_content_event
                 BEFORE INSERT ON article_change_events
                 BEGIN SELECT RAISE(ABORT, 'forced content failure'); END;",
            )
            .expect("content failpoint should install");
        let content_error = commit_content_then_checkpoint(
            &control,
            "chinese_journals",
            "provider-a",
            &scope,
            "page-2",
            "2026-07-18T00:00:00Z",
            || {
                write_content_batch(
                    &content,
                    &catalog,
                    &batch,
                    "revision-a",
                    "2026-07-18T00:00:00Z",
                )
            },
        )
        .expect_err("content failure should stop checkpoint commit");
        assert!(matches!(
            content_error,
            ContentCheckpointCommitError::Content(_)
        ));
        assert_eq!(table_count(&content, "articles"), 0);
        assert_eq!(
            read_checkpoint(&control, "chinese_journals", "provider-a", &scope)
                .expect("checkpoint should read"),
            None
        );
        content
            .execute_batch("DROP TRIGGER fail_content_event")
            .expect("content failpoint should drop");

        control
            .execute_batch(
                "CREATE TRIGGER fail_checkpoint
                 BEFORE INSERT ON provider_checkpoints
                 BEGIN SELECT RAISE(ABORT, 'forced checkpoint failure'); END;",
            )
            .expect("checkpoint failpoint should install");
        let checkpoint_error = commit_content_then_checkpoint(
            &control,
            "chinese_journals",
            "provider-a",
            &scope,
            "page-2",
            "2026-07-18T00:00:00Z",
            || {
                write_content_batch(
                    &content,
                    &catalog,
                    &batch,
                    "revision-a",
                    "2026-07-18T00:00:00Z",
                )
            },
        )
        .expect_err("checkpoint failure should surface after content commits");
        assert!(
            matches!(checkpoint_error, ContentCheckpointCommitError::Control(_)),
            "unexpected checkpoint-side error: {checkpoint_error:?}"
        );
        assert_eq!(table_count(&content, "articles"), 1);
        assert_eq!(table_count(&content, "article_change_events"), 1);
        assert_eq!(
            read_checkpoint(&control, "chinese_journals", "provider-a", &scope)
                .expect("checkpoint should read"),
            None
        );
        control
            .execute_batch("DROP TRIGGER fail_checkpoint")
            .expect("checkpoint failpoint should drop");

        let replay = commit_content_then_checkpoint(
            &control,
            "chinese_journals",
            "provider-a",
            &scope,
            "page-2",
            "2026-07-18T00:01:00Z",
            || {
                write_content_batch(
                    &content,
                    &catalog,
                    &batch,
                    "revision-a",
                    "2026-07-18T00:00:00Z",
                )
            },
        )
        .expect("idempotent replay should advance the checkpoint");
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(replay.change_events_emitted, 0);
        assert_eq!(table_count(&content, "article_change_events"), 1);
        assert_eq!(
            read_checkpoint(&control, "chinese_journals", "provider-a", &scope)
                .expect("checkpoint should read")
                .as_deref(),
            Some("page-2")
        );
    }

    #[test]
    fn legacy_alias_checkpoint_detection_covers_every_provider_and_scope() {
        let connection = rusqlite::Connection::open_in_memory().expect("control database opens");
        super::init_control_db(&connection).expect("control schema should initialize");
        let aliases = vec!["legacy-journal".to_string()];
        write_checkpoint(
            &connection,
            "english_journals",
            "provider-a",
            &CheckpointScope::Journal {
                catalog_id: "canonical-journal".to_string(),
            },
            "complete",
            "2026-07-20T00:00:00Z",
        )
        .expect("canonical checkpoint should write");
        assert!(
            !has_catalog_alias_checkpoints(&connection, "english_journals", &aliases)
                .expect("canonical checkpoint should not block an alias")
        );

        write_checkpoint(
            &connection,
            "english_journals",
            "provider-b",
            &CheckpointScope::Year {
                catalog_id: aliases[0].clone(),
                year: 2025,
            },
            "cursor",
            "2026-07-20T00:00:00Z",
        )
        .expect("legacy year checkpoint should write");
        assert!(
            has_catalog_alias_checkpoints(&connection, "english_journals", &aliases)
                .expect("legacy checkpoint should be detected")
        );
    }

    #[test]
    fn leases_are_fenced_by_provider_scope_owner_and_expiry() {
        let connection = rusqlite::Connection::open_in_memory().expect("control database opens");
        super::init_control_db(&connection).expect("control schema should initialize");
        acquire_lease(&connection, "catalog", "provider", "run-a", 100)
            .expect("first owner should acquire");
        assert!(matches!(
            acquire_lease(&connection, "catalog", "provider", "run-b", 101),
            Err(ControlDatabaseError::ActiveLease { .. })
        ));
        heartbeat_lease(&connection, "catalog", "provider", "run-a", 102)
            .expect("owner should renew");
        assert!(matches!(
            heartbeat_lease(&connection, "catalog", "provider", "run-b", 103),
            Err(ControlDatabaseError::OwnershipLost { .. })
        ));
        release_lease(&connection, "catalog", "provider", "run-a").expect("owner should release");
        acquire_lease(&connection, "catalog", "provider", "run-b", 104)
            .expect("next owner should acquire");

        acquire_lease(&connection, "catalog", "expired", "run-old", 100)
            .expect("expiring owner should acquire");
        acquire_lease(&connection, "catalog", "expired", "run-new", 400)
            .expect("expired lease should be reclaimed");
    }

    fn canonical_catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "issn-1234-5679".to_string(),
            catalog_aliases: Vec::new(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: Some("Systems".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn canonical_batch() -> ProviderBatch {
        ProviderBatch {
            catalog_id: "issn-1234-5679".to_string(),
            journal: JournalDraft {
                catalog_id: "issn-1234-5679".to_string(),
                observed_title: Some("Canonical Journal".to_string()),
                observed_issns: vec!["1234-5679".to_string()],
                observed_title_aliases: Vec::new(),
            },
            issues: vec![IssueDraft {
                catalog_id: "issn-1234-5679".to_string(),
                publication_year: Some(2026),
                title: None,
                volume: Some("1".to_string()),
                number: Some("2".to_string()),
                date: Some("2026-07".to_string()),
            }],
            articles: vec![ArticleDraft {
                catalog_id: "issn-1234-5679".to_string(),
                title: "Canonical Article".to_string(),
                publication_year: Some(2026),
                date: Some("2026-07-18".to_string()),
                issue_title: None,
                volume: Some("1".to_string()),
                issue_number: Some("2".to_string()),
                authors: Vec::new(),
                start_page: Some("1".to_string()),
                end_page: Some("8".to_string()),
                abstract_text: Some("Canonical abstract".to_string()),
                doi: Some("10.1000/canonical".to_string()),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_dois: Vec::new(),
            }],
            is_complete: false,
            next_checkpoint: Some("page-2".to_string()),
        }
    }

    fn table_count(connection: &rusqlite::Connection, table_name: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
                row.get(0)
            })
            .expect("table count should load")
    }
}
