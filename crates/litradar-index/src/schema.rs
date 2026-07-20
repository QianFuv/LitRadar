//! SQLite schema and writer helpers for Rust scholarly indexing.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use litradar_domain::{
    ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, ProviderBatch,
};
use litradar_provider::conformance::{
    validate_catalog_entry, validate_provider_batch, ContractViolation,
};

use crate::identity::{
    article_identity_keys, issue_id_from_draft, journal_id_from_catalog_id,
    merge_resolved_article_drafts, resolve_article_identity, ArticleIdentityError,
    ArticleIdentityKey, ArticleMergeError,
};
const INDEX_BUSY_TIMEOUT_SECONDS: u64 = 30;
const JOURNAL_IDENTITY_CONFLICT_MESSAGE: &str =
    "journal identity ownership conflicts with canonical catalog";
const LEGACY_JOURNAL_NOT_EMPTY_MESSAGE: &str =
    "legacy journal entity owns content or durable history";

/// Current provider-neutral content database schema version.
pub const CONTENT_SCHEMA_VERSION: i64 = 5;

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

    CREATE TABLE journal_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
        identity_value TEXT NOT NULL,
        canonical_catalog_id TEXT NOT NULL,
        PRIMARY KEY (identity_kind, identity_value)
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
    CREATE INDEX idx_journal_identity_keys_catalog
        ON journal_identity_keys(canonical_catalog_id);
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct JournalProjectionRefresh {
    refresh_listing_area: bool,
    refresh_search_title: bool,
}

/// Open and validate a provider-neutral content database.
///
/// # Arguments
///
/// * `path` - Content database path derived only from the catalog filename.
///
/// # Returns
///
/// Initialized v5 connection or an explicit rebuild-required failure.
pub fn open_content_db(path: impl AsRef<Path>) -> Result<Connection, ContentDatabaseError> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(INDEX_BUSY_TIMEOUT_SECONDS))?;
    init_content_db(&connection)?;
    Ok(connection)
}

/// Run SQLite maintenance after a canonical content index completes.
///
/// # Arguments
///
/// * `connection` - Open provider-neutral content database.
///
/// # Returns
///
/// Success after SQLite refreshes its query planner metadata.
pub fn optimize_content_db(connection: &Connection) -> Result<(), ContentDatabaseError> {
    connection.execute_batch("PRAGMA optimize;")?;
    Ok(())
}

/// Initialize an empty content database or validate an existing v5 database.
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

