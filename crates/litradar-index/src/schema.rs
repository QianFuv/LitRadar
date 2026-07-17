//! SQLite schema and writer helpers for Rust scholarly indexing.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{
    params, params_from_iter, Connection, LoadExtensionGuard, OptionalExtension, Transaction,
    TransactionBehavior,
};

use litradar_domain::{
    ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, ProviderBatch,
};
use litradar_provider::conformance::{validate_provider_batch, ContractViolation};

use crate::identity::{
    article_identity_keys, issue_id_from_draft, journal_id_from_catalog_id, merge_article_drafts,
    resolve_article_identity, ArticleIdentityError, ArticleIdentityKey, ArticleMergeError,
};
use crate::stats::IndexRunStats;
use crate::transforms::{ArticleRecord, IssueRecord, JournalRecord, MetaRecord};

const INDEX_BUSY_TIMEOUT_SECONDS: u64 = 30;
const INDEX_RUN_LEASE_DURATION_SECONDS: i64 = 300;
const PENDING_EVENT_ADOPTION_BATCH_SIZE: i64 = 1_000;

/// Current provider-neutral content database schema version.
pub const CONTENT_SCHEMA_VERSION: i64 = 4;

const CONTENT_TABLES_SQL: &str = "
    CREATE TABLE journals (
        journal_id INTEGER PRIMARY KEY,
        catalog_id TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        title_aliases_json TEXT NOT NULL,
        issns_json TEXT NOT NULL,
        issn TEXT,
        eissn TEXT,
        area TEXT,
        utd_rank TEXT,
        utd_rating TEXT,
        abs_rank TEXT,
        abs_rating TEXT,
        fms_rank TEXT,
        fms_rating TEXT,
        fmscn_rank TEXT,
        fmscn_rating TEXT
    );

    CREATE TABLE issues (
        issue_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        publication_year INTEGER,
        title TEXT,
        volume TEXT,
        number TEXT,
        date TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
    );

    CREATE TABLE articles (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        title TEXT NOT NULL,
        publication_year INTEGER,
        date TEXT,
        authors_json TEXT NOT NULL,
        start_page TEXT,
        end_page TEXT,
        abstract_text TEXT,
        doi TEXT,
        pmid TEXT,
        open_access INTEGER,
        in_press INTEGER,
        retraction_doi TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE TABLE article_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('doi', 'pmid', 'bibliographic')),
        identity_value TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        PRIMARY KEY (identity_kind, identity_value),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_listing (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        publication_year INTEGER,
        date TEXT,
        open_access INTEGER,
        in_press INTEGER,
        doi TEXT,
        pmid TEXT,
        area TEXT,
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE VIRTUAL TABLE article_search
    USING fts5(
        article_id UNINDEXED,
        title,
        abstract_text,
        doi,
        pmid,
        authors,
        journal_title,
        tokenize = 'unicode61 remove_diacritics 2'
    );

    CREATE TABLE article_change_events (
        event_id INTEGER PRIMARY KEY,
        content_revision TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        change_kind TEXT NOT NULL CHECK (change_kind IN ('upsert', 'remove')),
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        in_press INTEGER NOT NULL CHECK (in_press IN (0, 1)),
        created_at TEXT NOT NULL
    );

    CREATE INDEX idx_journals_issn ON journals(issn);
    CREATE INDEX idx_journals_eissn ON journals(eissn);
    CREATE INDEX idx_issues_journal_year ON issues(journal_id, publication_year);
    CREATE INDEX idx_articles_journal ON articles(journal_id);
    CREATE INDEX idx_articles_issue ON articles(issue_id);
    CREATE INDEX idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX idx_articles_doi ON articles(doi);
    CREATE INDEX idx_articles_pmid ON articles(pmid);
    CREATE INDEX idx_article_identity_keys_article ON article_identity_keys(article_id);
    CREATE INDEX idx_article_listing_date_id ON article_listing(date, article_id);
    CREATE INDEX idx_article_listing_journal_date_id
        ON article_listing(journal_id, date, article_id);
    CREATE INDEX idx_article_listing_issue ON article_listing(issue_id);
    CREATE UNIQUE INDEX idx_article_change_events_revision
        ON article_change_events(
            content_revision, article_id, change_kind, journal_id,
            COALESCE(issue_id, -1), in_press
        );
    CREATE INDEX idx_article_change_events_order ON article_change_events(event_id);
";

/// Context attached to normalized index change events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeEventContext {
    run_id: String,
    worker_id: String,
    created_at: String,
    is_backfill: bool,
}

impl ChangeEventContext {
    /// Build a change-event context for one indexing worker.
    pub(crate) fn new(
        run_id: impl Into<String>,
        worker_id: impl Into<String>,
        created_at: impl Into<String>,
        is_backfill: bool,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            worker_id: worker_id.into(),
            created_at: created_at.into(),
            is_backfill,
        }
    }
}

/// Ownership token passed through live index write paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexRunLeaseContext {
    run_id: String,
}

impl IndexRunLeaseContext {
    /// Build a lease context for one live run.
    pub(crate) fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }

    /// Return the owned run identifier.
    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Renew this run's lease using the current system time.
    pub(crate) fn heartbeat(&self, connection: &Connection) -> Result<(), IndexRunLeaseError> {
        heartbeat_index_run_lease(connection, &self.run_id, current_epoch_seconds()?)
    }

    /// Refresh this exact owner after its acquisition transaction commits.
    pub(crate) fn refresh_after_acquisition(
        &self,
        connection: &Connection,
    ) -> Result<(), IndexRunLeaseError> {
        refresh_index_run_lease_after_acquisition(
            connection,
            &self.run_id,
            current_epoch_seconds()?,
        )
    }

    /// Assert that this run still owns an unexpired lease.
    pub(crate) fn assert_owner(&self, connection: &Connection) -> Result<(), IndexRunLeaseError> {
        assert_index_run_lease_owner(connection, &self.run_id, current_epoch_seconds()?)
    }

    /// Release this run's lease and reject a mismatched owner.
    pub(crate) fn release(&self, connection: &Connection) -> Result<(), IndexRunLeaseError> {
        if release_index_run_lease(connection, &self.run_id)? {
            Ok(())
        } else {
            Err(IndexRunLeaseError::OwnershipLost {
                run_id: self.run_id.clone(),
            })
        }
    }
}

/// Parameters required to acquire one live index database lease.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexRunStartRequest<'a> {
    /// Unique run identifier.
    pub(crate) run_id: &'a str,
    /// Source CSV filename.
    pub(crate) csv_file: &'a str,
    /// Human-readable run start timestamp.
    pub(crate) started_at: &'a str,
    /// Expected journal count.
    pub(crate) total_journals: i64,
    /// Current Unix timestamp in seconds.
    pub(crate) now_epoch_seconds: i64,
    /// Whether pending events should be adopted by this run.
    pub(crate) should_adopt_events: bool,
}

/// Recovery details returned after a live index run acquires its lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexRunStartOutcome {
    /// Previous lease owner reclaimed by this run.
    pub(crate) interrupted_run_id: Option<String>,
    /// Source event rows replayed under the new run identifier.
    pub(crate) adopted_event_count: usize,
}

/// Errors returned by durable live index lease operations.
#[derive(Debug)]
pub(crate) enum IndexRunLeaseError {
    /// A different run still owns an unexpired lease.
    ActiveLease {
        /// Current owner run identifier.
        run_id: String,
        /// Current lease expiry as Unix seconds.
        expires_at: i64,
    },
    /// The requested run no longer owns an unexpired lease.
    OwnershipLost {
        /// Run identifier that failed the ownership check.
        run_id: String,
    },
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// The system clock could not produce a Unix timestamp.
    Clock(std::time::SystemTimeError),
}

impl fmt::Display for IndexRunLeaseError {
    /// Format a lease error without database contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActiveLease { run_id, expires_at } => write!(
                formatter,
                "index database is owned by active run {run_id} until {expires_at}"
            ),
            Self::OwnershipLost { run_id } => {
                write!(
                    formatter,
                    "index run {run_id} no longer owns the database lease"
                )
            }
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Clock(error) => {
                write!(formatter, "system clock is before the Unix epoch: {error}")
            }
        }
    }
}

