//! Durable CFP snapshots, alias ownership and revision-fenced refresh publication.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;

use litradar_domain::cfp::{
    is_cfp_source_url, parse_cfp_source, CfpEmptyJournal, CfpNotice, CfpSeed, CfpSource,
    CFP_PARSER_VERSION,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::open_sqlite_connection;

pub(crate) const SCHEMA_SQL: &str = r#"
CREATE TABLE cfp_journals (
    journal_key TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    checked_on TEXT NOT NULL,
    source_url TEXT,
    source_statement TEXT
);
CREATE TABLE cfp_journal_aliases (
    catalog_id TEXT PRIMARY KEY,
    journal_key TEXT NOT NULL REFERENCES cfp_journals(journal_key) ON DELETE CASCADE
);
CREATE INDEX idx_cfp_alias_owner ON cfp_journal_aliases(journal_key);
CREATE TABLE cfp_sources (
    source_key TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    config_version INTEGER NOT NULL DEFAULT 0,
    capture TEXT NOT NULL,
    capture_format TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT,
    last_attempt INTEGER,
    last_success INTEGER,
    last_error TEXT,
    status TEXT NOT NULL,
    generation INTEGER NOT NULL DEFAULT 0,
    lease_expires_at INTEGER,
    revision INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE cfp_source_journals (
    source_key TEXT NOT NULL REFERENCES cfp_sources(source_key) ON DELETE CASCADE,
    journal_key TEXT NOT NULL REFERENCES cfp_journals(journal_key) ON DELETE CASCADE,
    PRIMARY KEY(source_key, journal_key)
);
CREATE TABLE cfp_notices (
    journal_key TEXT NOT NULL,
    notice_key TEXT NOT NULL,
    source_key TEXT NOT NULL,
    source_json TEXT NOT NULL,
    normalized_json TEXT NOT NULL,
    display_order INTEGER NOT NULL,
    PRIMARY KEY(journal_key, notice_key),
    FOREIGN KEY(source_key, journal_key) REFERENCES cfp_source_journals(source_key, journal_key)
);
CREATE INDEX idx_cfp_notice_source ON cfp_notices(source_key, journal_key, display_order);
CREATE TABLE cfp_seed_imports (
    seed_id TEXT PRIMARY KEY,
    format_version INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    journal_count INTEGER NOT NULL,
    notice_count INTEGER NOT NULL
);
"#;

/// Invalid snapshots, stale refreshes and persistence failures.
#[derive(Debug)]
pub enum CfpRepositoryError {
    /// Database access failed.
    Sqlite(rusqlite::Error),
    /// An import payload could not be decoded.
    Json(serde_json::Error),
    /// Validation failed before publication.
    Invalid(String),
    /// A newer generation or an expired lease superseded this result.
    StaleRefresh,
}

impl fmt::Display for CfpRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "CFP database error: {error}"),
            Self::Json(error) => write!(formatter, "CFP payload error: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
            Self::StaleRefresh => formatter.write_str("CFP refresh was superseded or expired"),
        }
    }
}

impl Error for CfpRepositoryError {}
impl From<rusqlite::Error> for CfpRepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl From<serde_json::Error> for CfpRepositoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Counts and disposition of an atomic offline import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfpImportResult {
    /// Whether this invocation wrote the seed.
    pub did_import: bool,
    /// Distinct journal identities in the seed.
    pub journals: usize,
    /// Distinct journal-scoped announcements in the seed.
    pub notices: usize,
    /// Digest of the exact imported bytes.
    pub content_hash: String,
}

/// Persisted source freshness, independent of current notice availability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfpSourceStatus {
    /// Stable registered acquisition unit.
    pub source_key: String,
    /// Snapshot, success, failed, refreshing or unsupported.
    pub status: String,
    /// Last attempt Unix timestamp, when an acquisition has run.
    pub last_attempt: Option<i64>,
    /// Last successfully published acquisition Unix timestamp.
    pub last_success: Option<i64>,
    /// Sanitized failure reason from the latest attempt.
    pub last_error: Option<String>,
    /// Monotonically increasing published content revision.
    pub revision: i64,
    /// End of the active refresh lease, used to detect an interrupted attempt on read.
    pub lease_expires_at: Option<i64>,
}