/// Reconcile maintained journal identities and existing canonical metadata atomically.
///
/// # Arguments
///
/// * `connection` - Open current-version content database.
/// * `entries` - Fully validated maintained catalog selected for one index run.
///
/// # Returns
///
/// Success after desired identities are owned by their canonical catalog IDs, proven-empty legacy
/// journal shells are removed, and metadata projections for existing canonical journals converge.
pub fn reconcile_catalog_identities(
    connection: &Connection,
    entries: &[JournalCatalogEntry],
) -> Result<(), ContentDatabaseError> {
    let mut desired_owners = BTreeMap::new();
    let mut legacy_aliases = BTreeMap::new();
    for entry in entries {
        validate_catalog_entry(entry)?;
        for (identity_kind, identity_value) in catalog_identity_keys(entry) {
            register_desired_identity_owner(
                &mut desired_owners,
                identity_kind,
                identity_value,
                &entry.catalog_id,
            )?;
        }
        for alias in &entry.catalog_aliases {
            if legacy_aliases
                .insert(alias.clone(), entry.catalog_id.clone())
                .is_some_and(|owner| owner != entry.catalog_id)
            {
                return Err(journal_identity_conflict());
            }
        }
    }

    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    for alias in legacy_aliases.keys() {
        remove_empty_legacy_journal(&transaction, alias)?;
    }
    for ((identity_kind, identity_value), canonical_catalog_id) in desired_owners {
        claim_journal_identity_key(
            &transaction,
            &identity_kind,
            &identity_value,
            &canonical_catalog_id,
        )?;
    }
    for entry in entries {
        refresh_existing_canonical_journal(&transaction, entry)?;
    }
    transaction.commit()?;
    Ok(())
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
    claim_catalog_identity_keys(&transaction, catalog)?;
    let (journal_id, projection_refresh) = upsert_canonical_journal(&transaction, catalog)?;
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
    refresh_journal_projections(&transaction, journal_id, catalog, projection_refresh)?;
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
        "journal_identity_keys",
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
            "journal_identity_keys",
            &["identity_kind", "identity_value", "canonical_catalog_id"],
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
        "idx_journal_identity_keys_catalog",
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

fn catalog_identity_keys(catalog: &JournalCatalogEntry) -> Vec<(&'static str, &str)> {
    let mut keys = Vec::with_capacity(1 + catalog.catalog_aliases.len() + catalog.all_issns.len());
    keys.push(("catalog_id", catalog.catalog_id.as_str()));
    keys.extend(
        catalog
            .catalog_aliases
            .iter()
            .map(|alias| ("catalog_id", alias.as_str())),
    );
    keys.extend(catalog.all_issns.iter().map(|issn| ("issn", issn.as_str())));
    keys
}

fn register_desired_identity_owner(
    owners: &mut BTreeMap<(String, String), String>,
    identity_kind: &str,
    identity_value: &str,
    canonical_catalog_id: &str,
) -> Result<(), ContentDatabaseError> {
    let key = (identity_kind.to_string(), identity_value.to_string());
    if owners
        .get(&key)
        .is_some_and(|owner| owner != canonical_catalog_id)
    {
        return Err(journal_identity_conflict());
    }
    owners
        .entry(key)
        .or_insert_with(|| canonical_catalog_id.to_string());
    Ok(())
}

fn remove_empty_legacy_journal(
    connection: &Connection,
    legacy_catalog_id: &str,
) -> Result<(), ContentDatabaseError> {
    let journal_id = connection
        .query_row(
            "SELECT journal_id FROM journals WHERE catalog_id = ?1",
            [legacy_catalog_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(journal_id) = journal_id else {
        return Ok(());
    };
    let has_durable_state = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM issues WHERE journal_id = ?1
             UNION ALL
             SELECT 1 FROM articles WHERE journal_id = ?1
             UNION ALL
             SELECT 1 FROM article_listing WHERE journal_id = ?1
             UNION ALL
             SELECT 1
             FROM article_search AS article_search
             JOIN articles AS articles
               ON articles.article_id = CAST(article_search.article_id AS INTEGER)
             WHERE articles.journal_id = ?1
             UNION ALL
             SELECT 1 FROM article_change_events WHERE journal_id = ?1
         )",
        [journal_id],
        |row| row.get::<_, bool>(0),
    )?;
    if has_durable_state {
        return Err(legacy_journal_not_empty());
    }
    connection.execute(
        "DELETE FROM journal_identity_keys WHERE canonical_catalog_id = ?1",
        [legacy_catalog_id],
    )?;
    let deleted = connection.execute(
        "DELETE FROM journals WHERE journal_id = ?1 AND catalog_id = ?2",
        params![journal_id, legacy_catalog_id],
    )?;
    if deleted != 1 {
        return Err(journal_identity_conflict());
    }
    Ok(())
}

fn claim_journal_identity_key(
    connection: &Connection,
    identity_kind: &str,
    identity_value: &str,
    canonical_catalog_id: &str,
) -> Result<(), ContentDatabaseError> {
    let existing_owner = connection
        .query_row(
            "SELECT canonical_catalog_id
             FROM journal_identity_keys
             WHERE identity_kind = ?1 AND identity_value = ?2",
            params![identity_kind, identity_value],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    match existing_owner {
        Some(owner) if owner != canonical_catalog_id => Err(journal_identity_conflict()),
        Some(_) => Ok(()),
        None => {
            connection.execute(
                "INSERT INTO journal_identity_keys (
                     identity_kind, identity_value, canonical_catalog_id
                 ) VALUES (?1, ?2, ?3)",
                params![identity_kind, identity_value, canonical_catalog_id],
            )?;
            Ok(())
        }
    }
}

fn ensure_canonical_journal_slot(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
) -> Result<(), ContentDatabaseError> {
    let expected_journal_id = journal_id_from_catalog_id(&catalog.catalog_id);
    let catalog_journal_id = connection
        .query_row(
            "SELECT journal_id FROM journals WHERE catalog_id = ?1",
            [&catalog.catalog_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if catalog_journal_id.is_some_and(|journal_id| journal_id != expected_journal_id) {
        return Err(journal_identity_conflict());
    }
    let slot_owner = connection
        .query_row(
            "SELECT catalog_id FROM journals WHERE journal_id = ?1",
            [expected_journal_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if slot_owner.is_some_and(|owner| owner != catalog.catalog_id) {
        return Err(journal_identity_conflict());
    }
    Ok(())
}

fn claim_catalog_identity_keys(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
) -> Result<(), ContentDatabaseError> {
    ensure_canonical_journal_slot(connection, catalog)?;
    for alias in &catalog.catalog_aliases {
        let has_legacy_journal = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM journals WHERE catalog_id = ?1)",
            [alias],
            |row| row.get::<_, bool>(0),
        )?;
        if has_legacy_journal {
            return Err(journal_identity_conflict());
        }
    }
    for (identity_kind, identity_value) in catalog_identity_keys(catalog) {
        claim_journal_identity_key(
            connection,
            identity_kind,
            identity_value,
            &catalog.catalog_id,
        )?;
    }
    Ok(())
}

fn refresh_existing_canonical_journal(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
) -> Result<(), ContentDatabaseError> {
    ensure_canonical_journal_slot(connection, catalog)?;
    let has_canonical_journal = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM journals WHERE catalog_id = ?1)",
        [&catalog.catalog_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_canonical_journal {
        return Ok(());
    }
    let (journal_id, projection_refresh) = upsert_canonical_journal(connection, catalog)?;
    refresh_journal_projections(connection, journal_id, catalog, projection_refresh)
}

fn journal_identity_conflict() -> ContentDatabaseError {
    ContentDatabaseError::Contract(ContractViolation::new(JOURNAL_IDENTITY_CONFLICT_MESSAGE))
}

fn legacy_journal_not_empty() -> ContentDatabaseError {
    ContentDatabaseError::Contract(ContractViolation::new(LEGACY_JOURNAL_NOT_EMPTY_MESSAGE))
}

fn upsert_canonical_journal(
    connection: &Connection,
    catalog: &JournalCatalogEntry,
) -> Result<(i64, JournalProjectionRefresh), ContentDatabaseError> {
    let journal_id = journal_id_from_catalog_id(&catalog.catalog_id);
    let previous_projection = connection
        .query_row(
            "SELECT title, area FROM journals WHERE journal_id = ?1",
            [journal_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let projection_refresh = previous_projection
        .map(|(title, area)| JournalProjectionRefresh {
            refresh_listing_area: area.as_deref() != catalog.area.as_deref(),
            refresh_search_title: title != catalog.title,
        })
        .unwrap_or_default();
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
    Ok((journal_id, projection_refresh))
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
        merge_resolved_article_drafts(existing, article)?
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

    let identity_keys = incoming_keys
        .into_iter()
        .chain(article_identity_keys(&merged))
        .collect::<BTreeSet<_>>();
    for key in identity_keys {
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
    refresh: JournalProjectionRefresh,
) -> Result<(), ContentDatabaseError> {
    if refresh.refresh_listing_area {
        connection.execute(
            "UPDATE article_listing SET area = ?1 WHERE journal_id = ?2",
            params![catalog.area, journal_id],
        )?;
    }
    if refresh.refresh_search_title {
        connection.execute(
            "UPDATE article_search SET journal_title = ?1 WHERE article_id IN (
                 SELECT article_id FROM articles WHERE journal_id = ?2
             )",
            params![catalog.title, journal_id],
        )?;
    }
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

#[cfg(test)]
mod tests {
    use litradar_domain::{
        ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft,
        JournalRankings, ProviderBatch,
    };
    use rusqlite::Connection;

    use super::{
        init_content_db, reconcile_catalog_identities, write_content_batch, ContentDatabaseError,
        ContentWriteOutcome, CONTENT_SCHEMA_VERSION,
    };

    const TEST_CREATED_AT: &str = "2026-07-18T00:00:00Z";

    fn catalog() -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: "journal-1".to_string(),
            catalog_aliases: Vec::new(),
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5679".to_string()),
            eissn: None,
            all_issns: vec!["1234-5679".to_string()],
            title_aliases: Vec::new(),
            area: Some("Computer Science".to_string()),
            rankings: JournalRankings::default(),
        }
    }

    fn batch() -> ProviderBatch {
        ProviderBatch {
            catalog_id: "journal-1".to_string(),
            journal: JournalDraft {
                catalog_id: "journal-1".to_string(),
                observed_title: Some("Canonical Journal".to_string()),
                observed_issns: vec!["1234-5679".to_string()],
                observed_title_aliases: Vec::new(),
            },
            issues: vec![IssueDraft {
                catalog_id: "journal-1".to_string(),
                publication_year: Some(2026),
                title: None,
                volume: Some("1".to_string()),
                number: Some("2".to_string()),
                date: Some("2026-07".to_string()),
            }],
            articles: vec![ArticleDraft {
                catalog_id: "journal-1".to_string(),
                title: "Shared Article".to_string(),
                publication_year: Some(2026),
                date: Some("2026-07-18".to_string()),
                issue_title: None,
                volume: Some("1".to_string()),
                issue_number: Some("2".to_string()),
                authors: vec![ArticleAuthorDraft {
                    display_name: "Ada Lovelace".to_string(),
                }],
                start_page: Some("1".to_string()),
                end_page: Some("8".to_string()),
                abstract_text: Some("Canonical abstract".to_string()),
                doi: Some("10.1000/shared".to_string()),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_doi: None,
            }],
            is_complete: true,
            next_checkpoint: None,
        }
    }

    fn batch_for_catalog(catalog: &JournalCatalogEntry) -> ProviderBatch {
        let mut provider_batch = batch();
        provider_batch.catalog_id.clone_from(&catalog.catalog_id);
        provider_batch
            .journal
            .catalog_id
            .clone_from(&catalog.catalog_id);
        provider_batch.journal.observed_title = Some(catalog.title.clone());
        provider_batch.journal.observed_issns = catalog.all_issns.clone();
        for issue in &mut provider_batch.issues {
            issue.catalog_id.clone_from(&catalog.catalog_id);
        }
        for article in &mut provider_batch.articles {
            article.catalog_id.clone_from(&catalog.catalog_id);
        }
        provider_batch
    }

    fn merged_catalog(
        catalog_id: &str,
        catalog_alias: &str,
        title: &str,
        print_issn: &str,
        electronic_issn: &str,
    ) -> JournalCatalogEntry {
        JournalCatalogEntry {
            catalog_id: catalog_id.to_string(),
            catalog_aliases: vec![catalog_alias.to_string()],
            title: title.to_string(),
            issn: Some(print_issn.to_string()),
            eissn: Some(electronic_issn.to_string()),
            all_issns: vec![electronic_issn.to_string(), print_issn.to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        }
    }

    fn identity_owners(connection: &Connection) -> Vec<(String, String, String)> {
        connection
            .prepare(
                "SELECT identity_kind, identity_value, canonical_catalog_id
                 FROM journal_identity_keys
                 ORDER BY identity_kind, identity_value",
            )
            .expect("journal identity query should prepare")
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("journal identities should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("journal identities should collect")
    }

    fn batch_with_article_count(article_count: usize) -> ProviderBatch {
        let mut provider_batch = batch();
        let template = provider_batch.articles[0].clone();
        provider_batch.articles = (0..article_count)
            .map(|index| {
                let mut article = template.clone();
                article.title = format!("Projection Article {index}");
                article.start_page = Some((index + 1).to_string());
                article.end_page = Some((index + 2).to_string());
                article.doi = Some(format!("10.1000/projection-{index}"));
                article
            })
            .collect();
        provider_batch
    }

    fn empty_batch(catalog: &JournalCatalogEntry) -> ProviderBatch {
        let mut provider_batch = batch_for_catalog(catalog);
        provider_batch.issues.clear();
        provider_batch.articles.clear();
        provider_batch
    }

    fn write_test_batch(
        connection: &Connection,
        catalog: &JournalCatalogEntry,
        provider_batch: &ProviderBatch,
        revision: &str,
    ) -> ContentWriteOutcome {
        write_content_batch(
            connection,
            catalog,
            provider_batch,
            revision,
            TEST_CREATED_AT,
        )
        .expect("test batch should write")
    }

    fn write_empty_catalog_update(
        connection: &Connection,
        catalog: &JournalCatalogEntry,
        revision: &str,
    ) -> u64 {
        let changes_before = connection.total_changes();
        write_test_batch(connection, catalog, &empty_batch(catalog), revision);
        connection.total_changes() - changes_before
    }

    fn assert_projection_metadata(
        connection: &Connection,
        catalog: &JournalCatalogEntry,
        expected_count: i64,
    ) {
        let counts = connection
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM article_listing),
                     (SELECT COUNT(*) FROM article_listing WHERE area IS ?1),
                     (SELECT COUNT(*) FROM article_search),
                     (SELECT COUNT(*) FROM article_search WHERE journal_title = ?2)",
                rusqlite::params![catalog.area, catalog.title],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .expect("projection metadata should read");
        assert_eq!(
            counts,
            (
                expected_count,
                expected_count,
                expected_count,
                expected_count
            )
        );
    }

    #[test]
    fn content_schema_has_only_the_provider_neutral_allowlist() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("schema version should read");
        assert_eq!(version, CONTENT_SCHEMA_VERSION);
        let identity_columns = connection
            .prepare("PRAGMA table_info(journal_identity_keys)")
            .expect("journal identity columns should prepare")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("journal identity columns should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("journal identity columns should collect");
        assert_eq!(
            identity_columns,
            ["identity_kind", "identity_value", "canonical_catalog_id"]
        );
        let identity_index_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                 WHERE type = 'index' AND name = 'idx_journal_identity_keys_catalog'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("journal identity index should read");
        assert_eq!(identity_index_count, 1);
        let identity_foreign_key_count = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list('journal_identity_keys')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("journal identity foreign keys should read");
        assert_eq!(identity_foreign_key_count, 0);
        let schema = connection
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_schema WHERE sql IS NOT NULL",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema SQL should read")
            .to_ascii_lowercase();
        assert!(schema.contains("identity_kind in ('catalog_id', 'issn')"));
        for forbidden in [
            "provider_name",
            "library_id",
            "platform_id",
            "permalink",
            "content_location",
            "full_text_file",
            "url",
            "checkpoint",
            "lease",
            "statistics",
        ] {
            assert!(
                !schema.contains(forbidden),
                "forbidden schema token {forbidden}"
            );
        }
    }

    #[test]
    fn catalog_reconciliation_converges_environment_metadata_and_projections() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let mut original = merged_catalog(
            "issn-1472-3409",
            "issn-0308-518x",
            "Environment and Planning A: Economy and Space",
            "0308-518X",
            "1472-3409",
        );
        original.catalog_aliases.clear();
        original.title = "Environment and Planning A".to_string();
        original.title_aliases.clear();
        original.issn = None;
        original.all_issns = vec!["1472-3409".to_string()];
        original.area = Some("Legacy Area".to_string());
        write_test_batch(
            &connection,
            &original,
            &batch_for_catalog(&original),
            "english:environment:seed",
        );

        let mut merged = merged_catalog(
            "issn-1472-3409",
            "issn-0308-518x",
            "Environment and Planning A: Economy and Space",
            "0308-518X",
            "1472-3409",
        );
        merged.title_aliases = vec!["Environment and Planning A".to_string()];
        merged.area = Some("Regional, Environmental & Resource Studies".to_string());
        reconcile_catalog_identities(&connection, std::slice::from_ref(&merged))
            .expect("merged identity should reconcile");

        assert_eq!(
            identity_owners(&connection),
            vec![
                (
                    "catalog_id".to_string(),
                    "issn-0308-518x".to_string(),
                    "issn-1472-3409".to_string(),
                ),
                (
                    "catalog_id".to_string(),
                    "issn-1472-3409".to_string(),
                    "issn-1472-3409".to_string(),
                ),
                (
                    "issn".to_string(),
                    "0308-518X".to_string(),
                    "issn-1472-3409".to_string(),
                ),
                (
                    "issn".to_string(),
                    "1472-3409".to_string(),
                    "issn-1472-3409".to_string(),
                ),
            ]
        );
        let metadata = connection
            .query_row(
                "SELECT title, title_aliases_json, issns_json, issn, eissn, area
                 FROM journals WHERE catalog_id = ?1",
                [&merged.catalog_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .expect("reconciled metadata should read");
        assert_eq!(
            metadata,
            (
                merged.title.clone(),
                serde_json::to_string(&merged.title_aliases).expect("aliases should serialize"),
                serde_json::to_string(&merged.all_issns).expect("ISSNs should serialize"),
                "0308-518X".to_string(),
                "1472-3409".to_string(),
                "Regional, Environmental & Resource Studies".to_string(),
            )
        );
        assert_projection_metadata(&connection, &merged, 1);
    }

    #[test]
    fn empty_catalog_reconciliation_claims_all_merged_identity_keys_without_journal_shells() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        connection
            .execute(
                "INSERT INTO journal_identity_keys (
                     identity_kind, identity_value, canonical_catalog_id
                 ) VALUES ('catalog_id', 'historical-journal', 'historical-journal')",
                [],
            )
            .expect("unrelated historical key should seed");
        let series_b = merged_catalog(
            "issn-1467-9868",
            "issn-1369-7412",
            "Journal of the Royal Statistical Society Series B: Statistical Methodology",
            "1369-7412",
            "1467-9868",
        );
        let transportation = merged_catalog(
            "issn-1879-2367",
            "issn-0191-2615",
            "Transportation Research Part B: Methodological",
            "0191-2615",
            "1879-2367",
        );

        reconcile_catalog_identities(&connection, &[series_b.clone(), transportation.clone()])
            .expect("empty catalog identities should reconcile");

        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM journals", [], |row| row
                    .get::<_, i64>(0))
                .expect("journal count should read"),
            0
        );
        let owners = identity_owners(&connection);
        assert_eq!(owners.len(), 9);
        assert!(owners.contains(&(
            "catalog_id".to_string(),
            "historical-journal".to_string(),
            "historical-journal".to_string(),
        )));
        for entry in [&series_b, &transportation] {
            for identity_value in std::iter::once(&entry.catalog_id).chain(&entry.catalog_aliases) {
                assert!(owners.contains(&(
                    "catalog_id".to_string(),
                    identity_value.clone(),
                    entry.catalog_id.clone(),
                )));
            }
            for identity_value in &entry.all_issns {
                assert!(owners.contains(&(
                    "issn".to_string(),
                    identity_value.clone(),
                    entry.catalog_id.clone(),
                )));
            }
        }
    }

    #[test]
    fn catalog_reconciliation_removes_only_a_proven_empty_legacy_shell() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let legacy = JournalCatalogEntry {
            catalog_id: "issn-0308-518x".to_string(),
            catalog_aliases: Vec::new(),
            title: "Environment and Planning A".to_string(),
            issn: Some("0308-518X".to_string()),
            eissn: None,
            all_issns: vec!["0308-518X".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        };
        write_test_batch(
            &connection,
            &legacy,
            &empty_batch(&legacy),
            "english:legacy:empty",
        );
        let canonical = merged_catalog(
            "issn-1472-3409",
            "issn-0308-518x",
            "Environment and Planning A: Economy and Space",
            "0308-518X",
            "1472-3409",
        );

        reconcile_catalog_identities(&connection, std::slice::from_ref(&canonical))
            .expect("empty legacy shell should reconcile");

        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM journals", [], |row| row
                    .get::<_, i64>(0))
                .expect("journal count should read"),
            0
        );
        assert_eq!(identity_owners(&connection).len(), 4);
        assert!(identity_owners(&connection)
            .iter()
            .all(|(_, _, owner)| owner == &canonical.catalog_id));
    }

    #[test]
    fn nonempty_legacy_journal_blocks_reconciliation_atomically() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let legacy = JournalCatalogEntry {
            catalog_id: "issn-0308-518x".to_string(),
            catalog_aliases: Vec::new(),
            title: "Environment and Planning A".to_string(),
            issn: Some("0308-518X".to_string()),
            eissn: None,
            all_issns: vec!["0308-518X".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        };
        write_test_batch(
            &connection,
            &legacy,
            &batch_for_catalog(&legacy),
            "english:legacy:content",
        );
        let owners_before = identity_owners(&connection);
        let canonical = merged_catalog(
            "issn-1472-3409",
            "issn-0308-518x",
            "Environment and Planning A: Economy and Space",
            "0308-518X",
            "1472-3409",
        );

        let error = reconcile_catalog_identities(&connection, &[canonical])
            .expect_err("nonempty legacy journal must fail closed");

        assert_eq!(
            error.to_string(),
            "legacy journal entity owns content or durable history"
        );
        assert_eq!(identity_owners(&connection), owners_before);
        for (table, expected) in [
            ("journals", 1),
            ("issues", 1),
            ("articles", 1),
            ("article_listing", 1),
            ("article_search", 1),
            ("article_change_events", 1),
        ] {
            let count = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("table count should read");
            assert_eq!(count, expected, "failed reconciliation changed {table}");
        }
    }

    #[test]
    fn journal_identity_conflict_rolls_back_legacy_shell_cleanup() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let legacy = JournalCatalogEntry {
            catalog_id: "issn-0308-518x".to_string(),
            catalog_aliases: Vec::new(),
            title: "Environment and Planning A".to_string(),
            issn: Some("0308-518X".to_string()),
            eissn: None,
            all_issns: vec!["0308-518X".to_string()],
            title_aliases: Vec::new(),
            area: None,
            rankings: JournalRankings::default(),
        };
        write_test_batch(
            &connection,
            &legacy,
            &empty_batch(&legacy),
            "english:legacy:empty",
        );
        connection
            .execute(
                "INSERT INTO journal_identity_keys (
                     identity_kind, identity_value, canonical_catalog_id
                 ) VALUES ('issn', '1472-3409', 'unrelated-journal')",
                [],
            )
            .expect("conflicting owner should seed");
        let owners_before = identity_owners(&connection);
        let canonical = merged_catalog(
            "issn-1472-3409",
            "issn-0308-518x",
            "Environment and Planning A: Economy and Space",
            "0308-518X",
            "1472-3409",
        );

        let error = reconcile_catalog_identities(&connection, &[canonical])
            .expect_err("identity conflict must fail closed");

        assert_eq!(
            error.to_string(),
            "journal identity ownership conflicts with canonical catalog"
        );
        assert_eq!(identity_owners(&connection), owners_before);
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM journals WHERE catalog_id = 'issn-0308-518x'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("legacy journal count should read"),
            1
        );
    }

    #[test]
    fn write_time_journal_identity_recheck_allows_only_the_same_owner() {
        let same_owner = Connection::open_in_memory().expect("database should open");
        init_content_db(&same_owner).expect("content schema should initialize");
        same_owner
            .execute(
                "INSERT INTO journal_identity_keys (
                     identity_kind, identity_value, canonical_catalog_id
                 ) VALUES ('catalog_id', 'journal-legacy', 'journal-1')",
                [],
            )
            .expect("same owner alias should seed");
        let mut aliased = catalog();
        aliased.catalog_aliases = vec!["journal-legacy".to_string()];
        write_content_batch(
            &same_owner,
            &aliased,
            &batch_for_catalog(&aliased),
            "catalog:journal-1:same-owner",
            TEST_CREATED_AT,
        )
        .expect("same identity owner should write");

        let other_owner = Connection::open_in_memory().expect("database should open");
        init_content_db(&other_owner).expect("content schema should initialize");
        write_test_batch(&other_owner, &catalog(), &batch(), "catalog:journal-1:seed");
        other_owner
            .execute(
                "INSERT INTO journal_identity_keys (
                     identity_kind, identity_value, canonical_catalog_id
                 ) VALUES ('catalog_id', 'journal-legacy', 'other-journal')",
                [],
            )
            .expect("other owner alias should seed");
        let owners_before = identity_owners(&other_owner);
        let events_before = other_owner
            .query_row("SELECT COUNT(*) FROM article_change_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("event count should read");

        let error = write_content_batch(
            &other_owner,
            &aliased,
            &batch_for_catalog(&aliased),
            "catalog:journal-1:other-owner",
            TEST_CREATED_AT,
        )
        .expect_err("other identity owner must fail closed");

        assert_eq!(
            error.to_string(),
            "journal identity ownership conflicts with canonical catalog"
        );
        assert_eq!(identity_owners(&other_owner), owners_before);
        assert_eq!(
            other_owner
                .query_row("SELECT COUNT(*) FROM article_change_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("event count should read"),
            events_before
        );
    }

    #[test]
    fn cross_catalog_doi_and_pmid_conflicts_roll_back_journal_identity_claims() {
        for identity_kind in ["doi", "pmid"] {
            let connection = Connection::open_in_memory().expect("database should open");
            init_content_db(&connection).expect("content schema should initialize");
            let first_catalog = catalog();
            let mut first_batch = batch_for_catalog(&first_catalog);
            if identity_kind == "pmid" {
                first_batch.articles[0].doi = None;
                first_batch.articles[0].pmid = Some("123456".to_string());
            }
            write_test_batch(
                &connection,
                &first_catalog,
                &first_batch,
                &format!("catalog:journal-1:{identity_kind}:seed"),
            );
            let owners_before = identity_owners(&connection);
            let mut second_catalog = catalog();
            second_catalog.catalog_id = "journal-2".to_string();
            second_catalog.issn = Some("2049-3630".to_string());
            second_catalog.all_issns = vec!["2049-3630".to_string()];
            let mut conflicting_batch = batch_for_catalog(&second_catalog);
            if identity_kind == "pmid" {
                conflicting_batch.articles[0].doi = None;
                conflicting_batch.articles[0].pmid = Some("123456".to_string());
            }

            let error = write_content_batch(
                &connection,
                &second_catalog,
                &conflicting_batch,
                &format!("catalog:journal-2:{identity_kind}:conflict"),
                TEST_CREATED_AT,
            )
            .expect_err("cross-catalog identifier must remain fatal");

            assert!(matches!(
                error,
                ContentDatabaseError::Merge(crate::identity::ArticleMergeError::CatalogMismatch)
            ));
            assert_eq!(identity_owners(&connection), owners_before);
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM journals WHERE catalog_id = 'journal-2'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("second journal count should read"),
                0
            );
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM articles", [], |row| row
                        .get::<_, i64>(0))
                    .expect("article count should read"),
                1
            );
        }
    }

    #[test]
    fn canonical_replay_keeps_ids_rows_and_outbox_stable() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let first = write_content_batch(
            &connection,
            &catalog(),
            &batch(),
            "catalog:journal-1:page-1",
            "2026-07-18T00:00:00Z",
        )
        .expect("first canonical batch should write");
        let article_id = connection
            .query_row("SELECT article_id FROM articles", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("article id should exist");
        let second = write_content_batch(
            &connection,
            &catalog(),
            &batch(),
            "catalog:journal-1:page-1",
            "2026-07-18T00:00:00Z",
        )
        .expect("replayed canonical batch should write");
        let replayed_id = connection
            .query_row("SELECT article_id FROM articles", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("article id should remain");
        assert_eq!(article_id, replayed_id);
        assert_eq!(first.articles_changed, 1);
        assert_eq!(second.articles_changed, 0);
        for (table, expected) in [
            ("journals", 1),
            ("journal_identity_keys", 2),
            ("issues", 1),
            ("articles", 1),
            ("article_identity_keys", 2),
            ("article_listing", 1),
            ("article_search", 1),
            ("article_change_events", 1),
        ] {
            let count = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("row count should read");
            assert_eq!(count, expected, "unexpected replay cardinality for {table}");
        }
    }

    #[test]
    fn unchanged_journal_metadata_does_not_rewrite_existing_projections() {
        let changes = [1_usize, 64].map(|article_count| {
            let connection = Connection::open_in_memory().expect("database should open");
            init_content_db(&connection).expect("content schema should initialize");
            let test_catalog = catalog();
            write_test_batch(
                &connection,
                &test_catalog,
                &batch_with_article_count(article_count),
                "catalog:journal-1:seed",
            );
            let replay_changes = write_empty_catalog_update(
                &connection,
                &test_catalog,
                "catalog:journal-1:empty-replay",
            );
            assert_projection_metadata(&connection, &test_catalog, article_count as i64);
            replay_changes
        });

        assert_eq!(
            changes,
            [1, 1],
            "an identical empty replay must change only the maintained journal row"
        );
    }

    #[test]
    fn journal_metadata_changes_refresh_only_affected_projections() {
        const ARTICLE_COUNT: usize = 4;
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let original_catalog = catalog();
        write_test_batch(
            &connection,
            &original_catalog,
            &batch_with_article_count(ARTICLE_COUNT),
            "catalog:journal-1:seed",
        );

        let mut area_only_catalog = original_catalog.clone();
        area_only_catalog.area = Some("Data Systems".to_string());
        assert_eq!(
            write_empty_catalog_update(
                &connection,
                &area_only_catalog,
                "catalog:journal-1:area-only",
            ),
            1 + ARTICLE_COUNT as u64,
            "area-only metadata must not rewrite search projections"
        );
        assert_projection_metadata(&connection, &area_only_catalog, ARTICLE_COUNT as i64);

        let mut title_only_catalog = area_only_catalog.clone();
        title_only_catalog.title = "Renamed Journal".to_string();
        let title_only_changes = write_empty_catalog_update(
            &connection,
            &title_only_catalog,
            "catalog:journal-1:title-only",
        );
        assert!(title_only_changes > 1);
        assert_projection_metadata(&connection, &title_only_catalog, ARTICLE_COUNT as i64);

        let mut combined_catalog = title_only_catalog.clone();
        combined_catalog.title = "Combined Journal".to_string();
        combined_catalog.area = Some("Combined Area".to_string());
        let combined_changes = write_empty_catalog_update(
            &connection,
            &combined_catalog,
            "catalog:journal-1:combined",
        );
        assert_eq!(
            combined_changes,
            title_only_changes + ARTICLE_COUNT as u64,
            "combined metadata must add exactly one listing refresh"
        );
        assert_projection_metadata(&connection, &combined_catalog, ARTICLE_COUNT as i64);

        assert_eq!(
            write_empty_catalog_update(
                &connection,
                &combined_catalog,
                "catalog:journal-1:combined-replay",
            ),
            1
        );

        let mut changed_article_batch = batch_with_article_count(ARTICLE_COUNT);
        changed_article_batch.journal.observed_title = Some(combined_catalog.title.clone());
        changed_article_batch.articles[0].abstract_text =
            Some("Updated canonical abstract with additional detail".to_string());
        let changed = write_test_batch(
            &connection,
            &combined_catalog,
            &changed_article_batch,
            "catalog:journal-1:article-change",
        );
        let changed_search_rows = connection
            .query_row(
                "SELECT COUNT(*) FROM article_search
                 WHERE abstract_text = 'Updated canonical abstract with additional detail'
                   AND journal_title = ?1",
                [combined_catalog.title.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .expect("changed search projection should read");
        assert_eq!(changed.articles_changed, 1);
        assert_eq!(changed.change_events_emitted, 1);
        assert_eq!(changed_search_rows, 1);
        assert_projection_metadata(&connection, &combined_catalog, ARTICLE_COUNT as i64);

        let new_connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&new_connection).expect("content schema should initialize");
        assert_eq!(
            write_empty_catalog_update(&new_connection, &catalog(), "catalog:journal-1:new-empty",),
            3
        );
        assert_projection_metadata(&new_connection, &catalog(), 0);
    }

    #[test]
    fn canonical_batch_preserves_alternate_doi_aliases() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let mut alternate_doi_batch = batch();
        let mut alternate_doi_article = alternate_doi_batch.articles[0].clone();
        alternate_doi_article.doi = Some("10.1000/alternate".to_string());
        alternate_doi_batch.articles.push(alternate_doi_article);

        let outcome = write_content_batch(
            &connection,
            &catalog(),
            &alternate_doi_batch,
            "catalog:journal-1:page-alternate-doi",
            "2026-07-18T00:00:00Z",
        )
        .expect("alternate DOI aliases should converge");

        assert_eq!(outcome.articles_seen, 2);
        let article_count = connection
            .query_row("SELECT COUNT(*) FROM articles", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("article count should read");
        let doi_alias_count = connection
            .query_row(
                "SELECT COUNT(*) FROM article_identity_keys WHERE identity_kind = 'doi'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("DOI alias count should read");
        let doi_owner_count = connection
            .query_row(
                "SELECT COUNT(DISTINCT article_id) FROM article_identity_keys
                 WHERE identity_kind = 'doi'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("DOI alias owner count should read");
        let canonical_doi = connection
            .query_row("SELECT doi FROM articles", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("canonical DOI should read");

        assert_eq!(article_count, 1);
        assert_eq!(doi_alias_count, 2);
        assert_eq!(doi_owner_count, 1);
        assert_eq!(canonical_doi, "10.1000/alternate");

        alternate_doi_batch.articles.reverse();
        let replay = write_content_batch(
            &connection,
            &catalog(),
            &alternate_doi_batch,
            "catalog:journal-1:page-alternate-doi",
            "2026-07-18T00:00:00Z",
        )
        .expect("reverse-order alternate DOI replay should converge");
        assert_eq!(replay.articles_changed, 0);
        assert_eq!(replay.identity_aliases_added, 0);
        assert_eq!(replay.change_events_emitted, 0);
        for (table, expected) in [
            ("journals", 1),
            ("issues", 1),
            ("articles", 1),
            ("article_identity_keys", 3),
            ("article_listing", 1),
            ("article_search", 1),
            ("article_change_events", 1),
        ] {
            let count = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("replay table count should read");
            assert_eq!(count, expected, "unexpected alternate DOI {table} count");
        }
        let listing_doi = connection
            .query_row("SELECT doi FROM article_listing", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("listing DOI should read");
        let search_doi = connection
            .query_row("SELECT doi FROM article_search", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("search DOI should read");
        assert_eq!(listing_doi, canonical_doi);
        assert_eq!(search_doi, canonical_doi);
    }

    #[test]
    fn alternate_doi_merge_preserves_multiple_identity_conflict_guard() {
        let connection = Connection::open_in_memory().expect("database should open");
        init_content_db(&connection).expect("content schema should initialize");
        let mut initial_batch = batch();
        let mut other_article = initial_batch.articles[0].clone();
        other_article.title = "Other Article".to_string();
        other_article.start_page = Some("9".to_string());
        other_article.doi = Some("10.1000/other".to_string());
        initial_batch.articles.push(other_article.clone());
        write_content_batch(
            &connection,
            &catalog(),
            &initial_batch,
            "catalog:journal-1:page-initial",
            "2026-07-18T00:00:00Z",
        )
        .expect("separate canonical articles should write");

        let before = [
            "articles",
            "article_identity_keys",
            "article_listing",
            "article_search",
            "article_change_events",
        ]
        .into_iter()
        .map(|table| {
            let count = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("pre-conflict table count should read");
            (table, count)
        })
        .collect::<Vec<_>>();
        let mut bridge_article = other_article;
        bridge_article.doi = Some("10.1000/shared".to_string());
        let mut bridge_batch = batch();
        bridge_batch.articles = vec![bridge_article];

        let error = write_content_batch(
            &connection,
            &catalog(),
            &bridge_batch,
            "catalog:journal-1:page-conflict",
            "2026-07-18T00:00:00Z",
        )
        .expect_err("aliases owned by two articles should remain invalid");
        assert!(matches!(
            error,
            ContentDatabaseError::Identity(
                crate::identity::ArticleIdentityError::ConflictingAliases { .. }
            )
        ));

        for (table, expected) in before {
            let count = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("post-conflict table count should read");
            assert_eq!(count, expected, "conflict changed {table}");
        }
    }
}