impl Error for IndexRunLeaseError {
    /// Return the underlying SQLite error when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Clock(error) => Some(error),
            Self::ActiveLease { .. } | Self::OwnershipLost { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for IndexRunLeaseError {
    /// Convert SQLite errors into lease errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::time::SystemTimeError> for IndexRunLeaseError {
    /// Convert system clock errors into lease errors.
    fn from(error: std::time::SystemTimeError) -> Self {
        Self::Clock(error)
    }
}

#[derive(Debug)]
struct PendingChangeEvent {
    event_id: i64,
    worker_id: String,
    article_id: i64,
    event_type: String,
    membership_type: String,
    journal_id: i64,
    issue_id: Option<i64>,
    is_backfill: bool,
    created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArticleMembership {
    membership_type: &'static str,
    journal_id: i64,
    issue_id: Option<i64>,
}

/// Provider-neutral content database initialization or write failure.
#[derive(Debug)]
pub enum ContentDatabaseError {
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// JSON encoding or decoding of an explicit canonical array failed.
    Json(serde_json::Error),
    /// A provider response violated the canonical contract.
    Contract(ContractViolation),
    /// Canonical aliases were missing or conflicted.
    Identity(ArticleIdentityError),
    /// Canonical article values could not be merged safely.
    Merge(ArticleMergeError),
    /// A legacy or unknown schema must be rebuilt instead of migrated.
    RebuildRequired {
        /// Existing SQLite user version.
        found_version: i64,
    },
    /// A current-version database does not match the exact content schema.
    InvalidCurrentSchema(String),
    /// A deterministic SQLite ID collided with an unrelated existing article.
    ArticleIdCollision {
        /// Colliding internal article identifier.
        article_id: i64,
    },
}

impl fmt::Display for ContentDatabaseError {
    /// Format a content database failure without provider payloads or paths.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Contract(error) => write!(formatter, "{error}"),
            Self::Identity(error) => write!(formatter, "{error}"),
            Self::Merge(error) => write!(formatter, "{error}"),
            Self::RebuildRequired { found_version } => write!(
                formatter,
                "index schema version {found_version} requires an explicit rebuild for content schema v{CONTENT_SCHEMA_VERSION}"
            ),
            Self::InvalidCurrentSchema(message) => {
                write!(formatter, "invalid content schema v{CONTENT_SCHEMA_VERSION}: {message}")
            }
            Self::ArticleIdCollision { article_id } => {
                write!(formatter, "canonical article ID collision for {article_id}")
            }
        }
    }
}

impl Error for ContentDatabaseError {
    /// Return the underlying failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Contract(error) => Some(error),
            Self::Identity(error) => Some(error),
            Self::Merge(error) => Some(error),
            Self::RebuildRequired { .. }
            | Self::InvalidCurrentSchema(_)
            | Self::ArticleIdCollision { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for ContentDatabaseError {
    /// Convert SQLite failures into content database errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for ContentDatabaseError {
    /// Convert canonical JSON failures into content database errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<ContractViolation> for ContentDatabaseError {
    /// Convert provider contract failures into content database errors.
    fn from(error: ContractViolation) -> Self {
        Self::Contract(error)
    }
}

impl From<ArticleIdentityError> for ContentDatabaseError {
    /// Convert identity failures into content database errors.
    fn from(error: ArticleIdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<ArticleMergeError> for ContentDatabaseError {
    /// Convert merge failures into content database errors.
    fn from(error: ArticleMergeError) -> Self {
        Self::Merge(error)
    }
}

/// Aggregate result of one atomic canonical content write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContentWriteOutcome {
    /// Canonical article drafts examined.
    pub articles_seen: usize,
    /// New or durably changed article rows.
    pub articles_changed: usize,
    /// New immutable identity aliases attached.
    pub identity_aliases_added: usize,
    /// Provider-neutral outbox events emitted.
    pub change_events_emitted: usize,
}

/// Open and validate a provider-neutral content database.
///
/// # Arguments
///
/// * `path` - Content database path derived only from the catalog filename.
///
/// # Returns
///
/// Initialized v4 connection or an explicit rebuild-required failure.
pub fn open_content_db(path: impl AsRef<Path>) -> Result<Connection, ContentDatabaseError> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(INDEX_BUSY_TIMEOUT_SECONDS))?;
    init_content_db(&connection)?;
    Ok(connection)
}

/// Initialize an empty content database or validate an existing v4 database.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
///
/// # Returns
///
/// Success only for an empty database or an exact current schema.
pub fn init_content_db(connection: &Connection) -> Result<(), ContentDatabaseError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    let object_count = content_schema_object_count(connection)?;
    if version == CONTENT_SCHEMA_VERSION {
        return validate_current_content_schema(connection);
    }
    if version != 0 || object_count != 0 {
        return Err(ContentDatabaseError::RebuildRequired {
            found_version: version,
        });
    }

    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    transaction.execute_batch(CONTENT_TABLES_SQL)?;
    transaction.pragma_update(None, "user_version", CONTENT_SCHEMA_VERSION)?;
    transaction.commit()?;
    validate_current_content_schema(connection)
}

/// Atomically validate, identify, merge, project, and enqueue one canonical batch.
///
/// # Arguments
///
/// * `connection` - Open provider-neutral content database.
/// * `catalog` - LitRadar-owned maintained journal entry.
/// * `batch` - Canonical provider response.
/// * `content_revision` - Core-owned idempotency label for emitted outbox rows.
/// * `created_at` - Safe content change timestamp.
///
/// # Returns
///
/// Deterministic write counts after one immediate transaction commits.
pub fn write_content_batch(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
    batch: &ProviderBatch,
    content_revision: &str,
    created_at: &str,
) -> Result<ContentWriteOutcome, ContentDatabaseError> {
    validate_provider_batch(catalog, batch)?;
    if content_revision.is_empty() || content_revision != content_revision.trim() {
        return Err(ContentDatabaseError::InvalidCurrentSchema(
            "content revision must be non-empty and trimmed".to_string(),
        ));
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let journal_id = upsert_canonical_journal(&transaction, catalog)?;
    for issue in &batch.issues {
        upsert_canonical_issue(&transaction, journal_id, issue)?;
    }

    let mut outcome = ContentWriteOutcome {
        articles_seen: batch.articles.len(),
        ..ContentWriteOutcome::default()
    };
    for article in &batch.articles {
        write_canonical_article(
            &transaction,
            catalog,
            journal_id,
            article,
            content_revision,
            created_at,
            &mut outcome,
        )?;
    }
    refresh_journal_projections(&transaction, journal_id, catalog)?;
    transaction.commit()?;
    Ok(outcome)
}

fn content_schema_object_count(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )
}

fn validate_current_content_schema(connection: &Connection) -> Result<(), ContentDatabaseError> {
    let expected = [
        "article_change_events",
        "article_identity_keys",
        "article_listing",
        "article_search",
        "articles",
        "issues",
        "journals",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    let mut statement = connection.prepare(
        "SELECT name
         FROM sqlite_schema
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND name NOT LIKE 'article_search_%'
         ORDER BY name",
    )?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if actual != expected {
        return Err(ContentDatabaseError::InvalidCurrentSchema(format!(
            "table inventory mismatch: {actual:?}"
        )));
    }
    let expected_columns: &[(&str, &[&str])] = &[
        (
            "journals",
            &[
                "journal_id",
                "catalog_id",
                "title",
                "title_aliases_json",
                "issns_json",
                "issn",
                "eissn",
                "area",
                "utd_rank",
                "utd_rating",
                "abs_rank",
                "abs_rating",
                "fms_rank",
                "fms_rating",
                "fmscn_rank",
                "fmscn_rating",
            ],
        ),
        (
            "issues",
            &[
                "issue_id",
                "journal_id",
                "publication_year",
                "title",
                "volume",
                "number",
                "date",
            ],
        ),
        (
            "articles",
            &[
                "article_id",
                "journal_id",
                "issue_id",
                "title",
                "publication_year",
                "date",
                "authors_json",
                "start_page",
                "end_page",
                "abstract_text",
                "doi",
                "pmid",
                "open_access",
                "in_press",
                "retraction_doi",
            ],
        ),
        (
            "article_identity_keys",
            &["identity_kind", "identity_value", "article_id"],
        ),
        (
            "article_listing",
            &[
                "article_id",
                "journal_id",
                "issue_id",
                "publication_year",
                "date",
                "open_access",
                "in_press",
                "doi",
                "pmid",
                "area",
            ],
        ),
        (
            "article_search",
            &[
                "article_id",
                "title",
                "abstract_text",
                "doi",
                "pmid",
                "authors",
                "journal_title",
            ],
        ),
        (
            "article_change_events",
            &[
                "event_id",
                "content_revision",
                "article_id",
                "change_kind",
                "journal_id",
                "issue_id",
                "in_press",
                "created_at",
            ],
        ),
    ];
    for (table_name, expected) in expected_columns {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
        let actual = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual != *expected {
            return Err(ContentDatabaseError::InvalidCurrentSchema(format!(
                "column inventory mismatch for {table_name}: {actual:?}"
            )));
        }
    }
    let expected_indexes = [
        "idx_article_change_events_order",
        "idx_article_change_events_revision",
        "idx_article_identity_keys_article",
        "idx_article_listing_date_id",
        "idx_article_listing_issue",
        "idx_article_listing_journal_date_id",
        "idx_articles_date_id",
        "idx_articles_doi",
        "idx_articles_issue",
        "idx_articles_journal",
        "idx_articles_pmid",
        "idx_issues_journal_year",
        "idx_journals_eissn",
        "idx_journals_issn",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let actual_indexes = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if actual_indexes != expected_indexes {
        return Err(ContentDatabaseError::InvalidCurrentSchema(format!(
            "index inventory mismatch: {actual_indexes:?}"
        )));
    }
    for forbidden in [
        "provider",
        "source",
        "platform",
        "url",
        "permalink",
        "content_location",
        "full_text",
        "checkpoint",
        "lease",
        "run_id",
        "statistics",
        "stats",
    ] {
        let count = connection.query_row(
            "SELECT COUNT(*)
             FROM pragma_table_list() AS tables
             JOIN pragma_table_xinfo(tables.name) AS columns
             WHERE tables.schema = 'main'
               AND tables.name NOT LIKE 'article_search_%'
               AND lower(columns.name) LIKE '%' || ?1 || '%'",
            [forbidden],
            |row| row.get::<_, i64>(0),
        )?;
        if count != 0 {
            return Err(ContentDatabaseError::InvalidCurrentSchema(format!(
                "forbidden column fragment {forbidden}"
            )));
        }
    }
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}

fn upsert_canonical_journal(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
) -> Result<i64, ContentDatabaseError> {
    let journal_id = journal_id_from_catalog_id(&catalog.catalog_id);
    let title_aliases_json = serde_json::to_string(&catalog.title_aliases)?;
    let issns_json = serde_json::to_string(&catalog.all_issns)?;
    connection.execute(
        "INSERT INTO journals (
             journal_id, catalog_id, title, title_aliases_json, issns_json, issn, eissn, area,
             utd_rank, utd_rating, abs_rank, abs_rating, fms_rank, fms_rating,
             fmscn_rank, fmscn_rating
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
         )
         ON CONFLICT(journal_id) DO UPDATE SET
             catalog_id = excluded.catalog_id,
             title = excluded.title,
             title_aliases_json = excluded.title_aliases_json,
             issns_json = excluded.issns_json,
             issn = excluded.issn,
             eissn = excluded.eissn,
             area = excluded.area,
             utd_rank = excluded.utd_rank,
             utd_rating = excluded.utd_rating,
             abs_rank = excluded.abs_rank,
             abs_rating = excluded.abs_rating,
             fms_rank = excluded.fms_rank,
             fms_rating = excluded.fms_rating,
             fmscn_rank = excluded.fmscn_rank,
             fmscn_rating = excluded.fmscn_rating",
        params![
            journal_id,
            catalog.catalog_id,
            catalog.title,
            title_aliases_json,
            issns_json,
            catalog.issn,
            catalog.eissn,
            catalog.area,
            catalog.rankings.utd_rank,
            catalog.rankings.utd_rating,
            catalog.rankings.abs_rank,
            catalog.rankings.abs_rating,
            catalog.rankings.fms_rank,
            catalog.rankings.fms_rating,
            catalog.rankings.fmscn_rank,
            catalog.rankings.fmscn_rating,
        ],
    )?;
    Ok(journal_id)
}

fn upsert_canonical_issue(
    connection: &Connection,
    journal_id: i64,
    issue: &IssueDraft,
) -> Result<Option<i64>, ContentDatabaseError> {
    let Some(issue_id) = issue_id_from_draft(journal_id, issue) else {
        return Ok(None);
    };
    connection.execute(
        "INSERT INTO issues (
             issue_id, journal_id, publication_year, title, volume, number, date
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(issue_id) DO UPDATE SET
             publication_year = CASE
                 WHEN issues.publication_year IS NULL THEN excluded.publication_year
                 WHEN excluded.publication_year IS NULL THEN issues.publication_year
                 ELSE MIN(issues.publication_year, excluded.publication_year)
             END,
             title = CASE
                 WHEN issues.title IS NULL THEN excluded.title
                 WHEN excluded.title IS NULL THEN issues.title
                 WHEN length(excluded.title) > length(issues.title) THEN excluded.title
                 WHEN length(excluded.title) < length(issues.title) THEN issues.title
                 ELSE MIN(issues.title, excluded.title)
             END,
             volume = CASE
                 WHEN issues.volume IS NULL THEN excluded.volume
                 WHEN excluded.volume IS NULL THEN issues.volume
                 ELSE MIN(issues.volume, excluded.volume)
             END,
             number = CASE
                 WHEN issues.number IS NULL THEN excluded.number
                 WHEN excluded.number IS NULL THEN issues.number
                 ELSE MIN(issues.number, excluded.number)
             END,
             date = CASE
                 WHEN issues.date IS NULL THEN excluded.date
                 WHEN excluded.date IS NULL THEN issues.date
                 WHEN length(excluded.date) > length(issues.date) THEN excluded.date
                 WHEN length(excluded.date) < length(issues.date) THEN issues.date
                 ELSE MIN(issues.date, excluded.date)
             END",
        params![
            issue_id,
            journal_id,
            issue.publication_year,
            issue.title,
            issue.volume,
            issue.number,
            issue.date,
        ],
    )?;
    Ok(Some(issue_id))
}

#[allow(clippy::too_many_arguments)]
fn write_canonical_article(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
    journal_id: i64,
    article: &ArticleDraft,
    content_revision: &str,
    created_at: &str,
    outcome: &mut ContentWriteOutcome,
) -> Result<(), ContentDatabaseError> {
    let incoming_keys = article_identity_keys(article);
    let existing_aliases = load_existing_identity_aliases(connection, &incoming_keys)?;
    let resolution = resolve_article_identity(article, &existing_aliases)?;
    let existing = load_canonical_article(connection, resolution.article_id)?;
    if !resolution.is_existing && existing.is_some() {
        return Err(ContentDatabaseError::ArticleIdCollision {
            article_id: resolution.article_id,
        });
    }
    let merged = if let Some((existing, _)) = &existing {
        merge_article_drafts(existing, article)?
    } else {
        article.clone()
    };
    let issue = IssueDraft {
        catalog_id: merged.catalog_id.clone(),
        publication_year: merged.publication_year,
        title: merged.issue_title.clone(),
        volume: merged.volume.clone(),
        number: merged.issue_number.clone(),
        date: merged.date.clone(),
    };
    let issue_id = upsert_canonical_issue(connection, journal_id, &issue)?;
    let previous_issue_id = existing.as_ref().and_then(|(_, issue_id)| *issue_id);
    let has_changed = existing
        .as_ref()
        .map(|(value, previous_issue_id)| value != &merged || *previous_issue_id != issue_id)
        .unwrap_or(true);

    if has_changed {
        upsert_canonical_article(
            connection,
            resolution.article_id,
            journal_id,
            issue_id,
            &merged,
        )?;
        refresh_article_projection(
            connection,
            resolution.article_id,
            journal_id,
            issue_id,
            catalog,
            &merged,
        )?;
        if existing.is_some()
            && (previous_issue_id != issue_id
                || existing
                    .as_ref()
                    .is_some_and(|(value, _)| value.in_press != merged.in_press))
        {
            outcome.change_events_emitted += record_content_change_event(
                connection,
                content_revision,
                resolution.article_id,
                "remove",
                journal_id,
                previous_issue_id,
                existing
                    .as_ref()
                    .and_then(|(value, _)| value.in_press)
                    .unwrap_or(false),
                created_at,
            )?;
        }
        outcome.change_events_emitted += record_content_change_event(
            connection,
            content_revision,
            resolution.article_id,
            "upsert",
            journal_id,
            issue_id,
            merged.in_press.unwrap_or(false),
            created_at,
        )?;
        outcome.articles_changed += 1;
    }

    for key in article_identity_keys(&merged) {
        outcome.identity_aliases_added += connection.execute(
            "INSERT INTO article_identity_keys (identity_kind, identity_value, article_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(identity_kind, identity_value) DO NOTHING",
            params![key.kind.as_str(), key.value, resolution.article_id],
        )?;
        let owner = connection.query_row(
            "SELECT article_id FROM article_identity_keys
             WHERE identity_kind = ?1 AND identity_value = ?2",
            params![key.kind.as_str(), key.value],
            |row| row.get::<_, i64>(0),
        )?;
        if owner != resolution.article_id {
            return Err(ContentDatabaseError::Identity(
                ArticleIdentityError::ConflictingAliases {
                    article_ids: vec![owner, resolution.article_id],
                },
            ));
        }
    }
    Ok(())
}

fn load_existing_identity_aliases(
    connection: &Connection,
    keys: &[ArticleIdentityKey],
) -> Result<BTreeMap<ArticleIdentityKey, i64>, ContentDatabaseError> {
    let mut aliases = BTreeMap::new();
    for key in keys {
        let article_id = connection
            .query_row(
                "SELECT article_id FROM article_identity_keys
                 WHERE identity_kind = ?1 AND identity_value = ?2",
                params![key.kind.as_str(), key.value],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(article_id) = article_id {
            aliases.insert(key.clone(), article_id);
        }
    }
    Ok(aliases)
}

fn load_canonical_article(
    connection: &Connection,
    article_id: i64,
) -> Result<Option<(ArticleDraft, Option<i64>)>, ContentDatabaseError> {
    let row = connection
        .query_row(
            "SELECT
                 j.catalog_id, a.title, a.publication_year, a.date, i.title, i.volume, i.number,
                 a.authors_json, a.start_page, a.end_page, a.abstract_text, a.doi, a.pmid,
                 a.open_access, a.in_press, a.retraction_doi, a.issue_id
             FROM articles AS a
             JOIN journals AS j ON j.journal_id = a.journal_id
             LEFT JOIN issues AS i ON i.issue_id = a.issue_id
             WHERE a.article_id = ?1",
            [article_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<i64>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            catalog_id,
            title,
            publication_year,
            date,
            issue_title,
            volume,
            issue_number,
            authors_json,
            start_page,
            end_page,
            abstract_text,
            doi,
            pmid,
            open_access,
            in_press,
            retraction_doi,
            issue_id,
        )| {
            Ok((
                ArticleDraft {
                    catalog_id,
                    title,
                    publication_year,
                    date,
                    issue_title,
                    volume,
                    issue_number,
                    authors: serde_json::from_str::<Vec<ArticleAuthorDraft>>(&authors_json)?,
                    start_page,
                    end_page,
                    abstract_text,
                    doi,
                    pmid,
                    open_access: open_access.map(|value| value != 0),
                    in_press: in_press.map(|value| value != 0),
                    retraction_doi,
                },
                issue_id,
            ))
        },
    )
    .transpose()
}

fn upsert_canonical_article(
    connection: &Connection,
    article_id: i64,
    journal_id: i64,
    issue_id: Option<i64>,
    article: &ArticleDraft,
) -> Result<(), ContentDatabaseError> {
    let authors_json = serde_json::to_string(&article.authors)?;
    connection.execute(
        "INSERT INTO articles (
             article_id, journal_id, issue_id, title, publication_year, date, authors_json,
             start_page, end_page, abstract_text, doi, pmid, open_access, in_press,
             retraction_doi
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
         )
         ON CONFLICT(article_id) DO UPDATE SET
             journal_id = excluded.journal_id,
             issue_id = excluded.issue_id,
             title = excluded.title,
             publication_year = excluded.publication_year,
             date = excluded.date,
             authors_json = excluded.authors_json,
             start_page = excluded.start_page,
             end_page = excluded.end_page,
             abstract_text = excluded.abstract_text,
             doi = excluded.doi,
             pmid = excluded.pmid,
             open_access = excluded.open_access,
             in_press = excluded.in_press,
             retraction_doi = excluded.retraction_doi",
        params![
            article_id,
            journal_id,
            issue_id,
            article.title,
            article.publication_year,
            article.date,
            authors_json,
            article.start_page,
            article.end_page,
            article.abstract_text,
            article.doi,
            article.pmid,
            article.open_access.map(i64::from),
            article.in_press.map(i64::from),
            article.retraction_doi,
        ],
    )?;
    Ok(())
}

fn refresh_article_projection(
    connection: &Connection,
    article_id: i64,
    journal_id: i64,
    issue_id: Option<i64>,
    catalog: &JournalCatalogEntry,
    article: &ArticleDraft,
) -> Result<(), ContentDatabaseError> {
    connection.execute(
        "INSERT INTO article_listing (
             article_id, journal_id, issue_id, publication_year, date, open_access,
             in_press, doi, pmid, area
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(article_id) DO UPDATE SET
             journal_id = excluded.journal_id,
             issue_id = excluded.issue_id,
             publication_year = excluded.publication_year,
             date = excluded.date,
             open_access = excluded.open_access,
             in_press = excluded.in_press,
             doi = excluded.doi,
             pmid = excluded.pmid,
             area = excluded.area",
        params![
            article_id,
            journal_id,
            issue_id,
            article.publication_year,
            article.date,
            article.open_access.map(i64::from),
            article.in_press.map(i64::from),
            article.doi,
            article.pmid,
            catalog.area,
        ],
    )?;
    connection.execute("DELETE FROM article_search WHERE rowid = ?1", [article_id])?;
    let authors = article
        .authors
        .iter()
        .map(|author| author.display_name.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    connection.execute(
        "INSERT INTO article_search (
             rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
         ) VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            article_id,
            article.title,
            article.abstract_text.as_deref().unwrap_or_default(),
            article.doi.as_deref().unwrap_or_default(),
            article.pmid.as_deref().unwrap_or_default(),
            authors,
            catalog.title,
        ],
    )?;
    Ok(())
}

fn refresh_journal_projections(
    connection: &Connection,
    journal_id: i64,
    catalog: &JournalCatalogEntry,
) -> Result<(), ContentDatabaseError> {
    connection.execute(
        "UPDATE article_listing SET area = ?1 WHERE journal_id = ?2",
        params![catalog.area, journal_id],
    )?;
    connection.execute(
        "UPDATE article_search SET journal_title = ?1 WHERE article_id IN (
             SELECT article_id FROM articles WHERE journal_id = ?2
         )",
        params![catalog.title, journal_id],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_content_change_event(
    connection: &Connection,
    content_revision: &str,
    article_id: i64,
    change_kind: &str,
    journal_id: i64,
    issue_id: Option<i64>,
    in_press: bool,
    created_at: &str,
) -> Result<usize, ContentDatabaseError> {
    Ok(connection.execute(
        "INSERT OR IGNORE INTO article_change_events (
             content_revision, article_id, change_kind, journal_id, issue_id, in_press, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            content_revision,
            article_id,
            change_kind,
            journal_id,
            issue_id,
            i64::from(in_press),
            created_at,
        ],
    )?)
}

/// Open and initialize an index SQLite database.
///
/// # Arguments
///
/// * `path` - SQLite database path.
///
/// # Returns
///
/// Open initialized SQLite connection.
pub fn open_index_db(path: impl AsRef<Path>) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(INDEX_BUSY_TIMEOUT_SECONDS))?;
    init_index_db(&connection)?;
    Ok(connection)
}