/// Complete persisted journal snapshot for backend query evaluation.
#[derive(Debug, Clone)]
pub struct CfpJournalSnapshot {
    /// Canonical seed-owned journal identity.
    pub journal_key: String,
    /// All recorded aliases with exclusive journal ownership.
    pub catalog_ids: Vec<String>,
    /// Original journal title.
    pub journal_title: String,
    /// Last verified original-source date.
    pub checked_on: String,
    /// Source page for an explicit empty statement.
    pub source_url: Option<String>,
    /// Literal verified-empty statement, if present.
    pub source_statement: Option<String>,
    /// Original normalized notices in stable source order.
    pub notices: Vec<CfpNotice>,
    /// Acquisition units and freshness for this journal.
    pub sources: Vec<CfpSourceStatus>,
}

/// A generation token required to publish or fail a refresh attempt.
#[derive(Debug, Clone)]
pub struct CfpRefreshLease {
    /// Stable acquisition unit.
    pub source_key: String,
    /// Current generation; older writers cannot publish.
    pub generation: i64,
    /// Latest permitted publication Unix timestamp.
    pub expires_at: i64,
}

/// A complete verified acquisition result, constructed outside a write transaction.
#[derive(Debug, Clone)]
pub struct CfpPublication {
    /// Original fetched bytes decoded as text (or extracted PDF text).
    pub capture: String,
    /// Original capture representation, such as html or pdf_text.
    pub capture_format: String,
    /// Discovery URL verified by the transport.
    pub source_url: String,
    /// Version of the registered acquisition configuration.
    pub config_version: u32,
    /// Literal source fields, retaining each journal membership.
    pub sources: Vec<CfpSource>,
    /// Explicit scoped statements for journals verified empty.
    pub empty_journals: Vec<CfpEmptyJournal>,
}

struct PreparedJournal {
    title: String,
    checked_on: String,
    aliases: BTreeSet<String>,
    notices: Vec<(CfpSource, CfpNotice)>,
    empty: Option<CfpEmptyJournal>,
}

/// Fully validated immutable seed that may be reused across backend preparations.
pub struct PreparedCfpSeed {
    journals: BTreeMap<String, PreparedJournal>,
    notices: usize,
    digest: String,
}

impl PreparedCfpSeed {
    /// Exact-byte digest used for an immutable operator import identity.
    pub fn content_hash(&self) -> &str {
        &self.digest
    }
}

/// Decode and normalize an offline seed before opening a write transaction.
pub fn prepare_cfp_seed(input: &[u8]) -> Result<PreparedCfpSeed, CfpRepositoryError> {
    let seed: CfpSeed = serde_json::from_slice(input)?;
    let journals = prepare_seed(&seed)?;
    let notices = journals.values().map(|journal| journal.notices.len()).sum();
    Ok(PreparedCfpSeed {
        journals,
        notices,
        digest: hex::encode(Sha256::digest(input)),
    })
}

fn prepare_seed(seed: &CfpSeed) -> Result<BTreeMap<String, PreparedJournal>, CfpRepositoryError> {
    if seed.format_version != 1 {
        return Err(CfpRepositoryError::Invalid(
            "Unsupported CFP input format".into(),
        ));
    }
    let mut journals: BTreeMap<String, PreparedJournal> = BTreeMap::new();
    for source in &seed.sources {
        let notice = parse_cfp_source(source).ok_or_else(|| {
            CfpRepositoryError::Invalid(format!(
                "Invalid CFP source: {} / {}",
                source.journal_title, source.title
            ))
        })?;
        let journal = journals
            .entry(source.catalog_ids[0].clone())
            .or_insert_with(|| PreparedJournal {
                title: source.journal_title.clone(),
                checked_on: source.checked_on.clone(),
                aliases: BTreeSet::new(),
                notices: Vec::new(),
                empty: None,
            });
        journal.aliases.extend(source.catalog_ids.iter().cloned());
        if journal
            .notices
            .iter()
            .any(|(_, existing)| existing.id == notice.id)
        {
            return Err(CfpRepositoryError::Invalid(format!(
                "Duplicate CFP notice within {}",
                source.catalog_ids[0]
            )));
        }
        journal.checked_on = journal.checked_on.clone().max(source.checked_on.clone());
        journal.notices.push((source.clone(), notice));
    }
    for empty in &seed.empty_journals {
        if empty.catalog_ids.is_empty()
            || empty.catalog_ids.iter().any(|id| id.trim().is_empty())
            || empty.journal_title.trim().is_empty()
            || empty.source_statement.trim().is_empty()
            || !empty.notices.is_empty()
            || !is_cfp_source_url(&empty.source_url)
            || chrono::NaiveDate::parse_from_str(&empty.checked_on, "%Y-%m-%d").is_err()
        {
            return Err(CfpRepositoryError::Invalid(
                "Invalid verified-empty CFP statement".into(),
            ));
        }
        if journals
            .insert(
                empty.catalog_ids[0].clone(),
                PreparedJournal {
                    title: empty.journal_title.clone(),
                    checked_on: empty.checked_on.clone(),
                    aliases: empty.catalog_ids.iter().cloned().collect(),
                    notices: Vec::new(),
                    empty: Some(empty.clone()),
                },
            )
            .is_some()
        {
            return Err(CfpRepositoryError::Invalid(
                "A CFP journal cannot be both empty and populated in one source snapshot".into(),
            ));
        }
    }
    let mut owners = BTreeMap::new();
    for (key, journal) in &journals {
        for alias in &journal.aliases {
            if owners.insert(alias, key).is_some_and(|owner| owner != key) {
                return Err(CfpRepositoryError::Invalid(format!(
                    "Conflicting CFP alias ownership: {alias}"
                )));
            }
        }
    }
    let count: usize = journals.values().map(|journal| journal.notices.len()).sum();
    if seed
        .expected_journals
        .is_some_and(|expected| expected != journals.len())
        || seed
            .expected_notices
            .is_some_and(|expected| expected != count)
    {
        return Err(CfpRepositoryError::Invalid(
            "CFP seed counts do not match the reviewed export".into(),
        ));
    }
    Ok(journals)
}

fn insert_journal(
    transaction: &Transaction<'_>,
    key: &str,
    journal: &PreparedJournal,
    source_key: &str,
) -> Result<(), CfpRepositoryError> {
    transaction.execute("INSERT INTO cfp_journals(journal_key,title,checked_on,source_url,source_statement) VALUES (?1,?2,?3,?4,?5)", params![key,journal.title,journal.checked_on,journal.empty.as_ref().map(|empty| &empty.source_url),journal.empty.as_ref().map(|empty| &empty.source_statement)])?;
    for alias in &journal.aliases {
        transaction.execute(
            "INSERT INTO cfp_journal_aliases(catalog_id,journal_key) VALUES (?1,?2)",
            params![alias, key],
        )?;
    }
    transaction.execute(
        "INSERT INTO cfp_source_journals(source_key,journal_key) VALUES (?1,?2)",
        params![source_key, key],
    )?;
    insert_notices(transaction, key, source_key, &journal.notices)
}

fn insert_notices(
    transaction: &Transaction<'_>,
    key: &str,
    source_key: &str,
    notices: &[(CfpSource, CfpNotice)],
) -> Result<(), CfpRepositoryError> {
    for (order, (source, notice)) in notices.iter().enumerate() {
        transaction.execute("INSERT INTO cfp_notices(journal_key,notice_key,source_key,source_json,normalized_json,display_order) VALUES (?1,?2,?3,?4,?5,?6)", params![key,notice.id,source_key,serde_json::to_string(source)?,serde_json::to_string(notice)?,order as i64])?;
    }
    Ok(())
}