/// Run SQLite index maintenance after an index run.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
///
/// # Returns
///
/// SQLite result.
pub fn optimize_index_db(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("PRAGMA optimize;")
}

/// Initialize the index database schema used by Python-compatible readers.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
///
/// # Returns
///
/// SQLite result.
pub fn init_index_db(connection: &Connection) -> rusqlite::Result<()> {
    let simple_tokenizer_path = resolve_simple_tokenizer_path(connection)?;
    init_index_db_with_simple_tokenizer_path(connection, simple_tokenizer_path.as_deref())
}

fn init_index_db_with_simple_tokenizer_path(
    connection: &Connection,
    simple_tokenizer_path: Option<&Path>,
) -> rusqlite::Result<()> {
    let existing_simple_tokenizer = article_search_uses_simple_tokenizer(connection)?;
    if existing_simple_tokenizer != Some(false) {
        let path = simple_tokenizer_path.ok_or_else(missing_simple_tokenizer_error)?;
        load_simple_tokenizer(connection, path)?;
        if existing_simple_tokenizer == Some(true) {
            probe_article_search(connection)?;
        }
    }

    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS journals (
            journal_id INTEGER PRIMARY KEY,
            library_id TEXT NOT NULL,
            platform_journal_id TEXT,
            title TEXT,
            issn TEXT,
            eissn TEXT,
            scimago_rank REAL,
            cover_url TEXT,
            available INTEGER,
            toc_data_approved_and_live INTEGER,
            has_articles INTEGER
        );

        CREATE TABLE IF NOT EXISTS journal_meta (
            journal_id INTEGER PRIMARY KEY,
            source_csv TEXT NOT NULL,
            area TEXT,
            csv_title TEXT,
            csv_issn TEXT,
            csv_library TEXT,
            resolved_source TEXT,
            resolved_source_id TEXT,
            resolved_title TEXT,
            resolved_issn TEXT,
            resolved_eissn TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS issues (
            issue_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            publication_year INTEGER,
            title TEXT,
            volume TEXT,
            number TEXT,
            date TEXT,
            is_valid_issue INTEGER,
            suppressed INTEGER,
            embargoed INTEGER,
            within_subscription INTEGER,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS articles (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            title TEXT,
            date TEXT,
            authors TEXT,
            start_page TEXT,
            end_page TEXT,
            abstract TEXT,
            doi TEXT,
            pmid TEXT,
            permalink TEXT,
            suppressed INTEGER,
            in_press INTEGER,
            open_access INTEGER,
            platform_id TEXT,
            retraction_doi TEXT,
            within_library_holdings INTEGER,
            content_location TEXT,
            full_text_file TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE,
            FOREIGN KEY (issue_id) REFERENCES issues(issue_id)
                ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS article_listing (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            publication_year INTEGER,
            date TEXT,
            open_access INTEGER,
            in_press INTEGER,
            suppressed INTEGER,
            within_library_holdings INTEGER,
            doi TEXT,
            pmid TEXT,
            area TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE,
            FOREIGN KEY (issue_id) REFERENCES issues(issue_id)
                ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS listing_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            status TEXT NOT NULL,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS journal_year_state (
            journal_id INTEGER NOT NULL,
            year INTEGER NOT NULL,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (journal_id, year)
        );

        CREATE TABLE IF NOT EXISTS journal_state (
            journal_id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS index_runs (
            run_id TEXT PRIMARY KEY,
            csv_file TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            status TEXT NOT NULL,
            total_journals INTEGER NOT NULL,
            succeeded_journals INTEGER NOT NULL,
            failed_journals INTEGER NOT NULL,
            resumed_journals INTEGER NOT NULL,
            error_summary TEXT
        );

        CREATE TABLE IF NOT EXISTS index_run_lease (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            run_id TEXT NOT NULL,
            heartbeat_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS index_path_stats (
            run_id TEXT NOT NULL,
            source TEXT NOT NULL,
            path TEXT NOT NULL,
            journal_id INTEGER,
            journal_title TEXT,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            works_count INTEGER NOT NULL,
            issues_count INTEGER NOT NULL,
            article_summaries_count INTEGER NOT NULL,
            article_details_count INTEGER NOT NULL,
            articles_written_count INTEGER NOT NULL,
            articles_deleted_no_authors_count INTEGER NOT NULL,
            error_type TEXT,
            error_message TEXT,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS index_api_call_stats (
            run_id TEXT NOT NULL,
            source TEXT NOT NULL,
            service TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            method TEXT NOT NULL,
            url_path TEXT NOT NULL,
            journal_id INTEGER,
            journal_title TEXT,
            logical_calls INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            successes INTEGER NOT NULL,
            failures INTEGER NOT NULL,
            retry_count INTEGER NOT NULL,
            status_codes_json TEXT NOT NULL,
            transport_errors INTEGER NOT NULL,
            rate_limit_failures INTEGER NOT NULL,
            total_latency_ms INTEGER NOT NULL,
            error_samples_json TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS index_change_events (
            event_id INTEGER PRIMARY KEY,
            run_id TEXT NOT NULL,
            worker_id TEXT NOT NULL DEFAULT '',
            article_id INTEGER NOT NULL,
            event_type TEXT NOT NULL CHECK (event_type IN ('add', 'remove')),
            membership_type TEXT NOT NULL
                CHECK (membership_type IN ('issue', 'inpress')),
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            is_backfill INTEGER NOT NULL DEFAULT 0 CHECK (is_backfill IN (0, 1)),
            created_at TEXT NOT NULL,
            CHECK (
                (membership_type = 'issue' AND issue_id IS NOT NULL)
                OR (membership_type = 'inpress' AND issue_id IS NULL)
            )
        );
        ",
    )?;
    create_article_search(connection)?;
    if existing_simple_tokenizer.is_none() {
        probe_article_search(connection)?;
    }
    create_runtime_indexes(connection)
}

fn create_article_search(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS article_search
        USING fts5(
            article_id UNINDEXED,
            title,
            abstract,
            doi,
            authors,
            journal_title,
            tokenize = 'simple'
        );
        ",
    )
}

fn create_runtime_indexes(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_journals_issn ON journals(issn);
        CREATE INDEX IF NOT EXISTS idx_journals_library_id ON journals(library_id);
        CREATE INDEX IF NOT EXISTS idx_journals_available ON journals(available);
        CREATE INDEX IF NOT EXISTS idx_journals_has_articles ON journals(has_articles);
        CREATE INDEX IF NOT EXISTS idx_journals_scimago_rank ON journals(scimago_rank);

        CREATE INDEX IF NOT EXISTS idx_journal_meta_area ON journal_meta(area);
        CREATE INDEX IF NOT EXISTS idx_journal_meta_area_journal
            ON journal_meta(area, journal_id);

        CREATE INDEX IF NOT EXISTS idx_issues_journal_year
            ON issues(journal_id, publication_year);
        CREATE INDEX IF NOT EXISTS idx_issues_publication_year
            ON issues(publication_year);

        CREATE INDEX IF NOT EXISTS idx_articles_journal ON articles(journal_id);
        CREATE INDEX IF NOT EXISTS idx_articles_issue ON articles(issue_id);
        CREATE INDEX IF NOT EXISTS idx_articles_date ON articles(date);
        CREATE INDEX IF NOT EXISTS idx_articles_date_id ON articles(date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_journal_date_id
            ON articles(journal_id, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_issue_date_id
            ON articles(issue_id, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_open_access ON articles(open_access);
        CREATE INDEX IF NOT EXISTS idx_articles_open_access_date_id
            ON articles(open_access, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_in_press ON articles(in_press);
        CREATE INDEX IF NOT EXISTS idx_articles_in_press_date_id
            ON articles(in_press, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_suppressed ON articles(suppressed);
        CREATE INDEX IF NOT EXISTS idx_articles_suppressed_date_id
            ON articles(suppressed, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_within_holdings
            ON articles(within_library_holdings);
        CREATE INDEX IF NOT EXISTS idx_articles_within_holdings_date_id
            ON articles(within_library_holdings, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_articles_doi ON articles(doi);
        CREATE INDEX IF NOT EXISTS idx_articles_pmid ON articles(pmid);

        CREATE INDEX IF NOT EXISTS idx_article_listing_date_id
            ON article_listing(date, article_id);
        CREATE INDEX IF NOT EXISTS idx_article_listing_area ON article_listing(area);
        CREATE INDEX IF NOT EXISTS idx_article_listing_area_date_id
            ON article_listing(area, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_article_listing_publication_year
            ON article_listing(publication_year);
        CREATE INDEX IF NOT EXISTS idx_article_listing_journal
            ON article_listing(journal_id);
        CREATE INDEX IF NOT EXISTS idx_article_listing_journal_date_id
            ON article_listing(journal_id, date, article_id);
        CREATE INDEX IF NOT EXISTS idx_article_listing_issue ON article_listing(issue_id);

        CREATE INDEX IF NOT EXISTS idx_index_api_call_stats_run
            ON index_api_call_stats(run_id);

        CREATE UNIQUE INDEX IF NOT EXISTS idx_index_change_events_identity
            ON index_change_events(
                run_id, article_id, event_type, membership_type, journal_id,
                COALESCE(issue_id, -1)
            );
        CREATE INDEX IF NOT EXISTS idx_index_change_events_run_order
            ON index_change_events(run_id, event_id);
        CREATE INDEX IF NOT EXISTS idx_index_change_events_run_membership
            ON index_change_events(
                run_id, membership_type, journal_id, issue_id, event_id
            );
        CREATE INDEX IF NOT EXISTS idx_index_change_events_run_article
            ON index_change_events(run_id, article_id, event_id);
        ",
    )
}

fn load_simple_tokenizer(connection: &Connection, path: &Path) -> rusqlite::Result<()> {
    let _guard = unsafe { LoadExtensionGuard::new(connection)? };
    unsafe { connection.load_extension(path, None::<&str>) }
        .map_err(|error| simple_tokenizer_load_error(path, error))
}

fn simple_tokenizer_load_error(path: &Path, error: rusqlite::Error) -> rusqlite::Error {
    let detail = error.to_string();
    match error {
        rusqlite::Error::SqliteFailure(code, _) => rusqlite::Error::SqliteFailure(
            code,
            Some(format!(
                "failed to load SQLite simple tokenizer {}: {detail}",
                path.display()
            )),
        ),
        other => other,
    }
}

fn article_search_uses_simple_tokenizer(connection: &Connection) -> rusqlite::Result<Option<bool>> {
    let sql = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'article_search'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(sql.map(|value| {
        let compact = value
            .to_ascii_lowercase()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        compact.contains("tokenize='simple'")
            || compact.contains("tokenize=\"simple\"")
            || compact.contains("tokenize=simple")
    }))
}

fn probe_article_search(connection: &Connection) -> rusqlite::Result<()> {
    connection
        .query_row(
            "SELECT rowid FROM article_search WHERE article_search MATCH ?1 LIMIT 1",
            ["litradartokenizerprobe"],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(())
}

fn missing_simple_tokenizer_error() -> rusqlite::Error {
    rusqlite::Error::InvalidPath(expected_simple_tokenizer_path())
}

fn expected_simple_tokenizer_path() -> PathBuf {
    let libs_dir = PathBuf::from("libs");
    if cfg!(windows) {
        libs_dir
            .join("simple-windows")
            .join("libsimple-windows-x64")
            .join("simple.dll")
    } else if cfg!(target_os = "linux") {
        libs_dir
            .join("simple-linux")
            .join("libsimple-linux-ubuntu-latest")
            .join("libsimple.so")
    } else {
        libs_dir.join("simple-tokenizer-extension")
    }
}

fn resolve_simple_tokenizer_path(connection: &Connection) -> rusqlite::Result<Option<PathBuf>> {
    for root in simple_tokenizer_root_candidates(connection)? {
        if let Some(path) = simple_tokenizer_path_from_root(&root) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn simple_tokenizer_root_candidates(connection: &Connection) -> rusqlite::Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if let Some(database_path) = main_database_path(connection)? {
        push_path_ancestors(&mut roots, &database_path);
    }
    if let Ok(current_dir) = std::env::current_dir() {
        push_path_ancestors(&mut roots, &current_dir);
    }
    Ok(roots)
}

fn main_database_path(connection: &Connection) -> rusqlite::Result<Option<PathBuf>> {
    let mut statement = connection.prepare("PRAGMA database_list")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (name, path) = row?;
        if name == "main" && !path.trim().is_empty() {
            return Ok(Some(PathBuf::from(path)));
        }
    }
    Ok(None)
}

fn push_path_ancestors(roots: &mut Vec<PathBuf>, path: &Path) {
    let start = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    for ancestor in start.ancestors() {
        let candidate = ancestor.to_path_buf();
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
}

fn simple_tokenizer_path_from_root(root: &Path) -> Option<PathBuf> {
    let libs_dir = root.join("libs");
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

/// Execute one index write unit in an immediate SQLite transaction.
pub(crate) fn with_immediate_index_transaction<T, E, F>(
    connection: &Connection,
    operation: F,
) -> Result<T, E>
where
    E: From<rusqlite::Error>,
    F: FnOnce(&Transaction<'_>) -> Result<T, E>,
{
    let transaction =
        Transaction::new_unchecked(connection, TransactionBehavior::Immediate).map_err(E::from)?;
    let value = operation(&transaction)?;
    transaction.commit().map_err(E::from)?;
    Ok(value)
}

/// Acquire a durable live index run lease and create its running parent row.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `request` - Run identity, timing, expected work, and recovery policy.
///
/// # Returns
///
/// Reclaimed run identity and adopted source event count.
pub(crate) fn begin_index_run(
    connection: &Connection,
    request: &IndexRunStartRequest<'_>,
) -> Result<IndexRunStartOutcome, IndexRunLeaseError> {
    with_immediate_index_transaction(connection, |transaction| {
        let existing_lease = transaction
            .query_row(
                "SELECT run_id, expires_at FROM index_run_lease WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        if let Some((run_id, expires_at)) = &existing_lease {
            if *expires_at > request.now_epoch_seconds {
                return Err(IndexRunLeaseError::ActiveLease {
                    run_id: run_id.clone(),
                    expires_at: *expires_at,
                });
            }
        }

        let interrupted_run_id = existing_lease.map(|(run_id, _)| run_id);
        if let Some(run_id) = &interrupted_run_id {
            transaction.execute(
                "
                UPDATE index_runs
                SET finished_at = ?2,
                    status = 'interrupted',
                    error_summary = COALESCE(
                        error_summary,
                        'run lease expired and was reclaimed'
                    )
                WHERE run_id = ?1 AND status = 'running'
                ",
                params![run_id, request.started_at],
            )?;
        }

        transaction.execute(
            "
            INSERT INTO index_runs (
                run_id, csv_file, started_at, finished_at, status,
                total_journals, succeeded_journals, failed_journals,
                resumed_journals, error_summary
            ) VALUES (?1, ?2, ?3, NULL, 'running', ?4, 0, 0, 0, NULL)
            ",
            params![
                request.run_id,
                request.csv_file,
                request.started_at,
                request.total_journals,
            ],
        )?;
        transaction.execute(
            "
            INSERT INTO index_run_lease (id, run_id, heartbeat_at, expires_at)
            VALUES (1, ?1, ?2, ?3)
            ON CONFLICT(id) DO UPDATE SET
                run_id = excluded.run_id,
                heartbeat_at = excluded.heartbeat_at,
                expires_at = excluded.expires_at
            ",
            params![
                request.run_id,
                request.now_epoch_seconds,
                lease_expiry(request.now_epoch_seconds),
            ],
        )?;
        let adopted_event_count = if request.should_adopt_events {
            adopt_pending_change_events(transaction, request.run_id)?
        } else {
            0
        };

        Ok(IndexRunStartOutcome {
            interrupted_run_id,
            adopted_event_count,
        })
    })
}

/// Renew one live index run lease when it remains current and unexpired.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `run_id` - Expected current run identifier.
/// * `now_epoch_seconds` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Empty result when the lease was renewed.
pub(crate) fn heartbeat_index_run_lease(
    connection: &Connection,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), IndexRunLeaseError> {
    let updated = connection.execute(
        "
        UPDATE index_run_lease
        SET heartbeat_at = ?2, expires_at = ?3
        WHERE id = 1 AND run_id = ?1 AND expires_at > ?2
        ",
        params![run_id, now_epoch_seconds, lease_expiry(now_epoch_seconds)],
    )?;
    if updated == 1 {
        Ok(())
    } else {
        Err(IndexRunLeaseError::OwnershipLost {
            run_id: run_id.to_string(),
        })
    }
}

/// Refresh the exact run owner after an atomic acquisition transaction.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `run_id` - Exact run identifier installed by the acquisition transaction.
/// * `now_epoch_seconds` - Current Unix timestamp in seconds after acquisition commits.
///
/// # Returns
///
/// Empty result when the exact owner was refreshed, including after initial expiry.
pub(crate) fn refresh_index_run_lease_after_acquisition(
    connection: &Connection,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), IndexRunLeaseError> {
    let updated = connection.execute(
        "
        UPDATE index_run_lease
        SET heartbeat_at = ?2, expires_at = ?3
        WHERE id = 1 AND run_id = ?1
        ",
        params![run_id, now_epoch_seconds, lease_expiry(now_epoch_seconds)],
    )?;
    if updated == 1 {
        Ok(())
    } else {
        Err(IndexRunLeaseError::OwnershipLost {
            run_id: run_id.to_string(),
        })
    }
}

/// Assert that one live index run owns an unexpired lease.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection or write transaction.
/// * `run_id` - Expected current run identifier.
/// * `now_epoch_seconds` - Current Unix timestamp in seconds.
///
/// # Returns
///
/// Empty result when the exact run owns an unexpired lease.
pub(crate) fn assert_index_run_lease_owner(
    connection: &Connection,
    run_id: &str,
    now_epoch_seconds: i64,
) -> Result<(), IndexRunLeaseError> {
    let is_owner = connection.query_row(
        "
        SELECT EXISTS(
            SELECT 1 FROM index_run_lease
            WHERE id = 1 AND run_id = ?1 AND expires_at > ?2
        )
        ",
        params![run_id, now_epoch_seconds],
        |row| row.get::<_, bool>(0),
    )?;
    if is_owner {
        Ok(())
    } else {
        Err(IndexRunLeaseError::OwnershipLost {
            run_id: run_id.to_string(),
        })
    }
}

/// Release one live index lease only when its run identifier matches.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `run_id` - Expected current run identifier.
///
/// # Returns
///
/// Whether the exact owner's lease row was deleted.
pub(crate) fn release_index_run_lease(
    connection: &Connection,
    run_id: &str,
) -> rusqlite::Result<bool> {
    Ok(connection.execute(
        "DELETE FROM index_run_lease WHERE id = 1 AND run_id = ?1",
        [run_id],
    )? == 1)
}

fn lease_expiry(now_epoch_seconds: i64) -> i64 {
    now_epoch_seconds.saturating_add(INDEX_RUN_LEASE_DURATION_SECONDS)
}

fn current_epoch_seconds() -> Result<i64, IndexRunLeaseError> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(i64::try_from(seconds).unwrap_or(i64::MAX))
}

fn adopt_pending_change_events(
    connection: &Connection,
    current_run_id: &str,
) -> rusqlite::Result<usize> {
    let mut last_event_id = None;
    let mut adopted_event_count = 0;
    loop {
        let events = pending_change_event_batch(connection, current_run_id, last_event_id)?;
        let Some(next_last_event_id) = events.last().map(|event| event.event_id) else {
            return Ok(adopted_event_count);
        };
        for event in events {
            let deleted = connection.execute(
                "DELETE FROM index_change_events WHERE event_id = ?1 AND run_id <> ?2",
                params![event.event_id, current_run_id],
            )?;
            if deleted != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            replay_pending_change_event(connection, current_run_id, event)?;
            adopted_event_count += 1;
        }
        last_event_id = Some(next_last_event_id);
    }
}

fn pending_change_event_batch(
    connection: &Connection,
    current_run_id: &str,
    last_event_id: Option<i64>,
) -> rusqlite::Result<Vec<PendingChangeEvent>> {
    let mut statement = connection.prepare(
        "
        SELECT
            event_id, worker_id, article_id, event_type, membership_type,
            journal_id, issue_id, is_backfill, created_at
        FROM index_change_events
        WHERE run_id <> ?1 AND (?2 IS NULL OR event_id > ?2)
        ORDER BY event_id
        LIMIT ?3
        ",
    )?;
    let rows = statement.query_map(
        params![
            current_run_id,
            last_event_id,
            PENDING_EVENT_ADOPTION_BATCH_SIZE
        ],
        |row| {
            Ok(PendingChangeEvent {
                event_id: row.get(0)?,
                worker_id: row.get(1)?,
                article_id: row.get(2)?,
                event_type: row.get(3)?,
                membership_type: row.get(4)?,
                journal_id: row.get(5)?,
                issue_id: row.get(6)?,
                is_backfill: row.get::<_, i64>(7)? == 1,
                created_at: row.get(8)?,
            })
        },
    )?;
    rows.collect()
}

fn replay_pending_change_event(
    connection: &Connection,
    current_run_id: &str,
    event: PendingChangeEvent,
) -> rusqlite::Result<()> {
    let membership_type = match (event.membership_type.as_str(), event.issue_id) {
        ("issue", Some(issue_id)) => ArticleMembership {
            membership_type: "issue",
            journal_id: event.journal_id,
            issue_id: Some(issue_id),
        },
        ("inpress", None) => ArticleMembership {
            membership_type: "inpress",
            journal_id: event.journal_id,
            issue_id: None,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    if !matches!(event.event_type.as_str(), "add" | "remove") {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let context = ChangeEventContext::new(
        current_run_id,
        event.worker_id,
        event.created_at,
        event.is_backfill,
    );
    record_membership_event(
        connection,
        &context,
        event.article_id,
        &event.event_type,
        &membership_type,
    )
}

/// Insert or update a journal record.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `record` - Journal record.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_journal(connection: &Connection, record: &JournalRecord) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journals (
            journal_id, library_id, platform_journal_id, title, issn, eissn,
            scimago_rank, cover_url, available, toc_data_approved_and_live,
            has_articles
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(journal_id) DO UPDATE SET
            library_id = excluded.library_id,
            platform_journal_id = excluded.platform_journal_id,
            title = excluded.title,
            issn = excluded.issn,
            eissn = excluded.eissn,
            scimago_rank = excluded.scimago_rank,
            cover_url = excluded.cover_url,
            available = excluded.available,
            toc_data_approved_and_live = excluded.toc_data_approved_and_live,
            has_articles = excluded.has_articles
        ",
        params![
            record.journal_id,
            record.library_id,
            record.platform_journal_id,
            record.title,
            record.issn,
            record.eissn,
            record.scimago_rank,
            record.cover_url,
            record.available,
            record.toc_data_approved_and_live,
            record.has_articles,
        ],
    )?;
    Ok(())
}

/// Insert or update journal metadata.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `record` - Metadata record.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_meta(connection: &Connection, record: &MetaRecord) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journal_meta (
            journal_id, source_csv, area, csv_title, csv_issn, csv_library,
            resolved_source, resolved_source_id, resolved_title, resolved_issn,
            resolved_eissn
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(journal_id) DO UPDATE SET
            source_csv = excluded.source_csv,
            area = excluded.area,
            csv_title = excluded.csv_title,
            csv_issn = excluded.csv_issn,
            csv_library = excluded.csv_library,
            resolved_source = excluded.resolved_source,
            resolved_source_id = excluded.resolved_source_id,
            resolved_title = excluded.resolved_title,
            resolved_issn = excluded.resolved_issn,
            resolved_eissn = excluded.resolved_eissn
        ",
        params![
            record.journal_id,
            record.source_csv,
            record.area,
            record.csv_title,
            record.csv_issn,
            record.csv_library,
            record.resolved_source,
            record.resolved_source_id,
            record.resolved_title,
            record.resolved_issn,
            record.resolved_eissn,
        ],
    )?;
    Ok(())
}

/// Synchronize one CSV-owned journal catalog entry without overwriting provider data.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection or transaction.
/// * `journal` - Neutral journal shell used only when the journal is missing.
/// * `metadata` - CSV-owned metadata projection to insert or refresh.
///
/// # Returns
///
/// Number of inserted or updated rows across `journals` and `journal_meta`.
pub(crate) fn sync_journal_catalog_entry(
    connection: &Connection,
    journal: &JournalRecord,
    metadata: &MetaRecord,
) -> rusqlite::Result<usize> {
    let journal_change_count = connection.execute(
        "
        INSERT INTO journals (
            journal_id, library_id, platform_journal_id, title, issn, eissn,
            scimago_rank, cover_url, available, toc_data_approved_and_live,
            has_articles
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(journal_id) DO NOTHING
        ",
        params![
            journal.journal_id,
            journal.library_id,
            journal.platform_journal_id,
            journal.title,
            journal.issn,
            journal.eissn,
            journal.scimago_rank,
            journal.cover_url,
            journal.available,
            journal.toc_data_approved_and_live,
            journal.has_articles,
        ],
    )?;
    let metadata_change_count = connection.execute(
        "
        INSERT INTO journal_meta (
            journal_id, source_csv, area, csv_title, csv_issn, csv_library,
            resolved_source, resolved_source_id, resolved_title, resolved_issn,
            resolved_eissn
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL, NULL)
        ON CONFLICT(journal_id) DO UPDATE SET
            source_csv = excluded.source_csv,
            area = excluded.area,
            csv_title = excluded.csv_title,
            csv_issn = excluded.csv_issn,
            csv_library = excluded.csv_library
        WHERE journal_meta.source_csv IS NOT excluded.source_csv
           OR journal_meta.area IS NOT excluded.area
           OR journal_meta.csv_title IS NOT excluded.csv_title
           OR journal_meta.csv_issn IS NOT excluded.csv_issn
           OR journal_meta.csv_library IS NOT excluded.csv_library
        ",
        params![
            metadata.journal_id,
            metadata.source_csv,
            metadata.area,
            metadata.csv_title,
            metadata.csv_issn,
            metadata.csv_library,
        ],
    )?;
    Ok(journal_change_count + metadata_change_count)
}

/// Check that one journal and its CSV-owned metadata projection are current.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection or transaction.
/// * `metadata` - Expected CSV-owned metadata projection.
///
/// # Returns
///
/// Whether the journal exists and every CSV-owned metadata field matches.
pub(crate) fn journal_catalog_entry_is_current(
    connection: &Connection,
    metadata: &MetaRecord,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "
        SELECT EXISTS (
            SELECT 1
            FROM journals AS journal
            INNER JOIN journal_meta AS metadata
                ON metadata.journal_id = journal.journal_id
            WHERE journal.journal_id = ?1
              AND metadata.source_csv = ?2
              AND metadata.area IS ?3
              AND metadata.csv_title IS ?4
              AND metadata.csv_issn IS ?5
              AND metadata.csv_library IS ?6
        )
        ",
        params![
            metadata.journal_id,
            metadata.source_csv,
            metadata.area,
            metadata.csv_title,
            metadata.csv_issn,
            metadata.csv_library,
        ],
        |row| row.get::<_, i64>(0).map(|value| value == 1),
    )
}

/// Insert or update issue records.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `records` - Issue records.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_issues(connection: &Connection, records: &[IssueRecord]) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(
        "
        INSERT INTO issues (
            issue_id, journal_id, publication_year, title, volume, number,
            date, is_valid_issue, suppressed, embargoed, within_subscription
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(issue_id) DO UPDATE SET
            journal_id = excluded.journal_id,
            publication_year = excluded.publication_year,
            title = excluded.title,
            volume = excluded.volume,
            number = excluded.number,
            date = excluded.date,
            is_valid_issue = excluded.is_valid_issue,
            suppressed = excluded.suppressed,
            embargoed = excluded.embargoed,
            within_subscription = excluded.within_subscription
        ",
    )?;
    for record in records {
        statement.execute(params![
            record.issue_id,
            record.journal_id,
            record.publication_year,
            record.title,
            record.volume,
            record.number,
            record.date,
            record.is_valid_issue,
            record.suppressed,
            record.embargoed,
            record.within_subscription,
        ])?;
    }
    Ok(())
}

/// Insert or update article records.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `records` - Article records.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_articles(connection: &Connection, records: &[ArticleRecord]) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(
        "
        INSERT INTO articles (
            article_id, journal_id, issue_id, title, date, authors, start_page,
            end_page, abstract, doi, pmid, permalink, suppressed, in_press,
            open_access, platform_id, retraction_doi, within_library_holdings,
            content_location, full_text_file
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?20
        )
        ON CONFLICT(article_id) DO UPDATE SET
            journal_id = excluded.journal_id,
            issue_id = excluded.issue_id,
            title = excluded.title,
            date = excluded.date,
            authors = excluded.authors,
            start_page = excluded.start_page,
            end_page = excluded.end_page,
            abstract = excluded.abstract,
            doi = excluded.doi,
            pmid = excluded.pmid,
            permalink = excluded.permalink,
            suppressed = excluded.suppressed,
            in_press = excluded.in_press,
            open_access = excluded.open_access,
            platform_id = excluded.platform_id,
            retraction_doi = excluded.retraction_doi,
            within_library_holdings = excluded.within_library_holdings,
            content_location = excluded.content_location,
            full_text_file = excluded.full_text_file
        ",
    )?;
    for record in records {
        statement.execute(params![
            record.article_id,
            record.journal_id,
            record.issue_id,
            record.title,
            record.date,
            record.authors,
            record.start_page,
            record.end_page,
            record.abstract_text,
            record.doi,
            record.pmid,
            record.permalink,
            record.suppressed,
            record.in_press,
            record.open_access,
            record.platform_id,
            record.retraction_doi,
            record.within_library_holdings,
            record.content_location,
            record.full_text_file,
        ])?;
    }
    Ok(())
}

/// Apply article, FTS, listing, deletion, and change-event writes together.
pub(crate) fn apply_article_changes(
    connection: &Connection,
    records: &[ArticleRecord],
    deleted_article_ids: &[i64],
    journal_title: &str,
    change_event_context: Option<&ChangeEventContext>,
) -> rusqlite::Result<()> {
    let mut affected_article_ids = records
        .iter()
        .map(|record| record.article_id)
        .chain(deleted_article_ids.iter().copied())
        .collect::<BTreeSet<_>>();
    let before_memberships = if change_event_context.is_some() {
        collect_article_memberships(connection, &affected_article_ids)?
    } else {
        BTreeMap::new()
    };

    if !deleted_article_ids.is_empty() {
        delete_articles(connection, deleted_article_ids)?;
    }
    if !records.is_empty() {
        upsert_articles(connection, records)?;
        upsert_article_search(connection, records, journal_title)?;
        refresh_article_listing_for_articles(
            connection,
            &records
                .iter()
                .map(|record| record.article_id)
                .collect::<Vec<_>>(),
        )?;
    }

    let Some(context) = change_event_context else {
        return Ok(());
    };
    let after_memberships = records
        .iter()
        .filter_map(|record| {
            article_membership(record.journal_id, record.issue_id, record.in_press)
                .map(|membership| (record.article_id, membership))
        })
        .collect::<BTreeMap<_, _>>();
    affected_article_ids.extend(before_memberships.keys().copied());
    affected_article_ids.extend(after_memberships.keys().copied());
    for article_id in affected_article_ids {
        let before = before_memberships.get(&article_id);
        let after = after_memberships.get(&article_id);
        if before == after {
            continue;
        }
        if let Some(membership) = before {
            record_membership_event(connection, context, article_id, "remove", membership)?;
        }
        if let Some(membership) = after {
            record_membership_event(connection, context, article_id, "add", membership)?;
        }
    }
    Ok(())
}

fn collect_article_memberships(
    connection: &Connection,
    article_ids: &BTreeSet<i64>,
) -> rusqlite::Result<BTreeMap<i64, ArticleMembership>> {
    let mut statement = connection
        .prepare("SELECT journal_id, issue_id, in_press FROM articles WHERE article_id = ?1")?;
    let mut memberships = BTreeMap::new();
    for article_id in article_ids {
        let fields = statement
            .query_row([article_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .optional()?;
        if let Some((journal_id, issue_id, in_press)) = fields {
            if let Some(membership) = article_membership(journal_id, issue_id, in_press) {
                memberships.insert(*article_id, membership);
            }
        }
    }
    Ok(memberships)
}

fn article_membership(
    journal_id: i64,
    issue_id: Option<i64>,
    in_press: Option<i64>,
) -> Option<ArticleMembership> {
    if let Some(issue_id) = issue_id {
        return Some(ArticleMembership {
            membership_type: "issue",
            journal_id,
            issue_id: Some(issue_id),
        });
    }
    (in_press == Some(1)).then_some(ArticleMembership {
        membership_type: "inpress",
        journal_id,
        issue_id: None,
    })
}

fn record_membership_event(
    connection: &Connection,
    context: &ChangeEventContext,
    article_id: i64,
    event_type: &str,
    membership: &ArticleMembership,
) -> rusqlite::Result<()> {
    let inverse_event_type = if event_type == "add" { "remove" } else { "add" };
    let deleted_inverse = connection.execute(
        "
        DELETE FROM index_change_events
        WHERE run_id = ?1
          AND article_id = ?2
          AND event_type = ?3
          AND membership_type = ?4
          AND journal_id = ?5
          AND issue_id IS ?6
        ",
        params![
            context.run_id,
            article_id,
            inverse_event_type,
            membership.membership_type,
            membership.journal_id,
            membership.issue_id,
        ],
    )?;
    if deleted_inverse > 0 {
        return Ok(());
    }
    connection.execute(
        "
        INSERT OR IGNORE INTO index_change_events (
            run_id, worker_id, article_id, event_type, membership_type,
            journal_id, issue_id, is_backfill, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ",
        params![
            context.run_id,
            context.worker_id,
            article_id,
            event_type,
            membership.membership_type,
            membership.journal_id,
            membership.issue_id,
            context.is_backfill as i64,
            context.created_at,
        ],
    )?;
    Ok(())
}

/// Delete article records and derived rows.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `article_ids` - Article ids to delete.
///
/// # Returns
///
/// SQLite result.
pub fn delete_articles(connection: &Connection, article_ids: &[i64]) -> rusqlite::Result<()> {
    let mut delete_search = connection.prepare("DELETE FROM article_search WHERE rowid = ?1")?;
    let mut delete_listing =
        connection.prepare("DELETE FROM article_listing WHERE article_id = ?1")?;
    let mut delete_article = connection.prepare("DELETE FROM articles WHERE article_id = ?1")?;
    for article_id in article_ids {
        delete_search.execute([article_id])?;
        delete_listing.execute([article_id])?;
        delete_article.execute([article_id])?;
    }
    Ok(())
}

/// Insert or update article search rows.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `records` - Article records.
/// * `journal_title` - Journal title.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_article_search(
    connection: &Connection,
    records: &[ArticleRecord],
    journal_title: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(
        "
        INSERT OR REPLACE INTO article_search (
            rowid, article_id, title, abstract, doi, authors, journal_title
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
    )?;
    for record in records {
        statement.execute(params![
            record.article_id,
            record.article_id,
            record.title.as_deref().unwrap_or(""),
            record.abstract_text.as_deref().unwrap_or(""),
            record.doi.as_deref().unwrap_or(""),
            record.authors.as_deref().unwrap_or(""),
            journal_title,
        ])?;
    }
    Ok(())
}

/// Refresh article listing rows for article ids.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `article_ids` - Article ids.
///
/// # Returns
///
/// SQLite result.
pub fn refresh_article_listing_for_articles(
    connection: &Connection,
    article_ids: &[i64],
) -> rusqlite::Result<()> {
    if article_ids.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", article_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        INSERT INTO article_listing (
            article_id, journal_id, issue_id, publication_year, date, open_access,
            in_press, suppressed, within_library_holdings, doi, pmid, area
        )
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            i.publication_year,
            a.date,
            a.open_access,
            a.in_press,
            a.suppressed,
            a.within_library_holdings,
            a.doi,
            a.pmid,
            m.area
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        LEFT JOIN journal_meta m ON m.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
        ON CONFLICT(article_id) DO UPDATE SET
            journal_id = excluded.journal_id,
            issue_id = excluded.issue_id,
            publication_year = excluded.publication_year,
            date = excluded.date,
            open_access = excluded.open_access,
            in_press = excluded.in_press,
            suppressed = excluded.suppressed,
            within_library_holdings = excluded.within_library_holdings,
            doi = excluded.doi,
            pmid = excluded.pmid,
            area = excluded.area
        "
    );
    connection.execute(&sql, params_from_iter(article_ids.iter()))?;
    Ok(())
}

/// Fetch issue ids that already have articles for a journal.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
///
/// # Returns
///
/// Issue ids with existing articles.
pub fn get_journal_issue_ids_with_articles(
    connection: &Connection,
    journal_id: i64,
) -> rusqlite::Result<BTreeSet<i64>> {
    let mut statement = connection.prepare(
        "
        SELECT DISTINCT a.issue_id
        FROM articles a
        JOIN issues i ON i.issue_id = a.issue_id
        WHERE i.journal_id = ?1
        ",
    )?;
    let rows = statement.query_map([journal_id], |row| row.get::<_, Option<i64>>(0))?;
    let mut issue_ids = BTreeSet::new();
    for row in rows {
        if let Some(issue_id) = row? {
            issue_ids.insert(issue_id);
        }
    }
    Ok(issue_ids)
}

/// Fetch completed journal years.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
///
/// # Returns
///
/// Completed years for the journal.
pub fn get_completed_years(
    connection: &Connection,
    journal_id: i64,
) -> rusqlite::Result<BTreeSet<i64>> {
    let mut statement = connection
        .prepare("SELECT year FROM journal_year_state WHERE journal_id = ?1 AND status = 'done'")?;
    let rows = statement.query_map([journal_id], |row| row.get::<_, i64>(0))?;
    let mut years = BTreeSet::new();
    for row in rows {
        years.insert(row?);
    }
    Ok(years)
}

/// Check whether a journal is marked complete.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
///
/// # Returns
///
/// Whether the journal is complete.
pub fn is_journal_complete(connection: &Connection, journal_id: i64) -> rusqlite::Result<bool> {
    let status = connection.query_row(
        "SELECT status FROM journal_state WHERE journal_id = ?1",
        [journal_id],
        |row| row.get::<_, String>(0),
    );
    match status {
        Ok(value) => Ok(value == "done"),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Return the overlapped synchronization date for a completed journal.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
/// * `current_timestamp` - Current run timestamp as Unix seconds or SQLite-compatible ISO text.
///
/// # Returns
///
/// Prior completion date minus 30 days, or `None` for missing, invalid, or future state.
pub(crate) fn get_journal_synchronization_date(
    connection: &Connection,
    journal_id: i64,
    current_timestamp: &str,
) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "
            WITH normalized AS (
                SELECT
                    status,
                    CASE
                        WHEN trim(updated_at) <> ''
                            AND trim(updated_at) NOT GLOB '*[^0-9]*'
                        THEN datetime(CAST(trim(updated_at) AS INTEGER), 'unixepoch')
                        ELSE datetime(updated_at)
                    END AS checkpoint_at,
                    CASE
                        WHEN trim(?2) <> '' AND trim(?2) NOT GLOB '*[^0-9]*'
                        THEN datetime(CAST(trim(?2) AS INTEGER), 'unixepoch')
                        ELSE datetime(?2)
                    END AS current_at
                FROM journal_state
                WHERE journal_id = ?1
            )
            SELECT date(checkpoint_at, '-30 days')
            FROM normalized
            WHERE status = 'done'
                AND checkpoint_at IS NOT NULL
                AND current_at IS NOT NULL
                AND checkpoint_at <= current_at
            ",
            params![journal_id, current_timestamp],
            |row| row.get(0),
        )
        .optional()
}

/// Mark one journal year as indexed.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
/// * `year` - Publication year.
/// * `updated_at` - Updated timestamp.
///
/// # Returns
///
/// SQLite result.
pub fn mark_year_done(
    connection: &Connection,
    journal_id: i64,
    year: i64,
    updated_at: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journal_year_state (journal_id, year, status, updated_at)
        VALUES (?1, ?2, 'done', ?3)
        ON CONFLICT(journal_id, year) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        ",
        params![journal_id, year, updated_at],
    )?;
    Ok(())
}

/// Mark one journal as indexed.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
/// * `updated_at` - Updated timestamp.
///
/// # Returns
///
/// SQLite result.
pub fn mark_journal_done(
    connection: &Connection,
    journal_id: i64,
    updated_at: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journal_state (journal_id, status, updated_at)
        VALUES (?1, 'done', ?2)
        ON CONFLICT(journal_id) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        ",
        params![journal_id, updated_at],
    )?;
    Ok(())
}

/// Mark article listing rows as ready for reader queries.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `updated_at` - Updated timestamp.
///
/// # Returns
///
/// SQLite result.
pub fn mark_article_listing_ready(
    connection: &Connection,
    updated_at: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO listing_state (id, status, updated_at)
        VALUES (1, 'ready', ?1)
        ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        ",
        [updated_at],
    )?;
    Ok(())
}

/// Persist index run statistics.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `stats` - Index run statistics.
///
/// # Returns
///
/// SQLite result.
pub fn persist_index_run_stats(
    connection: &Connection,
    stats: &IndexRunStats,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO index_runs (
            run_id, csv_file, started_at, finished_at, status, total_journals,
            succeeded_journals, failed_journals, resumed_journals, error_summary
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(run_id) DO UPDATE SET
            csv_file = excluded.csv_file,
            started_at = excluded.started_at,
            finished_at = excluded.finished_at,
            status = excluded.status,
            total_journals = excluded.total_journals,
            succeeded_journals = excluded.succeeded_journals,
            failed_journals = excluded.failed_journals,
            resumed_journals = excluded.resumed_journals,
            error_summary = excluded.error_summary
        ",
        params![
            stats.run_id,
            stats.csv_file,
            stats.started_at,
            stats.finished_at,
            stats.status,
            stats.total_journals,
            stats.succeeded_journals,
            stats.failed_journals,
            stats.resumed_journals,
            stats.error_summary,
        ],
    )?;
    connection.execute(
        "DELETE FROM index_path_stats WHERE run_id = ?1",
        [&stats.run_id],
    )?;
    connection.execute(
        "DELETE FROM index_api_call_stats WHERE run_id = ?1",
        [&stats.run_id],
    )?;
    for path_stats in stats.path_stats.values() {
        connection.execute(
            "
            INSERT INTO index_path_stats (
                run_id, source, path, journal_id, journal_title, status,
                started_at, finished_at, works_count, issues_count,
                article_summaries_count, article_details_count,
                articles_written_count, articles_deleted_no_authors_count,
                error_type, error_message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ",
            params![
                stats.run_id,
                path_stats.key.source,
                path_stats.key.path,
                path_stats.key.journal_id,
                path_stats.key.journal_title,
                path_stats.status,
                path_stats.started_at,
                path_stats.finished_at,
                path_stats.works_count,
                path_stats.issues_count,
                path_stats.article_summaries_count,
                path_stats.article_details_count,
                path_stats.articles_written_count,
                path_stats.articles_deleted_no_authors_count,
                path_stats.error_type,
                path_stats.error_message,
            ],
        )?;
    }
    for api_stats in stats.api_stats.values() {
        let status_codes_json = python_status_codes_json(&api_stats.status_codes);
        let error_samples_json = serde_json::to_string(&api_stats.error_samples)
            .expect("error samples should serialize");
        connection.execute(
            "
            INSERT INTO index_api_call_stats (
                run_id, source, service, endpoint, method, url_path, journal_id,
                journal_title, logical_calls, attempts, successes, failures,
                retry_count, status_codes_json, transport_errors,
                rate_limit_failures, total_latency_ms, error_samples_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            ",
            params![
                stats.run_id,
                api_stats.key.source,
                api_stats.key.service,
                api_stats.key.endpoint,
                api_stats.key.method,
                api_stats.key.url_path,
                api_stats.key.journal_id,
                api_stats.key.journal_title,
                api_stats.logical_calls,
                api_stats.attempts,
                api_stats.successes,
                api_stats.failures,
                api_stats.retry_count,
                status_codes_json,
                api_stats.transport_errors,
                api_stats.rate_limit_failures,
                api_stats.total_latency_ms,
                error_samples_json,
            ],
        )?;
    }
    Ok(())
}

fn python_status_codes_json(status_codes: &std::collections::BTreeMap<u16, i64>) -> String {
    let fields = status_codes
        .iter()
        .map(|(key, value)| format!("\"{key}\": {value}"))
        .collect::<Vec<_>>();
    format!("{{{}}}", fields.join(", "))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use litradar_domain::{
        ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft, JournalRankings, ProviderBatch,
    };
    use litradar_sources::SourceAttempt;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    use crate::stats::{IndexRunStats, PathCountIncrements};
    use crate::transforms::{ArticleRecord, JournalRecord, MetaRecord};

    use super::{
        apply_article_changes, assert_index_run_lease_owner, begin_index_run,
        get_journal_synchronization_date, heartbeat_index_run_lease, init_content_db,
        init_index_db, init_index_db_with_simple_tokenizer_path, journal_catalog_entry_is_current,
        mark_article_listing_ready, mark_journal_done, open_index_db, persist_index_run_stats,
        refresh_article_listing_for_articles, refresh_index_run_lease_after_acquisition,
        release_index_run_lease, simple_tokenizer_path_from_root, sync_journal_catalog_entry,
        upsert_article_search, upsert_articles, upsert_journal, upsert_meta,
        with_immediate_index_transaction, write_content_batch, ChangeEventContext,
        ContentDatabaseError, IndexRunLeaseError, IndexRunStartRequest, CONTENT_SCHEMA_VERSION,
    };

    const RUNTIME_INDEXES: &[&str] = &[
        "idx_article_listing_area",
        "idx_article_listing_area_date_id",
        "idx_article_listing_date_id",
        "idx_article_listing_issue",
        "idx_article_listing_journal",
        "idx_article_listing_journal_date_id",
        "idx_article_listing_publication_year",
        "idx_articles_date",
        "idx_articles_date_id",
        "idx_articles_doi",
        "idx_articles_in_press",
        "idx_articles_in_press_date_id",
        "idx_articles_issue",
        "idx_articles_issue_date_id",
        "idx_articles_journal",
        "idx_articles_journal_date_id",
        "idx_articles_open_access",
        "idx_articles_open_access_date_id",
        "idx_articles_pmid",
        "idx_articles_suppressed",
        "idx_articles_suppressed_date_id",
        "idx_articles_within_holdings",
        "idx_articles_within_holdings_date_id",
        "idx_issues_journal_year",
        "idx_issues_publication_year",
        "idx_journal_meta_area",
        "idx_journal_meta_area_journal",
        "idx_journals_available",
        "idx_journals_has_articles",
        "idx_journals_issn",
        "idx_journals_library_id",
        "idx_journals_scimago_rank",
        "idx_index_change_events_identity",
        "idx_index_change_events_run_article",
        "idx_index_change_events_run_membership",
        "idx_index_change_events_run_order",
    ];

    #[test]
    fn content_v4_schema_has_exact_provider_neutral_inventory() {
        let connection = Connection::open_in_memory().expect("content database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("content version should load");
        assert_eq!(version, CONTENT_SCHEMA_VERSION);

        let tables = connection
            .prepare(
                "SELECT name FROM sqlite_schema
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                   AND name NOT LIKE 'article_search_%'
                 ORDER BY name",
            )
            .expect("schema query should prepare")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("schema query should run")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("schema rows should load");
        assert_eq!(
            tables,
            [
                "article_change_events",
                "article_identity_keys",
                "article_listing",
                "article_search",
                "articles",
                "issues",
                "journals",
            ]
        );

        let schema = connection
            .query_row(
                "SELECT group_concat(sql, '\n') FROM sqlite_schema WHERE sql IS NOT NULL",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema SQL should load")
            .to_ascii_lowercase();
        for forbidden in [
            "library_id",
            "platform_id",
            "source_csv",
            "cover_url",
            "permalink",
            "content_location",
            "full_text_file",
            "checkpoint",
            "lease",
            "index_runs",
            "stats",
        ] {
            assert!(!schema.contains(forbidden), "found {forbidden}");
        }
    }

    #[test]
    fn content_v4_rejects_extra_provider_columns_without_rewriting_schema() {
        let connection = Connection::open_in_memory().expect("content database should open");
        init_content_db(&connection).expect("content schema should initialize");
        connection
            .execute("ALTER TABLE articles ADD COLUMN provider TEXT", [])
            .expect("forbidden fixture column should be added");

        let error = init_content_db(&connection).expect_err("malformed v4 schema should fail");

        assert!(matches!(
            error,
            ContentDatabaseError::InvalidCurrentSchema(_)
        ));
        assert_eq!(
            table_columns(&connection, "articles")
                .last()
                .map(String::as_str),
            Some("provider")
        );
    }

    #[test]
    fn canonical_content_write_replays_without_duplicate_rows_or_events() {
        let connection = Connection::open_in_memory().expect("content database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let catalog = canonical_catalog();
        let initial_batch = canonical_batch(None);

        let first = write_content_batch(
            &connection,
            &catalog,
            &initial_batch,
            "revision-a",
            "2026-07-18T00:00:00Z",
        )
        .expect("first content batch should commit");
        let replay = write_content_batch(
            &connection,
            &catalog,
            &initial_batch,
            "revision-a",
            "2026-07-18T00:00:00Z",
        )
        .expect("identical replay should commit");
        assert_eq!(first.articles_changed, 1);
        assert_eq!(first.change_events_emitted, 1);
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(replay.identity_aliases_added, 0);
        assert_eq!(replay.change_events_emitted, 0);

        let enriched = canonical_batch(Some("10.1000/canonical"));
        let enrichment = write_content_batch(
            &connection,
            &catalog,
            &enriched,
            "revision-b",
            "2026-07-18T00:01:00Z",
        )
        .expect("later DOI should enrich the existing article");
        assert_eq!(enrichment.articles_changed, 1);
        assert_eq!(enrichment.identity_aliases_added, 1);
        assert_eq!(table_count(&connection, "journals"), 1);
        assert_eq!(table_count(&connection, "issues"), 1);
        assert_eq!(table_count(&connection, "articles"), 1);
        assert_eq!(table_count(&connection, "article_identity_keys"), 2);
        assert_eq!(table_count(&connection, "article_listing"), 1);
        assert_eq!(table_count(&connection, "article_search"), 1);
        assert_eq!(table_count(&connection, "article_change_events"), 2);
        let doi = connection
            .query_row("SELECT doi FROM articles", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("enriched DOI should load");
        assert_eq!(doi, "10.1000/canonical");
    }

    #[test]
    fn content_failure_rolls_back_rows_projections_aliases_and_outbox() {
        let connection = Connection::open_in_memory().expect("content database should open");
        init_content_db(&connection).expect("content schema should initialize");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_content_outbox
                 BEFORE INSERT ON article_change_events
                 BEGIN
                     SELECT RAISE(ABORT, 'injected outbox failure');
                 END;",
            )
            .expect("failure trigger should create");

        let error = write_content_batch(
            &connection,
            &canonical_catalog(),
            &canonical_batch(Some("10.1000/canonical")),
            "revision-fail",
            "2026-07-18T00:00:00Z",
        )
        .expect_err("outbox failure should abort the content transaction");
        assert!(matches!(error, ContentDatabaseError::Sqlite(_)));
        for table in [
            "journals",
            "issues",
            "articles",
            "article_identity_keys",
            "article_listing",
            "article_search",
            "article_change_events",
        ] {
            assert_eq!(table_count(&connection, table), 0, "table {table}");
        }
    }

    #[test]
    fn nonempty_unversioned_content_database_requires_rebuild_without_mutation() {
        let connection = Connection::open_in_memory().expect("legacy database should open");
        connection
            .execute_batch(
                "CREATE TABLE legacy_articles (article_id INTEGER PRIMARY KEY);
                 INSERT INTO legacy_articles (article_id) VALUES (7);",
            )
            .expect("legacy schema should create");

        assert!(matches!(
            init_content_db(&connection),
            Err(ContentDatabaseError::RebuildRequired { found_version: 0 })
        ));
        assert_eq!(table_count(&connection, "legacy_articles"), 1);
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("legacy version should load"),
            0
        );
    }

    fn canonical_catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "issn-1234-5679".to_string(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: Some("Systems".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn canonical_batch(doi: Option<&str>) -> ProviderBatch {
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
                doi: doi.map(str::to_string),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_doi: None,
            }],
            is_complete: true,
            next_checkpoint: None,
        }
    }

    fn table_count(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("table count should load")
    }

    #[test]
    fn catalog_sync_inserts_neutral_journal_and_csv_metadata() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let journal = catalog_journal_record();
        let metadata = catalog_meta_record();

        let change_count = sync_journal_catalog_entry(&connection, &journal, &metadata)
            .expect("missing catalog entry should synchronize");

        let stored_journal: (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ) = connection
            .query_row(
                "SELECT library_id, platform_journal_id, title, issn, available FROM journals WHERE journal_id = 51",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("neutral journal should load");
        assert_eq!(change_count, 2);
        assert_eq!(
            stored_journal,
            (
                "scholarly".to_string(),
                Some("1234-5678".to_string()),
                Some("Catalog Journal".to_string()),
                Some("1234-5678".to_string()),
                None,
            )
        );
        assert!(journal_catalog_entry_is_current(&connection, &metadata)
            .expect("catalog metadata should verify"));
    }

    #[test]
    fn catalog_sync_refreshes_csv_fields_and_preserves_provider_fields() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let provider_journal = JournalRecord {
            journal_id: 51,
            library_id: "provider-library".to_string(),
            platform_journal_id: Some("provider-id".to_string()),
            title: Some("Provider Title".to_string()),
            issn: Some("1111-1111".to_string()),
            eissn: Some("2222-2222".to_string()),
            scimago_rank: Some(4.5),
            cover_url: Some("https://example.test/cover".to_string()),
            available: Some(1),
            toc_data_approved_and_live: Some(1),
            has_articles: Some(1),
        };
        upsert_journal(&connection, &provider_journal).expect("provider journal should insert");
        upsert_meta(
            &connection,
            &MetaRecord {
                journal_id: 51,
                source_csv: "old.csv".to_string(),
                area: Some("old area".to_string()),
                csv_title: Some("Old CSV Title".to_string()),
                csv_issn: Some("0000-0000".to_string()),
                csv_library: Some("old-library".to_string()),
                resolved_source: Some("openalex".to_string()),
                resolved_source_id: Some("S123".to_string()),
                resolved_title: Some("Resolved Title".to_string()),
                resolved_issn: Some("1111-1111".to_string()),
                resolved_eissn: Some("2222-2222".to_string()),
            },
        )
        .expect("old metadata should insert");
        let metadata = catalog_meta_record();

        let change_count =
            sync_journal_catalog_entry(&connection, &catalog_journal_record(), &metadata)
                .expect("stale catalog fields should synchronize");

        let stored_journal: (String, Option<String>, Option<String>, Option<i64>) = connection
            .query_row(
                "SELECT library_id, platform_journal_id, eissn, available FROM journals WHERE journal_id = 51",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("provider journal should load");
        let resolved_fields: [Option<String>; 5] = connection
            .query_row(
                "SELECT resolved_source, resolved_source_id, resolved_title, resolved_issn, resolved_eissn FROM journal_meta WHERE journal_id = 51",
                [],
                |row| Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?]),
            )
            .expect("resolved metadata should load");
        assert_eq!(change_count, 1);
        assert_eq!(
            stored_journal,
            (
                "provider-library".to_string(),
                Some("provider-id".to_string()),
                Some("2222-2222".to_string()),
                Some(1),
            )
        );
        assert_eq!(
            resolved_fields,
            [
                Some("openalex".to_string()),
                Some("S123".to_string()),
                Some("Resolved Title".to_string()),
                Some("1111-1111".to_string()),
                Some("2222-2222".to_string()),
            ]
        );
        assert!(journal_catalog_entry_is_current(&connection, &metadata)
            .expect("refreshed catalog metadata should verify"));
    }

    #[test]
    fn catalog_sync_is_a_noop_when_csv_fields_are_current() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let journal = catalog_journal_record();
        let metadata = catalog_meta_record();
        sync_journal_catalog_entry(&connection, &journal, &metadata)
            .expect("first catalog sync should succeed");

        let change_count = sync_journal_catalog_entry(&connection, &journal, &metadata)
            .expect("current catalog sync should succeed");

        assert_eq!(change_count, 0);
        assert_eq!(table_count(&connection, "journals"), 1);
        assert_eq!(table_count(&connection, "journal_meta"), 1);
    }

    #[test]
    fn open_index_db_sets_busy_timeout() {
        let db_file = NamedTempFile::new().expect("database file should be created");
        let connection = open_index_db(db_file.path()).expect("index db should open");
        let busy_timeout_ms: i64 = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy timeout should be readable");

        assert_eq!(busy_timeout_ms, 30_000);
    }

    #[test]
    fn scholarly_journal_synchronization_date_accepts_trusted_unix_and_iso_checkpoints() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        mark_journal_done(&connection, 1, "1783900800").expect("Unix checkpoint should be written");
        mark_journal_done(&connection, 2, "2026-07-13T12:00:00Z")
            .expect("ISO checkpoint should be written");
        mark_journal_done(&connection, 3, "not-a-timestamp")
            .expect("malformed checkpoint should be written");
        mark_journal_done(&connection, 4, "2026-07-15T00:00:00Z")
            .expect("future checkpoint should be written");

        assert_eq!(
            get_journal_synchronization_date(&connection, 1, "1783987200")
                .expect("Unix checkpoint should query"),
            Some("2026-06-13".to_string())
        );
        assert_eq!(
            get_journal_synchronization_date(&connection, 2, "2026-07-14T00:00:00Z")
                .expect("ISO checkpoint should query"),
            Some("2026-06-13".to_string())
        );
        assert_eq!(
            get_journal_synchronization_date(&connection, 3, "2026-07-14T00:00:00Z")
                .expect("malformed checkpoint should fall back"),
            None
        );
        assert_eq!(
            get_journal_synchronization_date(&connection, 4, "2026-07-14T00:00:00Z")
                .expect("future checkpoint should fall back"),
            None
        );
        assert_eq!(
            get_journal_synchronization_date(&connection, 1, "invalid-current")
                .expect("invalid current timestamp should fall back"),
            None
        );
        assert_eq!(
            get_journal_synchronization_date(&connection, 99, "2026-07-14T00:00:00Z")
                .expect("missing checkpoint should fall back"),
            None
        );
    }

    #[test]
    fn preserves_existing_default_fts_schema() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE article_search USING fts5(
                    article_id UNINDEXED,
                    title,
                    abstract,
                    doi,
                    authors,
                    journal_title
                );",
            )
            .expect("default FTS table should be created");
        init_index_db_with_simple_tokenizer_path(&connection, None)
            .expect("existing default FTS schema should remain compatible");

        let listing_state_sql = object_sql(&connection, "listing_state");
        assert!(listing_state_sql.contains("CHECK (id = 1)"));
        assert!(listing_state_sql.contains("status TEXT NOT NULL"));
        assert!(listing_state_sql.contains("updated_at TEXT"));
        assert_eq!(
            table_columns(&connection, "index_run_lease"),
            ["id", "run_id", "heartbeat_at", "expires_at"]
        );
        let lease_sql = object_sql(&connection, "index_run_lease");
        assert!(lease_sql.contains("CHECK (id = 1)"));
        assert!(lease_sql.contains("REFERENCES index_runs(run_id)"));
        assert_eq!(
            table_columns(&connection, "article_search"),
            [
                "article_id",
                "title",
                "abstract",
                "doi",
                "authors",
                "journal_title"
            ]
        );
        let article_search_sql = object_sql(&connection, "article_search").to_ascii_lowercase();
        assert!(article_search_sql.contains("using fts5"));
        assert!(!article_search_sql.contains("tokenize = 'simple'"));

        let indexes = index_names(&connection);
        for index_name in RUNTIME_INDEXES {
            assert!(indexes.contains(*index_name), "missing index {index_name}");
        }
        assert!(indexes.contains("idx_index_api_call_stats_run"));
    }

    #[test]
    fn initializes_fts_with_simple_tokenizer_when_extension_loads() {
        let Some(simple_tokenizer_path) = workspace_simple_tokenizer_path() else {
            return;
        };
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db_with_simple_tokenizer_path(&connection, Some(&simple_tokenizer_path))
            .expect("schema should initialize with tokenizer extension");

        let article_search_sql = object_sql(&connection, "article_search").to_ascii_lowercase();
        assert!(article_search_sql.contains("tokenize = 'simple'"));
    }

    #[test]
    fn invalid_simple_tokenizer_fails_before_schema_mutation() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        let error = init_index_db_with_simple_tokenizer_path(
            &connection,
            Some(Path::new("missing-simple-tokenizer-extension")),
        )
        .expect_err("missing tokenizer extension should fail schema initialization");

        let object_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .expect("schema object count should query");
        assert!(error
            .to_string()
            .contains("missing-simple-tokenizer-extension"));
        assert_eq!(object_count, 0);
    }

    #[test]
    fn mark_article_listing_ready_upserts_single_ready_row() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        mark_article_listing_ready(&connection, "2026-07-05T12:00:00Z")
            .expect("listing should be marked ready");
        mark_article_listing_ready(&connection, "2026-07-05T12:01:00Z")
            .expect("listing ready row should update");

        let row_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM listing_state", [], |row| row.get(0))
            .expect("listing state count should query");
        let (status, updated_at): (String, String) = connection
            .query_row(
                "SELECT status, updated_at FROM listing_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("listing ready row should query");

        assert_eq!(row_count, 1);
        assert_eq!(status, "ready");
        assert_eq!(updated_at, "2026-07-05T12:01:00Z");
    }

    #[test]
    fn active_run_lease_rejects_contender_without_mutating_state() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        insert_pending_event(
            &connection,
            "pending-run",
            "pending-worker",
            100,
            "add",
            "inpress",
            10,
            None,
            false,
            "2026-07-14T00:00:00Z",
        );

        let outcome = begin_index_run(
            &connection,
            &index_run_start_request("run-active", "2026-07-14T00:01:00Z", 100, false),
        )
        .expect("first run should acquire the lease");
        assert_eq!(outcome.interrupted_run_id, None);
        assert_eq!(outcome.adopted_event_count, 0);

        let error = begin_index_run(
            &connection,
            &index_run_start_request("run-contender", "2026-07-14T00:01:01Z", 101, true),
        )
        .expect_err("fresh lease should reject a contender");
        assert!(matches!(
            error,
            IndexRunLeaseError::ActiveLease {
                ref run_id,
                expires_at: 400
            } if run_id == "run-active"
        ));

        let parent: (String, i64) = connection
            .query_row(
                "SELECT status, total_journals FROM index_runs WHERE run_id = 'run-active'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("active parent should load");
        let parent_count = table_count(&connection, "index_runs");
        let pending_run_id: String = connection
            .query_row("SELECT run_id FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("pending event should remain");
        assert_eq!(parent, ("running".to_string(), 2));
        assert_eq!(parent_count, 1);
        assert_eq!(pending_run_id, "pending-run");
    }

    #[test]
    fn expired_run_is_interrupted_and_pending_events_are_normalized() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        begin_index_run(
            &connection,
            &index_run_start_request("run-old", "2026-07-14T00:00:00Z", 100, false),
        )
        .expect("old run should acquire the lease");
        insert_pending_event(
            &connection,
            "run-old",
            "worker-a",
            101,
            "add",
            "issue",
            11,
            Some(111),
            false,
            "2026-07-14T00:00:01Z",
        );
        insert_pending_event(
            &connection,
            "run-other",
            "worker-a",
            101,
            "remove",
            "issue",
            11,
            Some(111),
            false,
            "2026-07-14T00:00:02Z",
        );
        insert_pending_event(
            &connection,
            "run-other",
            "worker-b",
            202,
            "add",
            "inpress",
            22,
            None,
            true,
            "2026-07-14T00:00:03Z",
        );
        insert_pending_event(
            &connection,
            "run-third",
            "worker-c",
            202,
            "add",
            "inpress",
            22,
            None,
            false,
            "2026-07-14T00:00:04Z",
        );

        let outcome = begin_index_run(
            &connection,
            &index_run_start_request("run-new", "2026-07-14T00:05:00Z", 400, true),
        )
        .expect("expired run should be reclaimed");

        assert_eq!(outcome.interrupted_run_id.as_deref(), Some("run-old"));
        assert_eq!(outcome.adopted_event_count, 4);
        let old_parent: (String, Option<String>) = connection
            .query_row(
                "SELECT status, finished_at FROM index_runs WHERE run_id = 'run-old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("interrupted parent should load");
        assert_eq!(
            old_parent,
            (
                "interrupted".to_string(),
                Some("2026-07-14T00:05:00Z".to_string())
            )
        );
        let event: (String, String, i64, String, String, i64) = connection
            .query_row(
                "SELECT run_id, worker_id, article_id, event_type, created_at, is_backfill
                 FROM index_change_events",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("normalized event should load");
        assert_eq!(
            event,
            (
                "run-new".to_string(),
                "worker-b".to_string(),
                202,
                "add".to_string(),
                "2026-07-14T00:00:03Z".to_string(),
                1,
            )
        );
    }

    #[test]
    fn event_adoption_crosses_the_bounded_batch_boundary() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        connection
            .execute_batch(
                "
                WITH RECURSIVE event_rows(article_id) AS (
                    SELECT 1
                    UNION ALL
                    SELECT article_id + 1 FROM event_rows WHERE article_id < 1001
                )
                INSERT INTO index_change_events (
                    run_id, worker_id, article_id, event_type, membership_type,
                    journal_id, issue_id, is_backfill, created_at
                )
                SELECT
                    'run-batch-old', 'worker-batch', article_id, 'add',
                    'inpress', 33, NULL, 0, '2026-07-14T00:00:00Z'
                FROM event_rows;
                ",
            )
            .expect("batched pending events should insert");

        let outcome = begin_index_run(
            &connection,
            &index_run_start_request("run-batch-new", "2026-07-14T00:01:00Z", 100, true),
        )
        .expect("pending events should be adopted");

        let current_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM index_change_events WHERE run_id = 'run-batch-new'",
                [],
                |row| row.get(0),
            )
            .expect("adopted event count should load");
        assert_eq!(outcome.adopted_event_count, 1001);
        assert_eq!(current_count, 1001);
        assert_eq!(table_count(&connection, "index_change_events"), 1001);
    }

    #[test]
    fn run_lease_heartbeat_fencing_and_release_require_exact_owner() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        begin_index_run(
            &connection,
            &index_run_start_request("run-owner", "2026-07-14T00:00:00Z", 100, false),
        )
        .expect("owner should acquire the lease");

        assert_index_run_lease_owner(&connection, "run-owner", 399)
            .expect("unexpired owner should pass fencing");
        let wrong_owner = assert_index_run_lease_owner(&connection, "run-other", 101)
            .expect_err("different run should fail fencing");
        assert!(matches!(
            wrong_owner,
            IndexRunLeaseError::OwnershipLost { ref run_id } if run_id == "run-other"
        ));
        heartbeat_index_run_lease(&connection, "run-owner", 150)
            .expect("current owner should renew the lease");
        let lease: (i64, i64) = connection
            .query_row(
                "SELECT heartbeat_at, expires_at FROM index_run_lease WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("renewed lease should load");
        assert_eq!(lease, (150, 450));
        assert_index_run_lease_owner(&connection, "run-owner", 449)
            .expect("renewed owner should pass fencing");
        assert!(matches!(
            assert_index_run_lease_owner(&connection, "run-owner", 450),
            Err(IndexRunLeaseError::OwnershipLost { .. })
        ));
        assert!(!release_index_run_lease(&connection, "run-other")
            .expect("wrong owner release should execute"));
        assert!(release_index_run_lease(&connection, "run-owner")
            .expect("current owner release should execute"));
        assert_eq!(table_count(&connection, "index_run_lease"), 0);
    }

    #[test]
    fn post_acquisition_refresh_requires_exact_owner_after_expiry() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        begin_index_run(
            &connection,
            &index_run_start_request("run-owner", "2026-07-14T00:00:00Z", 100, false),
        )
        .expect("owner should acquire the lease");

        refresh_index_run_lease_after_acquisition(&connection, "run-owner", 450)
            .expect("exact owner should refresh after initial expiry");
        let lease: (String, i64, i64) = connection
            .query_row(
                "SELECT run_id, heartbeat_at, expires_at FROM index_run_lease WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("refreshed lease should load");
        assert_eq!(lease, ("run-owner".to_string(), 450, 750));

        let wrong_owner = refresh_index_run_lease_after_acquisition(&connection, "run-other", 500)
            .expect_err("different owner must not refresh the lease");
        assert!(matches!(
            wrong_owner,
            IndexRunLeaseError::OwnershipLost { ref run_id } if run_id == "run-other"
        ));
        assert_index_run_lease_owner(&connection, "run-owner", 749)
            .expect("refreshed owner should remain fenced by the new expiry");
        assert!(matches!(
            assert_index_run_lease_owner(&connection, "run-owner", 750),
            Err(IndexRunLeaseError::OwnershipLost { .. })
        ));
    }

    #[test]
    fn event_adoption_failure_rolls_back_parent_lease_and_source_events() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        insert_pending_event(
            &connection,
            "run-pending",
            "worker-pending",
            303,
            "add",
            "inpress",
            33,
            None,
            false,
            "2026-07-14T00:00:00Z",
        );
        connection
            .execute_batch(
                "
                CREATE TRIGGER fail_event_adoption
                BEFORE DELETE ON index_change_events
                BEGIN
                    SELECT RAISE(ABORT, 'forced adoption failure');
                END;
                ",
            )
            .expect("adoption failpoint should install");

        let error = begin_index_run(
            &connection,
            &index_run_start_request("run-rollback", "2026-07-14T00:01:00Z", 100, true),
        )
        .expect_err("event adoption failure should abort the transaction");

        assert!(matches!(error, IndexRunLeaseError::Sqlite(_)));
        assert_eq!(table_count(&connection, "index_runs"), 0);
        assert_eq!(table_count(&connection, "index_run_lease"), 0);
        let pending_run_id: String = connection
            .query_row("SELECT run_id FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("source event should remain");
        assert_eq!(pending_run_id, "run-pending");
    }

    #[test]
    fn initializes_schema_and_writes_listing_rows() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        upsert_journal(
            &connection,
            &JournalRecord {
                journal_id: 1,
                library_id: "scholarly".into(),
                platform_journal_id: Some("1234-5678".into()),
                title: Some("Test Journal".into()),
                issn: Some("1234-5678".into()),
                eissn: None,
                scimago_rank: None,
                cover_url: None,
                available: Some(1),
                toc_data_approved_and_live: None,
                has_articles: Some(1),
            },
        )
        .expect("journal should insert");
        upsert_meta(
            &connection,
            &MetaRecord {
                journal_id: 1,
                source_csv: "journals.csv".into(),
                area: Some("testing".into()),
                csv_title: Some("Test Journal".into()),
                csv_issn: Some("1234-5678".into()),
                csv_library: Some("scholarly".into()),
                resolved_source: None,
                resolved_source_id: None,
                resolved_title: None,
                resolved_issn: None,
                resolved_eissn: None,
            },
        )
        .expect("meta should insert");
        let article = ArticleRecord {
            article_id: 2,
            journal_id: 1,
            issue_id: None,
            title: Some("Article".into()),
            date: Some("2025-01-01".into()),
            authors: Some("Ada Lovelace".into()),
            start_page: None,
            end_page: None,
            abstract_text: Some("Abstract".into()),
            doi: Some("10.1/a".into()),
            pmid: None,
            permalink: None,
            suppressed: None,
            in_press: Some(1),
            open_access: Some(1),
            platform_id: Some("10.1/a".into()),
            retraction_doi: None,
            within_library_holdings: None,
            content_location: None,
            full_text_file: None,
        };
        upsert_articles(&connection, std::slice::from_ref(&article))
            .expect("article should insert");
        upsert_article_search(&connection, &[article], "Test Journal")
            .expect("search should insert");
        refresh_article_listing_for_articles(&connection, &[2]).expect("listing should refresh");
        let area: String = connection
            .query_row(
                "SELECT area FROM article_listing WHERE article_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("listing should exist");

        assert_eq!(area, "testing");
    }

    #[test]
    fn article_batch_rolls_back_when_fts_write_fails() {
        assert_article_batch_failure_rolls_back(
            "
            DROP TABLE article_search;
            CREATE TABLE article_search (
                rowid INTEGER PRIMARY KEY,
                article_id INTEGER,
                title TEXT CHECK (title <> 'Atomic Article'),
                abstract TEXT,
                doi TEXT,
                authors TEXT,
                journal_title TEXT
            );
            ",
        );
    }

    #[test]
    fn article_batch_rolls_back_when_listing_write_fails() {
        assert_article_batch_failure_rolls_back(
            "
            CREATE TRIGGER fail_listing_insert
            BEFORE INSERT ON article_listing
            BEGIN
                SELECT RAISE(ABORT, 'forced listing failure');
            END;
            ",
        );
    }

    #[test]
    fn article_batch_rolls_back_when_event_write_fails() {
        assert_article_batch_failure_rolls_back(
            "
            CREATE TRIGGER fail_event_insert
            BEFORE INSERT ON index_change_events
            BEGIN
                SELECT RAISE(ABORT, 'forced event failure');
            END;
            ",
        );
    }

    #[test]
    fn article_batch_commits_projections_state_and_normalized_events() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let context =
            ChangeEventContext::new("run-atomic", "worker-0", "2026-07-13T00:00:00Z", false);
        let article = atomic_article_record();

        with_immediate_index_transaction(&connection, |transaction| {
            upsert_journal(transaction, &atomic_journal_record())?;
            upsert_meta(transaction, &atomic_meta_record())?;
            apply_article_changes(
                transaction,
                std::slice::from_ref(&article),
                &[],
                "Atomic Journal",
                Some(&context),
            )?;
            mark_journal_done(transaction, 41, "2026-07-13T00:00:00Z")?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("atomic article batch should commit");

        assert_atomic_counts(&connection, (1, 1, 1, 1, 1, 1));
        let event_type: String = connection
            .query_row("SELECT event_type FROM index_change_events", [], |row| {
                row.get(0)
            })
            .expect("change event should load");
        assert_eq!(event_type, "add");

        with_immediate_index_transaction(&connection, |transaction| {
            apply_article_changes(
                transaction,
                &[],
                &[article.article_id],
                "Atomic Journal",
                Some(&context),
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("inverse article batch should commit");

        assert_atomic_counts(&connection, (1, 0, 0, 0, 0, 1));
    }

    #[test]
    fn persists_index_run_path_and_api_stats() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        let mut stats = IndexRunStats::new(
            "run-1".to_string(),
            "journals.csv".to_string(),
            "2026-07-05T00:00:00Z".to_string(),
        );
        let key = stats.start_path(
            "scholarly",
            "journal",
            Some(1),
            "Test Journal".to_string(),
            "2026-07-05T00:00:01Z".to_string(),
        );
        stats.record_path_counts(
            &key,
            PathCountIncrements {
                works_count: 3,
                issues_count: 1,
                articles_written_count: 2,
                ..PathCountIncrements::default()
            },
        );
        stats.record_source_attempts(
            &[SourceAttempt {
                service: "openalex".to_string(),
                endpoint: "works".to_string(),
                method: "GET".to_string(),
                url: "https://api.openalex.org/works?api_key=SECRET".to_string(),
                status_code: Some(200),
                did_succeed: true,
                did_retry: false,
                error: None,
            }],
            Some(1),
            "Test Journal",
        );
        stats.finish_path(&key, "succeeded", "2026-07-05T00:00:02Z".to_string(), None);
        stats.finish("succeeded", "2026-07-05T00:00:03Z".to_string(), None);

        persist_index_run_stats(&connection, &stats).expect("stats should persist");
        persist_index_run_stats(&connection, &stats).expect("stats should replace prior rows");
        let run_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM index_runs", [], |row| row.get(0))
            .expect("run count should query");
        let path_status: String = connection
            .query_row("SELECT status FROM index_path_stats", [], |row| row.get(0))
            .expect("path stats should query");
        let attempts: i64 = connection
            .query_row("SELECT attempts FROM index_api_call_stats", [], |row| {
                row.get(0)
            })
            .expect("api stats should query");

        assert_eq!(run_count, 1);
        assert_eq!(path_status, "succeeded");
        assert_eq!(attempts, 1);
    }

    fn object_sql(connection: &Connection, name: &str) -> String {
        connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .expect("sqlite object should exist")
    }

    fn table_columns(connection: &Connection, table_name: &str) -> Vec<String> {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table_name})"))
            .expect("table info should prepare");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("columns should query");
        rows.collect::<Result<Vec<_>, _>>()
            .expect("columns should collect")
    }

    fn index_names(connection: &Connection) -> BTreeSet<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
            .expect("index query should prepare");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("indexes should query");
        rows.collect::<Result<BTreeSet<_>, _>>()
            .expect("indexes should collect")
    }

    fn assert_article_batch_failure_rolls_back(failpoint_sql: &str) {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        connection
            .execute_batch(failpoint_sql)
            .expect("failpoint should install");
        let context =
            ChangeEventContext::new("run-failure", "worker-0", "2026-07-13T00:00:00Z", false);
        let article = atomic_article_record();

        with_immediate_index_transaction(&connection, |transaction| {
            upsert_journal(transaction, &atomic_journal_record())?;
            upsert_meta(transaction, &atomic_meta_record())?;
            apply_article_changes(
                transaction,
                &[article],
                &[],
                "Atomic Journal",
                Some(&context),
            )?;
            mark_journal_done(transaction, 41, "2026-07-13T00:00:00Z")?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect_err("forced projection failure should abort the batch");

        assert_atomic_counts(&connection, (0, 0, 0, 0, 0, 0));
    }

    fn assert_atomic_counts(connection: &Connection, expected: (i64, i64, i64, i64, i64, i64)) {
        let counts = (
            table_count(connection, "journals"),
            table_count(connection, "articles"),
            table_count(connection, "article_search"),
            table_count(connection, "article_listing"),
            table_count(connection, "index_change_events"),
            table_count(connection, "journal_state"),
        );
        assert_eq!(counts, expected);
    }

    fn atomic_journal_record() -> JournalRecord {
        JournalRecord {
            journal_id: 41,
            library_id: "scholarly".into(),
            platform_journal_id: Some("atomic".into()),
            title: Some("Atomic Journal".into()),
            issn: Some("1234-5678".into()),
            eissn: None,
            scimago_rank: None,
            cover_url: None,
            available: Some(1),
            toc_data_approved_and_live: None,
            has_articles: Some(1),
        }
    }

    fn catalog_journal_record() -> JournalRecord {
        JournalRecord {
            journal_id: 51,
            library_id: "scholarly".to_string(),
            platform_journal_id: Some("1234-5678".to_string()),
            title: Some("Catalog Journal".to_string()),
            issn: Some("1234-5678".to_string()),
            eissn: None,
            scimago_rank: None,
            cover_url: None,
            available: None,
            toc_data_approved_and_live: None,
            has_articles: None,
        }
    }

    fn catalog_meta_record() -> MetaRecord {
        MetaRecord {
            journal_id: 51,
            source_csv: "journals.csv".to_string(),
            area: Some("catalog area".to_string()),
            csv_title: Some("Catalog Journal".to_string()),
            csv_issn: Some("1234-5678".to_string()),
            csv_library: Some("scholarly".to_string()),
            resolved_source: None,
            resolved_source_id: None,
            resolved_title: None,
            resolved_issn: None,
            resolved_eissn: None,
        }
    }

    fn atomic_meta_record() -> MetaRecord {
        MetaRecord {
            journal_id: 41,
            source_csv: "journals.csv".into(),
            area: Some("testing".into()),
            csv_title: Some("Atomic Journal".into()),
            csv_issn: Some("1234-5678".into()),
            csv_library: Some("scholarly".into()),
            resolved_source: None,
            resolved_source_id: None,
            resolved_title: None,
            resolved_issn: None,
            resolved_eissn: None,
        }
    }

    fn atomic_article_record() -> ArticleRecord {
        ArticleRecord {
            article_id: 4100,
            journal_id: 41,
            issue_id: None,
            title: Some("Atomic Article".into()),
            date: Some("2026-07-13".into()),
            authors: Some("Atomic Author".into()),
            start_page: None,
            end_page: None,
            abstract_text: Some("Atomic Abstract".into()),
            doi: Some("10.4100/atomic".into()),
            pmid: None,
            permalink: None,
            suppressed: Some(0),
            in_press: Some(1),
            open_access: Some(1),
            platform_id: Some("atomic-article".into()),
            retraction_doi: None,
            within_library_holdings: Some(0),
            content_location: None,
            full_text_file: None,
        }
    }

    fn index_run_start_request<'a>(
        run_id: &'a str,
        started_at: &'a str,
        now_epoch_seconds: i64,
        should_adopt_events: bool,
    ) -> IndexRunStartRequest<'a> {
        IndexRunStartRequest {
            run_id,
            csv_file: "journals.csv",
            started_at,
            total_journals: 2,
            now_epoch_seconds,
            should_adopt_events,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_pending_event(
        connection: &Connection,
        run_id: &str,
        worker_id: &str,
        article_id: i64,
        event_type: &str,
        membership_type: &str,
        journal_id: i64,
        issue_id: Option<i64>,
        is_backfill: bool,
        created_at: &str,
    ) {
        connection
            .execute(
                "
                INSERT INTO index_change_events (
                    run_id, worker_id, article_id, event_type, membership_type,
                    journal_id, issue_id, is_backfill, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ",
                rusqlite::params![
                    run_id,
                    worker_id,
                    article_id,
                    event_type,
                    membership_type,
                    journal_id,
                    issue_id,
                    is_backfill as i64,
                    created_at,
                ],
            )
            .expect("pending event should insert");
    }

    fn workspace_simple_tokenizer_path() -> Option<PathBuf> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let project_root = manifest_dir.ancestors().nth(2)?;
        simple_tokenizer_path_from_root(project_root)
    }
}