/// Import a validated seed once under an immutable import identity.
///
/// The marker is rechecked inside an immediate transaction, so concurrent startup
/// cannot overwrite an online refresh. Existing journal ownership is never merged.
pub fn import_cfp_seed(
    path: &Path,
    seed_id: &str,
    input: &[u8],
) -> Result<CfpImportResult, CfpRepositoryError> {
    import_prepared_cfp_seed(path, seed_id, &prepare_cfp_seed(input)?)
}

/// Publish an already validated seed once, rechecking its marker under the write lock.
pub fn import_prepared_cfp_seed(
    path: &Path,
    seed_id: &str,
    seed: &PreparedCfpSeed,
) -> Result<CfpImportResult, CfpRepositoryError> {
    let journals = &seed.journals;
    let notices = seed.notices;
    let digest = &seed.digest;
    let mut connection = open_sqlite_connection(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(previous) = transaction
        .query_row(
            "SELECT content_hash FROM cfp_seed_imports WHERE seed_id=?1",
            [seed_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        if previous != *digest {
            return Err(CfpRepositoryError::Invalid(
                "CFP seed identity has different content; use a new import identity".into(),
            ));
        }
        return Ok(CfpImportResult {
            did_import: false,
            journals: journals.len(),
            notices,
            content_hash: digest.clone(),
        });
    }
    for (key, journal) in journals {
        let source_key = format!("journal:{key}");
        let capture = serde_json::to_string(
            &journal
                .notices
                .iter()
                .map(|(source, _)| source)
                .collect::<Vec<_>>(),
        )?;
        let source_url = journal
            .empty
            .as_ref()
            .map(|empty| empty.source_url.as_str())
            .or_else(|| {
                journal
                    .notices
                    .first()
                    .map(|(source, _)| source.source_url.as_str())
            })
            .unwrap_or_default();
        transaction.execute("INSERT INTO cfp_sources(source_key,url,parser_version,capture,capture_format,content_hash,status) VALUES (?1,?2,?3,?4,'seed_json',?5,'snapshot')", params![source_key,source_url,CFP_PARSER_VERSION,capture,hex::encode(Sha256::digest(capture.as_bytes()))])?;
        insert_journal(&transaction, key, journal, &source_key)?;
    }
    transaction.execute("INSERT INTO cfp_seed_imports(seed_id,format_version,content_hash,parser_version,journal_count,notice_count) VALUES (?1,?2,?3,?4,?5,?6)", params![seed_id,1,digest,CFP_PARSER_VERSION,journals.len() as i64,notices as i64])?;
    transaction.commit()?;
    Ok(CfpImportResult {
        did_import: true,
        journals: journals.len(),
        notices,
        content_hash: digest.clone(),
    })
}

/// Load all persisted journals consistently in one read transaction.
pub fn load_cfp_journals(path: &Path) -> Result<Vec<CfpJournalSnapshot>, CfpRepositoryError> {
    let mut connection = open_sqlite_connection(path)?;
    let transaction = connection.transaction()?;
    let mut journals: BTreeMap<String, CfpJournalSnapshot> = transaction.prepare("SELECT journal_key,title,checked_on,source_url,source_statement FROM cfp_journals ORDER BY journal_key")?.query_map([], |row| {
        let key: String = row.get(0)?;
        Ok((key.clone(), CfpJournalSnapshot { journal_key:key, journal_title:row.get(1)?,checked_on:row.get(2)?,source_url:row.get(3)?,source_statement:row.get(4)?,catalog_ids:Vec::new(),notices:Vec::new(),sources:Vec::new() }))
    })?.collect::<Result<_,_>>()?;
    for row in transaction
        .prepare("SELECT catalog_id,journal_key FROM cfp_journal_aliases ORDER BY catalog_id")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
    {
        let (alias, key) = row?;
        if let Some(journal) = journals.get_mut(&key) {
            journal.catalog_ids.push(alias);
        }
    }
    for row in transaction.prepare("SELECT journal_key,normalized_json FROM cfp_notices ORDER BY journal_key,display_order,notice_key")?.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))? {
        let (key,json)=row?;
        if let Some(journal)=journals.get_mut(&key) { journal.notices.push(serde_json::from_str(&json)?); }
    }
    for row in transaction.prepare("SELECT j.journal_key,s.source_key,s.status,s.last_attempt,s.last_success,s.last_error,s.revision,s.lease_expires_at FROM cfp_source_journals j JOIN cfp_sources s USING(source_key) ORDER BY j.journal_key,s.source_key")?.query_map([], |row| Ok((row.get::<_,String>(0)?, CfpSourceStatus {source_key:row.get(1)?,status:row.get(2)?,last_attempt:row.get(3)?,last_success:row.get(4)?,last_error:row.get(5)?,revision:row.get(6)?,lease_expires_at:row.get(7)?})))? {
        let (key,source)=row?;
        if let Some(journal)=journals.get_mut(&key) { journal.sources.push(source); }
    }
    Ok(journals.into_values().collect())
}

/// Read last-good original fragments for a filtered discovery list's retention policy.
pub fn load_cfp_source_originals(
    path: &Path,
    source_key: &str,
) -> Result<Vec<CfpSource>, CfpRepositoryError> {
    let connection = open_sqlite_connection(path)?;
    let mut statement = connection.prepare(
        "SELECT source_json FROM cfp_notices WHERE source_key=?1 ORDER BY display_order,notice_key",
    )?;
    let originals = statement
        .query_map([source_key], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    originals
        .into_iter()
        .map(|original| serde_json::from_str(&original).map_err(Into::into))
        .collect()
}

/// Reserve a new refresh generation without holding a transaction during I/O.
pub fn begin_cfp_refresh(
    path: &Path,
    source_key: &str,
    now: i64,
    lease_seconds: i64,
) -> Result<CfpRefreshLease, CfpRepositoryError> {
    if !(1..=600).contains(&lease_seconds) {
        return Err(CfpRepositoryError::Invalid(
            "CFP lease must be between 1 and 600 seconds".into(),
        ));
    }
    let connection = open_sqlite_connection(path)?;
    let expires_at = now
        .checked_add(lease_seconds)
        .ok_or_else(|| CfpRepositoryError::Invalid("Invalid CFP lease timestamp".into()))?;
    let generation = connection.query_row("UPDATE cfp_sources SET generation=generation+1,lease_expires_at=?2,last_attempt=?3,last_error=NULL,status='refreshing' WHERE source_key=?1 RETURNING generation",params![source_key,expires_at,now], |row| row.get(0)).optional()?.ok_or_else(|| CfpRepositoryError::Invalid("CFP source is not registered in storage".into()))?;
    Ok(CfpRefreshLease {
        source_key: source_key.into(),
        generation,
        expires_at,
    })
}

/// Record a failed/unsupported attempt while retaining all last-good data.
pub fn fail_cfp_refresh(
    path: &Path,
    lease: &CfpRefreshLease,
    reason: &str,
    is_unsupported: bool,
) -> Result<(), CfpRepositoryError> {
    let connection = open_sqlite_connection(path)?;
    let reason: String = reason.chars().take(1000).collect();
    let changed = connection.execute("UPDATE cfp_sources SET status=?3,last_error=?4,lease_expires_at=NULL WHERE source_key=?1 AND generation=?2 AND lease_expires_at IS NOT NULL", params![lease.source_key,lease.generation,if is_unsupported {"unsupported"} else {"failed"},reason])?;
    if changed != 1 {
        return Err(CfpRepositoryError::StaleRefresh);
    }
    Ok(())
}

/// Atomically replace a complete verified source snapshot if its generation is current.
pub fn publish_cfp_refresh(
    path: &Path,
    lease: &CfpRefreshLease,
    publication: &CfpPublication,
    now: i64,
) -> Result<(), CfpRepositoryError> {
    publish_cfp_snapshot(path, lease, publication, now, None)
}

/// Publish verified full-text replacements while retaining unresolved original records.
///
/// Partial enrichment preserves the last complete-refresh timestamp and records its limitation.
pub fn publish_cfp_full_text_refresh(
    path: &Path,
    lease: &CfpRefreshLease,
    publication: &CfpPublication,
    now: i64,
    unresolved: usize,
) -> Result<(), CfpRepositoryError> {
    let identities = |sources: &[CfpSource]| {
        let mut identities: Vec<_> = sources
            .iter()
            .map(|source| (source.catalog_ids.clone(), source.title.clone()))
            .collect();
        identities.sort();
        identities
    };
    if identities(&load_cfp_source_originals(path, &lease.source_key)?)
        != identities(&publication.sources)
    {
        return Err(CfpRepositoryError::Invalid(
            "Full-text enrichment cannot remove or replace existing notice identities".into(),
        ));
    }
    let warning = (unresolved > 0).then(|| format!("Full original text remains unadapted for {unresolved} notices; previous records retained"));
    publish_cfp_snapshot(path, lease, publication, now, warning.as_deref())
}

fn publish_cfp_snapshot(
    path: &Path,
    lease: &CfpRefreshLease,
    publication: &CfpPublication,
    now: i64,
    warning: Option<&str>,
) -> Result<(), CfpRepositoryError> {
    if publication.capture.trim().is_empty() || !is_cfp_source_url(&publication.source_url) {
        return Err(CfpRepositoryError::Invalid(
            "CFP publication requires a verified capture and URL".into(),
        ));
    }
    let prepared = prepare_seed(&CfpSeed {
        format_version: 1,
        sources: publication.sources.clone(),
        empty_journals: publication.empty_journals.clone(),
        expected_journals: None,
        expected_notices: None,
    })?;
    let mut connection = open_sqlite_connection(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bindings: BTreeSet<String> = transaction
        .prepare("SELECT journal_key FROM cfp_source_journals WHERE source_key=?1")?
        .query_map([&lease.source_key], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    if bindings.is_empty() || bindings != prepared.keys().cloned().collect() {
        return Err(CfpRepositoryError::Invalid(
            "CFP publication does not cover every registered journal binding".into(),
        ));
    }
    let changed = transaction.execute("UPDATE cfp_sources SET capture=?3,capture_format=?4,content_hash=?5,url=?6,parser_version=?7,config_version=?8,last_success=CASE WHEN ?10 IS NULL THEN ?9 ELSE last_success END,last_error=?10,status=CASE WHEN ?10 IS NULL THEN 'success' ELSE 'failed' END,revision=revision+1,lease_expires_at=NULL WHERE source_key=?1 AND generation=?2 AND lease_expires_at>=?9",params![lease.source_key,lease.generation,publication.capture,publication.capture_format,hex::encode(Sha256::digest(publication.capture.as_bytes())),publication.source_url,CFP_PARSER_VERSION,publication.config_version,now,warning])?;
    if changed != 1 || now > lease.expires_at {
        return Err(CfpRepositoryError::StaleRefresh);
    }
    for (key, journal) in prepared {
        let aliases: BTreeSet<String> = transaction
            .prepare("SELECT catalog_id FROM cfp_journal_aliases WHERE journal_key=?1")?
            .query_map([&key], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        if !journal.aliases.is_subset(&aliases) {
            return Err(CfpRepositoryError::Invalid(
                "CFP refresh cannot change journal alias ownership".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM cfp_notices WHERE source_key=?1 AND journal_key=?2",
            params![lease.source_key, key],
        )?;
        insert_notices(&transaction, &key, &lease.source_key, &journal.notices)?;
        transaction.execute("UPDATE cfp_journals SET checked_on=?2,source_url=?3,source_statement=?4 WHERE journal_key=?1",params![key,journal.checked_on,journal.empty.as_ref().map(|empty|&empty.source_url),journal.empty.as_ref().map(|empty|&empty.source_statement)])?;
    }
    transaction.commit()?;
    Ok(())
}
