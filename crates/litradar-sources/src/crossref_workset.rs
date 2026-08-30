//! Disposable, bounded Crossref collection with durable count checks and local ordering.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use litradar_domain::{normalize_contract_date, normalize_contract_doi};
use litradar_provider::{ProviderError, ProviderErrorKind};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::providers::{crossref_date, crossref_work_issue_anchor, ScholarlyAnchor};
use crate::scholarly::{CrossrefQuery, CrossrefWorksPage, CROSSREF_ROWS};

const APPLICATION_ID: i32 = 0x4c524357;
const SCHEMA_VERSION: u32 = 1;
const MAXIMUM_PAGES: u64 = 1_048_576;
const MAXIMUM_BATCH_BYTES: usize = 16 * 1024 * 1024;
const OWNER: &str = "litradar-crossref-workset";

/// Frozen traversal reference stored in the core checkpoint, without filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrossrefCheckpoint {
    /// Random basename, never an arbitrary path.
    pub(crate) token: String,
    /// Journal endpoint selected from the maintained catalog.
    pub(crate) issn: String,
    /// Inclusive UTC upper bound shared by the entire traversal.
    pub(crate) frozen_at: i64,
    /// Earliest discovered creation second across the whole journal.
    pub(crate) created_from: Option<i64>,
    /// Existing anchor-year update filter, absent during unbounded replay.
    pub(crate) updated_from: Option<String>,
    /// Count reported by the first root query of this generation.
    pub(crate) root_total: Option<u64>,
    /// Frozen candidate issue also checked against the outer checkpoint.
    pub(crate) candidate: Option<ScholarlyAnchor>,
    /// Persisted whole-journal drift retry counter.
    pub(crate) generation: u8,
    /// Monotonic local operation number confirmed through the core ACK.
    pub(crate) sequence: u64,
    /// Exact next collection request or immutable emission position.
    pub(crate) phase: CrossrefPhase,
}

/// Collection or immutable emission position within one workset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CrossrefPhase {
    /// Discover creation history independently of the update filter.
    Discover,
    /// Probe one partition, or continue its dense-second cursor.
    Collect {
        partition: i64,
        from: i64,
        until: i64,
        cursor: Option<String>,
        received: u64,
        expected: Option<u64>,
        retry: u8,
    },
    /// All leaf, parent and root counts have reconciled.
    Ready,
    /// Read the sealed candidate-to-base issue window.
    Emit {
        upper: String,
        lower: Option<String>,
        after: Option<EmissionKey>,
    },
}

/// Keyset position in the verified group and work ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmissionKey {
    rank: i64,
    date: String,
    key: String,
}

/// One bounded page read from an immutable workset.
pub(crate) struct EmissionPage {
    /// Consumed payloads within both row and byte limits.
    pub(crate) works: Vec<Value>,
    /// Last emitted record in the indexed ordering.
    pub(crate) after: Option<EmissionKey>,
    /// Whether another bounded read is required before completion.
    pub(crate) has_more: bool,
}

/// An issue group and its maximum publication date used for boundary selection.
pub(crate) struct IssueGroup {
    /// Unchanged stable issue identity.
    pub(crate) anchor: ScholarlyAnchor,
    /// Maximum complete sorting date within the group.
    pub(crate) date: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerRecord {
    owner: String,
    version: u32,
    token: String,
    scope: String,
    issn: String,
    frozen_at: i64,
    updated_from: Option<String>,
}

/// A SQLite workset whose ownership has been checked against the core's frozen context.
pub(crate) struct CrossrefWorkset {
    connection: Connection,
    root: PathBuf,
    owner: OwnerRecord,
    state: CrossrefCheckpoint,
}

fn invalid(message: impl Into<String>) -> ProviderError {
    ProviderError::new(ProviderErrorKind::InvalidResponse, message)
}

fn storage_error(error: impl std::fmt::Display + 'static) -> ProviderError {
    if (&error as &dyn std::any::Any)
        .downcast_ref::<rusqlite::Error>()
        .is_some_and(|error| {
            matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
            )
        })
    {
        return invalid("Crossref workset database is damaged");
    }
    ProviderError::new(
        ProviderErrorKind::Internal,
        format!("Crossref workset: {error}"),
    )
}

fn encode(value: &impl Serialize) -> Result<String, ProviderError> {
    serde_json::to_string(value).map_err(storage_error)
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, ProviderError> {
    serde_json::from_str(value).map_err(|_| invalid("invalid Crossref workset metadata"))
}

fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

fn new_token() -> Result<String, ProviderError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(storage_error)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn validate_path(path: &Path, is_directory: bool) -> Result<(), ProviderError> {
    let metadata = fs::symlink_metadata(path).map_err(storage_error)?;
    #[cfg(windows)]
    let is_link = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let is_link = metadata.file_type().is_symlink();
    if is_link || metadata.is_dir() != is_directory || (!is_directory && !metadata.is_file()) {
        return Err(invalid(
            "Crossref workset path is not an ordinary owned file or directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !is_directory && metadata.nlink() != 1 {
            return Err(invalid("Crossref workset hard links are not allowed"));
        }
    }
    Ok(())
}

fn prepare_root(root: &Path) -> Result<PathBuf, ProviderError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(invalid(
            "Crossref workset root must be an absolute core-owned path",
        ));
    }
    for ancestor in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => validate_path(ancestor, true)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(ancestor).map_err(storage_error)?
            }
            Err(error) => return Err(storage_error(error)),
        }
    }
    fs::canonicalize(root).map_err(storage_error)
}

fn owned_paths(root: &Path, token: &str) -> Result<Vec<PathBuf>, ProviderError> {
    if !valid_token(token) {
        return Err(invalid("Crossref workset token is invalid"));
    }
    Ok([
        ".sqlite",
        ".sqlite-journal",
        ".sqlite-wal",
        ".sqlite-shm",
        ".json",
    ]
    .into_iter()
    .map(|suffix| root.join(format!("{token}{suffix}")))
    .collect())
}

fn validate_owned_files(root: &Path, owner: &OwnerRecord) -> Result<(), ProviderError> {
    if prepare_root(root)? != root {
        return Err(invalid("Crossref workset root changed"));
    }
    let paths = owned_paths(root, &owner.token)?;
    for path in &paths {
        match fs::symlink_metadata(path) {
            Ok(_) => validate_path(path, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
    }
    let manifest_path = paths.last().expect("owned manifest path");
    if fs::metadata(manifest_path).map_err(storage_error)?.len() > 65_536 {
        return Err(invalid("Crossref ownership record is too large"));
    }
    let manifest = fs::read_to_string(manifest_path).map_err(storage_error)?;
    let actual: OwnerRecord = decode(&manifest)?;
    if &actual != owner {
        return Err(invalid(
            "Crossref workset ownership or frozen context does not match",
        ));
    }
    Ok(())
}

fn remove_owned_files(root: &Path, owner: &OwnerRecord) -> Result<(), ProviderError> {
    validate_owned_files(root, owner)?;
    for path in owned_paths(root, &owner.token)? {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
    }
    Ok(())
}

impl CrossrefCheckpoint {
    /// Start a traversal with a fresh token and a frozen upper bound.
    pub(crate) fn new(
        issn: String,
        frozen_at: i64,
        updated_from: Option<String>,
    ) -> Result<Self, ProviderError> {
        let checkpoint = Self {
            token: new_token()?,
            issn,
            frozen_at,
            created_from: None,
            updated_from,
            root_total: None,
            candidate: None,
            generation: 0,
            sequence: 0,
            phase: CrossrefPhase::Discover,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    /// Reject malformed traversal references before consulting the filesystem.
    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        if !valid_token(&self.token)
            || self.issn.trim().is_empty()
            || self.generation > 1
            || chrono::DateTime::from_timestamp(self.frozen_at, 0).is_none()
            || self.created_from.is_some_and(|from| {
                from > self.frozen_at || chrono::DateTime::from_timestamp(from, 0).is_none()
            })
            || self.root_total.is_some_and(|count| count > i64::MAX as u64)
            || self.updated_from.as_ref().is_some_and(|date| {
                date.len() != 10
                    || normalize_contract_date(date)
                        .is_none_or(|normalized| normalized.value != *date)
            })
            || self
                .candidate
                .as_ref()
                .is_some_and(|anchor| !crate::providers::is_valid_scholarly_anchor(anchor))
        {
            return Err(invalid(
                "Crossref checkpoint has invalid frozen bounds or counters",
            ));
        }
        let valid = match &self.phase {
            CrossrefPhase::Discover => self.created_from.is_none() && self.root_total.is_none(),
            CrossrefPhase::Collect {
                partition,
                from,
                until,
                cursor,
                received,
                expected,
                retry,
            } => {
                *partition > 0
                    && (*partition == 1 || self.root_total.is_some())
                    && self.created_from.is_some_and(|start| *from >= start)
                    && from <= until
                    && *until <= self.frozen_at
                    && *retry <= 1
                    && expected.is_none_or(|total| total <= i64::MAX as u64 && *received <= total)
                    && match cursor {
                        Some(value) => {
                            from == until
                                && !value.is_empty()
                                && expected.is_some_and(|count| count > CROSSREF_ROWS as u64)
                        }
                        None => *received == 0,
                    }
            }
            CrossrefPhase::Ready => {
                self.root_total.is_some()
                    && (self.created_from.is_some() || self.root_total == Some(0))
            }
            CrossrefPhase::Emit {
                upper,
                lower,
                after,
            } => {
                self.root_total.is_some()
                    && (self.created_from.is_some() || self.root_total == Some(0))
                    && valid_sort_date(upper)
                    && lower.as_ref().is_none_or(|date| valid_sort_date(date))
                    && lower.as_ref().is_none_or(|value| value <= upper)
                    && after.as_ref().is_none_or(|key| {
                        key.rank > 0
                            && !key.key.is_empty()
                            && (key.date.is_empty() || valid_sort_date(&key.date))
                    })
            }
        };
        if !valid {
            return Err(invalid("Crossref checkpoint phase is inconsistent"));
        }
        Ok(())
    }

    /// Construct exactly the query represented by this confirmed collection step.
    pub(crate) fn query(&self) -> Result<CrossrefQuery, ProviderError> {
        match &self.phase {
            CrossrefPhase::Discover => Ok(CrossrefQuery::EarliestCreated {
                until: self.frozen_at,
            }),
            CrossrefPhase::Collect {
                from,
                until,
                cursor,
                ..
            } => Ok(CrossrefQuery::Works {
                created_from: *from,
                created_until: *until,
                updated_from: self.updated_from.clone(),
                updated_until: self.updated_from.as_ref().map(|_| self.frozen_at),
                cursor: cursor.clone(),
            }),
            _ => Err(invalid("Crossref workset has no pending network query")),
        }
    }

    fn restart_collection(&mut self) {
        self.phase = match self.created_from {
            Some(from) => CrossrefPhase::Collect {
                partition: 1,
                from,
                until: self.frozen_at,
                cursor: None,
                received: 0,
                expected: None,
                retry: 0,
            },
            None => CrossrefPhase::Discover,
        };
    }
}

impl CrossrefWorkset {
    fn owner(scope: &str, state: &CrossrefCheckpoint) -> OwnerRecord {
        OwnerRecord {
            owner: OWNER.to_string(),
            version: SCHEMA_VERSION,
            token: state.token.clone(),
            scope: scope.to_string(),
            issn: state.issn.clone(),
            frozen_at: state.frozen_at,
            updated_from: state.updated_from.clone(),
        }
    }

    /// Create a disposable database using only the core-provided root and generated token.
    pub(crate) fn create(
        root: &Path,
        scope: &str,
        state: CrossrefCheckpoint,
    ) -> Result<Self, ProviderError> {
        state.validate()?;
        let root = prepare_root(root)?;
        let owner = Self::owner(scope, &state);
        let paths = owned_paths(&root, &state.token)?;
        if paths.iter().any(|path| fs::symlink_metadata(path).is_ok()) {
            return Err(invalid("Crossref workset token already exists"));
        }
        let mut manifest = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(paths.last().expect("manifest path"))
            .map_err(storage_error)?;
        manifest
            .write_all(encode(&owner)?.as_bytes())
            .map_err(storage_error)?;
        manifest.sync_all().map_err(storage_error)?;
        let connection = Connection::open_with_flags(
            &paths[0],
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(storage_error)?;
        Self::configure(&connection)?;
        connection.execute_batch(&format!("PRAGMA page_size=4096; PRAGMA max_page_count={MAXIMUM_PAGES};
            PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={SCHEMA_VERSION};
            CREATE TABLE metadata (id INTEGER PRIMARY KEY CHECK (id=1), owner TEXT NOT NULL, state TEXT NOT NULL, previous TEXT);
            CREATE TABLE partitions (id INTEGER PRIMARY KEY, parent INTEGER, first_second INTEGER NOT NULL,
                last_second INTEGER NOT NULL, depth INTEGER NOT NULL, expected INTEGER, received INTEGER NOT NULL DEFAULT 0,
                cursor TEXT, retry INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL);
            CREATE TABLE works (key TEXT PRIMARY KEY, payload TEXT NOT NULL, leaf INTEGER NOT NULL,
                date TEXT NOT NULL, fingerprint TEXT NOT NULL, anchor TEXT, year INTEGER NOT NULL,
                volume TEXT NOT NULL, issue TEXT NOT NULL, group_rank INTEGER NOT NULL DEFAULT 0,
                sort_date INTEGER GENERATED ALWAYS AS (-CAST(replace(date,'-','') AS INTEGER)) STORED);
            CREATE INDEX works_leaf ON works(leaf);
            CREATE INDEX works_order ON works(fingerprint, date DESC, key);
            CREATE INDEX works_emit ON works(group_rank,sort_date,key);
            CREATE TABLE groups (fingerprint TEXT PRIMARY KEY, anchor TEXT, date TEXT NOT NULL,
                year INTEGER NOT NULL, volume TEXT NOT NULL, issue TEXT NOT NULL, rank INTEGER);
            CREATE INDEX groups_order ON groups(date DESC,year DESC,volume DESC,issue DESC,fingerprint);
            CREATE UNIQUE INDEX groups_rank ON groups(rank);" )).map_err(storage_error)?;
        connection
            .execute(
                "INSERT INTO metadata VALUES(1, ?1, ?2, NULL)",
                params![encode(&owner)?, encode(&state)?],
            )
            .map_err(storage_error)?;
        let workset = Self {
            connection,
            root,
            owner,
            state,
        };
        workset.initialize_root()?;
        Ok(workset)
    }

    fn configure(connection: &Connection) -> Result<(), ProviderError> {
        connection
            .execute_batch(
                "PRAGMA cache_size=-4096; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
            PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
            PRAGMA max_page_count=1048576;",
            )
            .map_err(storage_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage_error)
    }

    fn initialize_root(&self) -> Result<(), ProviderError> {
        if let Some(from) = self.state.created_from {
            self.connection.execute("INSERT INTO partitions(id,parent,first_second,last_second,depth,status) VALUES(1,NULL,?1,?2,0,'probe')",
                params![from, self.state.frozen_at]).map_err(storage_error)?;
        }
        Ok(())
    }

    /// Restore the exact confirmed step, replaying one cache-ahead result without another request.
    pub(crate) fn open(
        root: &Path,
        scope: &str,
        checkpoint: &CrossrefCheckpoint,
    ) -> Result<(Self, bool), ProviderError> {
        checkpoint.validate()?;
        let root = prepare_root(root)?;
        let owner = Self::owner(scope, checkpoint);
        let paths = owned_paths(&root, &checkpoint.token)?;
        let has_manifest = fs::symlink_metadata(paths.last().expect("manifest path")).is_ok();
        if !has_manifest && paths.iter().any(|path| fs::symlink_metadata(path).is_ok()) {
            return Err(invalid("Crossref workset files have no ownership record"));
        }
        if has_manifest {
            validate_owned_files(&root, &owner)?;
        }
        if !has_manifest || !paths[0].exists() {
            if has_manifest {
                remove_owned_files(&root, &owner)?;
            }
            let mut restored = checkpoint.clone();
            restored.token = new_token()?;
            restored.sequence = restored
                .sequence
                .checked_add(1)
                .ok_or_else(|| invalid("Crossref sequence overflow"))?;
            restored.restart_collection();
            if restored.created_from.is_none() {
                restored.root_total = None;
            }
            return Self::create(&root, scope, restored).map(|workset| (workset, true));
        }
        match Self::open_existing(root.clone(), owner, checkpoint) {
            Err(error) if error.to_string() == "Crossref workset database is damaged" => {
                let owner = Self::owner(scope, checkpoint);
                remove_owned_files(&root, &owner)?;
                let mut restored = checkpoint.clone();
                restored.token = new_token()?;
                restored.sequence = restored
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| invalid("Crossref sequence overflow"))?;
                restored.restart_collection();
                if restored.created_from.is_none() {
                    restored.root_total = None;
                }
                Self::create(&root, scope, restored).map(|workset| (workset, true))
            }
            result => result,
        }
    }

    fn open_existing(
        root: PathBuf,
        owner: OwnerRecord,
        checkpoint: &CrossrefCheckpoint,
    ) -> Result<(Self, bool), ProviderError> {
        let paths = owned_paths(&root, &checkpoint.token)?;
        let connection = Connection::open_with_flags(
            &paths[0],
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(storage_error)?;
        let application: i32 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(storage_error)?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(storage_error)?;
        if application != APPLICATION_ID || version != SCHEMA_VERSION {
            return Err(invalid(
                "Crossref workset database is foreign or has an unsupported schema",
            ));
        }
        let (stored_owner, state, previous): (String, String, Option<String>) = connection
            .query_row(
                "SELECT owner,state,previous FROM metadata WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(storage_error)?;
        if decode::<OwnerRecord>(&stored_owner)? != owner {
            return Err(invalid("Crossref workset database context does not match"));
        }
        let stored: CrossrefCheckpoint =
            decode(&state).map_err(|_| invalid("Crossref workset database is damaged"))?;
        stored
            .validate()
            .map_err(|_| invalid("Crossref workset database is damaged"))?;
        Self::configure(&connection)?;
        let is_emitting = match (&checkpoint.phase, &stored.phase) {
            (
                CrossrefPhase::Emit { upper, lower, .. },
                CrossrefPhase::Emit {
                    upper: stored_upper,
                    lower: stored_lower,
                    after: None,
                },
            ) => upper == stored_upper && lower == stored_lower,
            _ => false,
        } && stored.created_from == checkpoint.created_from
            && stored.root_total == checkpoint.root_total
            && stored.generation == checkpoint.generation
            && stored.candidate == checkpoint.candidate
            && checkpoint.sequence >= stored.sequence;
        let did_replay = if stored == *checkpoint || is_emitting {
            false
        } else if previous.as_deref() == Some(encode(checkpoint)?.as_str()) {
            true
        } else {
            return Err(invalid(
                "Crossref workset does not match the confirmed core checkpoint",
            ));
        };
        if matches!(stored.phase, CrossrefPhase::Emit { .. }) {
            connection
                .pragma_update(None, "query_only", true)
                .map_err(storage_error)?;
        }
        Ok((
            Self {
                connection,
                root,
                owner,
                state: if is_emitting {
                    checkpoint.clone()
                } else {
                    stored
                },
            },
            did_replay,
        ))
    }

    /// Return the current durable collection position.
    pub(crate) fn checkpoint(&self) -> CrossrefCheckpoint {
        self.state.clone()
    }

    /// Seal the selected issue window before any canonical article is emitted.
    pub(crate) fn seal_selection(
        &mut self,
        upper: String,
        lower: Option<String>,
        candidate: Option<ScholarlyAnchor>,
    ) -> Result<CrossrefCheckpoint, ProviderError> {
        if !matches!(self.state.phase, CrossrefPhase::Ready) {
            return Err(invalid("Crossref selection is already sealed"));
        }
        let previous = self.state.clone();
        let mut next = previous.clone();
        next.sequence = next
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("Crossref sequence overflow"))?;
        next.candidate = candidate;
        next.phase = CrossrefPhase::Emit {
            upper,
            lower,
            after: None,
        };
        next.validate()?;
        self.connection
            .execute(
                "UPDATE metadata SET state=?1,previous=?2 WHERE id=1",
                params![encode(&next)?, encode(&previous)?],
            )
            .map_err(storage_error)?;
        self.state = next;
        Ok(self.state.clone())
    }

    /// Persist a single response and all its progress counters in one SQLite transaction.
    pub(crate) fn accept(
        &mut self,
        page: CrossrefWorksPage,
    ) -> Result<CrossrefCheckpoint, ProviderError> {
        let previous = self.state.clone();
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage_error)?;
        let result = self.accept_response(page).and_then(|()| {
            self.state.sequence = previous
                .sequence
                .checked_add(1)
                .ok_or_else(|| invalid("Crossref sequence overflow"))?;
            self.state.validate()?;
            self.connection
                .execute(
                    "UPDATE metadata SET state=?1,previous=?2 WHERE id=1",
                    params![encode(&self.state)?, encode(&previous)?],
                )
                .map_err(storage_error)?;
            self.connection
                .execute_batch("COMMIT")
                .map_err(storage_error)?;
            Ok(self.state.clone())
        });
        if result.is_err() {
            self.state = previous;
            if !self.connection.is_autocommit() {
                self.connection
                    .execute_batch("ROLLBACK")
                    .map_err(storage_error)?;
            }
        }
        result
    }

    fn accept_response(&mut self, page: CrossrefWorksPage) -> Result<(), ProviderError> {
        if page.total_results > i64::MAX as u64 {
            return Err(invalid("Crossref count exceeds the supported range"));
        }
        match self.state.phase.clone() {
            CrossrefPhase::Discover => {
                if page.total_results == 0 && page.items.is_empty() {
                    self.state.root_total = Some(0);
                    self.state.phase = CrossrefPhase::Ready;
                    return Ok(());
                }
                if page.total_results == 0 || page.items.len() != 1 {
                    return Err(invalid("Crossref creation discovery is incomplete"));
                }
                let from = created_second(&page.items[0])?;
                if from > self.state.frozen_at {
                    return Err(invalid(
                        "Crossref creation discovery exceeds the frozen bound",
                    ));
                }
                self.state.created_from = Some(from);
                self.initialize_root()?;
                self.state.restart_collection();
                Ok(())
            }
            CrossrefPhase::Collect {
                partition,
                from,
                until,
                cursor,
                received,
                expected,
                retry,
            } => {
                if partition == 1 && cursor.is_none() {
                    if self
                        .state
                        .root_total
                        .is_some_and(|total| total != page.total_results)
                    {
                        return self.restart_generation("root count changed");
                    }
                    self.state.root_total = Some(page.total_results);
                }
                if expected.is_some_and(|expected| expected != page.total_results) {
                    return self.retry_leaf(partition, retry, "partition count changed");
                }
                if cursor.is_none() {
                    self.connection
                        .execute(
                            "UPDATE partitions SET expected=?1 WHERE id=?2",
                            params![page.total_results, partition],
                        )
                        .map_err(storage_error)?;
                    if page.items.len() as u64 != page.total_results.min(CROSSREF_ROWS as u64) {
                        return self.retry_leaf(partition, retry, "single response was truncated");
                    }
                    if page.total_results > CROSSREF_ROWS as u64 {
                        if from < until {
                            let depth: u32 = self
                                .connection
                                .query_row(
                                    "SELECT depth FROM partitions WHERE id=?1",
                                    [partition],
                                    |row| row.get(0),
                                )
                                .map_err(storage_error)?;
                            if depth >= 64 {
                                return Err(invalid("Crossref partition depth exceeded"));
                            }
                            let middle = split_second(from, until)?;
                            self.connection
                                .execute(
                                    "UPDATE partitions SET status='split' WHERE id=?1",
                                    [partition],
                                )
                                .map_err(storage_error)?;
                            for (first, last) in [
                                (from, middle),
                                (
                                    middle
                                        .checked_add(1)
                                        .ok_or_else(|| invalid("Crossref split overflow"))?,
                                    until,
                                ),
                            ] {
                                self.connection.execute("INSERT INTO partitions(parent,first_second,last_second,depth,status) VALUES(?1,?2,?3,?4,'probe')",
                                    params![partition, first, last, depth + 1]).map_err(storage_error)?;
                            }
                        } else {
                            self.connection
                                .execute(
                                    "UPDATE partitions SET status='cursor',cursor='*' WHERE id=?1",
                                    [partition],
                                )
                                .map_err(storage_error)?;
                        }
                        return self.next_partition();
                    }
                } else if page.items.len() > CROSSREF_ROWS {
                    return self.retry_leaf(
                        partition,
                        retry,
                        "cursor page exceeds the requested size",
                    );
                }
                let total_received = received
                    .checked_add(page.items.len() as u64)
                    .ok_or_else(|| invalid("Crossref received count overflow"))?;
                if total_received > page.total_results
                    || (page.items.is_empty() && total_received != page.total_results)
                    || (cursor.is_some()
                        && page.items.len() < CROSSREF_ROWS
                        && total_received != page.total_results)
                {
                    return self.retry_leaf(
                        partition,
                        retry,
                        "cursor count is incomplete or exceeds the expected count",
                    );
                }
                let is_terminal_page = cursor.is_none() || page.items.len() < CROSSREF_ROWS;
                let mut keys = BTreeSet::new();
                let mut prepared = Vec::with_capacity(page.items.len());
                for work in page.items {
                    let created = created_second(&work)?;
                    if !(from..=until).contains(&created) {
                        return self.retry_leaf(
                            partition,
                            retry,
                            "work moved outside its creation partition",
                        );
                    }
                    let payload = consumed_payload(work);
                    let serialized = encode(&payload)?;
                    let key = work_key(&payload, &serialized);
                    if !keys.insert(key.clone()) {
                        return self.retry_leaf(
                            partition,
                            retry,
                            "duplicate key inside one response",
                        );
                    }
                    let existing: Option<i64> = self
                        .connection
                        .query_row("SELECT leaf FROM works WHERE key=?1", [&key], |row| {
                            row.get(0)
                        })
                        .optional()
                        .map_err(storage_error)?;
                    if let Some(leaf) = existing {
                        if leaf != partition {
                            return self
                                .restart_generation("duplicate key across creation partitions");
                        }
                        return self.retry_leaf(
                            partition,
                            retry,
                            "cursor made no unique progress or repeated a key",
                        );
                    }
                    prepared.push((key, payload, serialized));
                }
                for (key, payload, serialized) in prepared {
                    let order = crossref_order(&payload, &key)?;
                    self.connection.execute("INSERT INTO works(key,payload,leaf,date,fingerprint,anchor,year,volume,issue) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                        params![key, serialized, partition, order.date, order.fingerprint, order.anchor, order.year, order.volume, order.issue]).map_err(storage_error)?;
                }
                let is_done = total_received == page.total_results && is_terminal_page;
                let next_cursor = if is_done {
                    None
                } else {
                    let Some(cursor) = page.next_cursor.filter(|value| !value.is_empty()) else {
                        return self.retry_leaf(
                            partition,
                            retry,
                            "cursor ended without a verified terminal response",
                        );
                    };
                    Some(cursor)
                };
                self.connection.execute("UPDATE partitions SET expected=?1,received=?2,cursor=?3,status=?4 WHERE id=?5",
                    params![page.total_results, total_received, next_cursor, if is_done { "done" } else { "cursor" }, partition]).map_err(storage_error)?;
                self.next_partition()
            }
            _ => Err(invalid(
                "Crossref response arrived after collection was verified",
            )),
        }
    }

    fn retry_leaf(&mut self, partition: i64, retry: u8, reason: &str) -> Result<(), ProviderError> {
        if retry >= 1 {
            return Err(invalid(format!(
                "Crossref partition drift persisted after one retry: {reason}"
            )));
        }
        self.connection
            .execute("DELETE FROM works WHERE leaf=?1", [partition])
            .map_err(storage_error)?;
        self.connection.execute("UPDATE partitions SET retry=retry+1,received=0,cursor=CASE WHEN status='cursor' THEN '*' ELSE NULL END WHERE id=?1", [partition]).map_err(storage_error)?;
        tracing::warn!(
            event = "source.crossref.partition_retried",
            reason,
            generation = self.state.generation
        );
        self.next_partition()
    }

    fn restart_generation(&mut self, reason: &str) -> Result<(), ProviderError> {
        if self.state.generation >= 1 {
            return Err(invalid(format!(
                "Crossref journal count drift persisted after one retry: {reason}"
            )));
        }
        self.connection
            .execute_batch("DELETE FROM works; DELETE FROM partitions; DELETE FROM groups;")
            .map_err(storage_error)?;
        self.state.generation += 1;
        self.state.root_total = None;
        self.initialize_root()?;
        self.state.restart_collection();
        tracing::warn!(
            event = "source.crossref.generation_retried",
            reason,
            generation = self.state.generation
        );
        Ok(())
    }

    fn next_partition(&mut self) -> Result<(), ProviderError> {
        let next = self.connection.query_row("SELECT id,first_second,last_second,cursor,received,expected,retry FROM partitions WHERE status IN ('probe','cursor') ORDER BY first_second,id LIMIT 1", [], |row| {
            Ok(CrossrefPhase::Collect { partition: row.get(0)?, from: row.get(1)?, until: row.get(2)?, cursor: row.get(3)?, received: row.get(4)?, expected: row.get(5)?, retry: row.get(6)? })
        }).optional().map_err(storage_error)?;
        if let Some(next) = next {
            self.state.phase = next;
            return Ok(());
        }
        let invalid_parents: u64 = self.connection.query_row("SELECT count(*) FROM partitions p WHERE p.status='split' AND p.expected != (SELECT sum(c.expected) FROM partitions c WHERE c.parent=p.id)", [], |row| row.get(0)).map_err(storage_error)?;
        let unique: u64 = self
            .connection
            .query_row("SELECT count(*) FROM works", [], |row| row.get(0))
            .map_err(storage_error)?;
        if invalid_parents != 0 || self.state.root_total != Some(unique) {
            return self.restart_generation(
                "partition tree does not reconcile with the global unique count",
            );
        }
        self.connection.execute_batch("INSERT INTO groups(fingerprint,anchor,date,year,volume,issue)
            SELECT fingerprint,min(anchor),max(date),max(year),max(volume),max(issue) FROM works INDEXED BY works_order GROUP BY fingerprint;").map_err(storage_error)?;
        let mut statement = self.connection.prepare("SELECT fingerprint FROM groups INDEXED BY groups_order ORDER BY date DESC,year DESC,volume DESC,issue DESC,fingerprint").map_err(storage_error)?;
        let mut rows = statement.query([]).map_err(storage_error)?;
        let mut rank = 0_i64;
        while let Some(row) = rows.next().map_err(storage_error)? {
            rank += 1;
            let fingerprint: String = row.get(0).map_err(storage_error)?;
            self.connection
                .execute(
                    "UPDATE groups SET rank=?1 WHERE fingerprint=?2",
                    params![rank, fingerprint],
                )
                .map_err(storage_error)?;
        }
        self.connection.execute("UPDATE works SET group_rank=(SELECT rank FROM groups WHERE groups.fingerprint=works.fingerprint)",[]).map_err(storage_error)?;
        self.state.phase = CrossrefPhase::Ready;
        Ok(())
    }

    /// Find the first safe issue in the complete, locally sorted result.
    pub(crate) fn first_group(&self) -> Result<Option<IssueGroup>, ProviderError> {
        self.read_group(
            "SELECT anchor,date FROM groups WHERE anchor IS NOT NULL ORDER BY rank LIMIT 1",
            None,
        )
    }

    /// Find a frozen issue regardless of the upstream retrieval order.
    pub(crate) fn group(
        &self,
        anchor: &ScholarlyAnchor,
    ) -> Result<Option<IssueGroup>, ProviderError> {
        self.read_group(
            "SELECT anchor,date FROM groups WHERE fingerprint=?1",
            Some(encode(&anchor.issue)?),
        )
    }

    fn read_group(
        &self,
        query: &str,
        key: Option<String>,
    ) -> Result<Option<IssueGroup>, ProviderError> {
        let mut statement = self.connection.prepare(query).map_err(storage_error)?;
        let mut rows = if let Some(key) = key {
            statement.query([key]).map_err(storage_error)?
        } else {
            statement.query([]).map_err(storage_error)?
        };
        rows.next()
            .map_err(storage_error)?
            .map(|row| {
                Ok(IssueGroup {
                    anchor: decode(&row.get::<_, String>(0).map_err(storage_error)?)?,
                    date: row.get(1).map_err(storage_error)?,
                })
            })
            .transpose()
    }

    /// Report unsafe fingerprints before selecting a bounded issue window.
    pub(crate) fn has_unknown_groups(&self) -> Result<bool, ProviderError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM groups WHERE anchor IS NULL)",
                [],
                |row| row.get(0),
            )
            .map_err(storage_error)
    }

    /// Read at most 225 works and 16 MiB using a stable keyset, never a journal-sized vector.
    pub(crate) fn emit(
        &self,
        upper: &str,
        lower: Option<&str>,
        after: Option<&EmissionKey>,
    ) -> Result<EmissionPage, ProviderError> {
        if let Some(after) = after {
            let exists: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM works WHERE key=?1 AND group_rank=?2 AND date=?3)",
                    params![after.key, after.rank, after.date],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            if !exists {
                return Err(invalid(
                    "Crossref emission key is not present in its verified workset",
                ));
            }
        }
        let mut statement = self.connection.prepare("SELECT w.payload,w.group_rank,w.date,w.key,length(CAST(w.payload AS BLOB)) FROM works w INDEXED BY works_emit JOIN groups g ON g.rank=w.group_rank
            WHERE (w.group_rank,w.sort_date,w.key)>(?3,?4,?5) AND g.date<=?1 AND (?2 IS NULL OR g.date>=?2)
            ORDER BY w.group_rank,w.sort_date,w.key LIMIT 226").map_err(storage_error)?;
        let sort_date = after
            .and_then(|key| key.date.replace('-', "").parse::<i64>().ok())
            .map(|value| -value)
            .unwrap_or(0);
        let mut rows = statement
            .query(params![
                upper,
                lower,
                after.map(|key| key.rank).unwrap_or(0),
                sort_date,
                after.map(|key| key.key.as_str()).unwrap_or("")
            ])
            .map_err(storage_error)?;
        let mut works = Vec::new();
        let mut bytes = 0;
        let mut next = after.cloned();
        let mut has_more = false;
        while let Some(row) = rows.next().map_err(storage_error)? {
            let payload_size: usize = row.get(4).map_err(storage_error)?;
            if works.len() == 225 || payload_size > MAXIMUM_BATCH_BYTES - bytes {
                if works.is_empty() {
                    return Err(invalid(
                        "Crossref work exceeds the local emission size limit",
                    ));
                }
                has_more = true;
                break;
            }
            let payload: String = row.get(0).map_err(storage_error)?;
            bytes += payload.len();
            works.push(decode(&payload)?);
            next = Some(EmissionKey {
                rank: row.get(1).map_err(storage_error)?,
                date: row.get(2).map_err(storage_error)?,
                key: row.get(3).map_err(storage_error)?,
            });
        }
        Ok(EmissionPage {
            works,
            after: next,
            has_more,
        })
    }

    /// Discard only this validated owned workset after completion or an unbounded replay.
    pub(crate) fn discard(self) -> Result<(), ProviderError> {
        let Self {
            connection,
            root,
            owner,
            ..
        } = self;
        connection
            .close()
            .map_err(|(_, error)| storage_error(error))?;
        remove_owned_files(&root, &owner)
    }

    /// Rebuild recognized damaged owned data without reusing a collection or emission cursor.
    pub(crate) fn recollect(self) -> Result<CrossrefCheckpoint, ProviderError> {
        let mut state = self.state.clone();
        let root = self.root.clone();
        let scope = self.owner.scope.clone();
        self.discard()?;
        state.token = new_token()?;
        state.sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("Crossref sequence overflow"))?;
        state.restart_collection();
        if state.created_from.is_none() {
            state.root_total = None;
        }
        Self::create(&root, &scope, state).map(|cache| cache.checkpoint())
    }
}

fn split_second(first: i64, last: i64) -> Result<i64, ProviderError> {
    last.checked_sub(first)
        .and_then(|span| first.checked_add(span / 2))
        .filter(|middle| *middle >= first && *middle < last)
        .ok_or_else(|| invalid("Crossref partition cannot be split safely"))
}

fn valid_sort_date(date: &str) -> bool {
    date.len() == 10
        && normalize_contract_date(date).is_some_and(|normalized| normalized.value == date)
}

fn created_second(work: &Value) -> Result<i64, ProviderError> {
    let milliseconds = work
        .pointer("/created/timestamp")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("Crossref work has no numeric creation timestamp"))?;
    let second = milliseconds.div_euclid(1_000);
    chrono::DateTime::from_timestamp(second, 0)
        .ok_or_else(|| invalid("Crossref creation timestamp is out of range"))?;
    Ok(second)
}

fn consumed_payload(mut work: Value) -> Value {
    let mut payload = serde_json::Map::new();
    for field in [
        "DOI", "PMID", "title", "volume", "issue", "page", "abstract",
    ] {
        if let Some(value) = work.get_mut(field) {
            payload.insert(field.to_string(), value.take());
        }
    }
    if let Some(doi) = payload
        .get("DOI")
        .and_then(Value::as_str)
        .and_then(normalize_contract_doi)
    {
        payload.insert("DOI".to_string(), Value::String(doi));
    }
    for field in ["published-online", "published-print", "published", "issued"] {
        if let Some(parts) = work
            .get_mut(field)
            .and_then(|date| date.get_mut("date-parts"))
        {
            payload.insert(
                field.to_string(),
                serde_json::json!({"date-parts": parts.take()}),
            );
        }
    }
    if let Some(timestamp) = work.pointer("/created/timestamp") {
        payload.insert(
            "created".to_string(),
            serde_json::json!({"timestamp": timestamp}),
        );
    }
    for (field, keys) in [
        ("author", ["given", "family"]),
        ("updated-by", ["DOI", "type"]),
    ] {
        if let Some(items) = work.get(field).and_then(Value::as_array) {
            payload.insert(
                field.to_string(),
                Value::Array(
                    items
                        .iter()
                        .map(|item| {
                            Value::Object(
                                keys.iter()
                                    .filter_map(|key| {
                                        item.get(*key)
                                            .map(|value| ((*key).to_string(), value.clone()))
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                ),
            );
        }
    }
    Value::Object(payload)
}

fn work_key(work: &Value, serialized: &str) -> String {
    work.get("DOI")
        .and_then(Value::as_str)
        .and_then(normalize_contract_doi)
        .map(|doi| format!("doi:{doi}"))
        .unwrap_or_else(|| {
            format!(
                "payload:{}",
                Sha256::digest(serialized.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        })
}

struct WorkOrder {
    date: String,
    fingerprint: String,
    anchor: Option<String>,
    year: i64,
    volume: String,
    issue: String,
}

fn crossref_order(work: &Value, key: &str) -> Result<WorkOrder, ProviderError> {
    let date = crossref_date(work)
        .map(|value| match value.len() {
            4 => format!("{value}-01-01"),
            7 => format!("{value}-01"),
            _ => value,
        })
        .unwrap_or_default();
    let anchor = crossref_work_issue_anchor(work);
    let fingerprint = anchor
        .as_ref()
        .map(|anchor| encode(&anchor.issue))
        .transpose()?
        .unwrap_or_else(|| format!("unknown:{key}"));
    let numeric_label = |field| {
        work.get(field)
            .and_then(Value::as_str)
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|value| format!("{value:020}"))
            .unwrap_or_default()
    };
    Ok(WorkOrder {
        year: date
            .get(..4)
            .and_then(|year| year.parse().ok())
            .unwrap_or(-1),
        date,
        fingerprint,
        anchor: anchor.as_ref().map(encode).transpose()?,
        volume: numeric_label("volume"),
        issue: numeric_label("issue"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn work(index: u64, created: i64) -> Value {
        json!({"DOI":format!("10.1000/{index:08}"),"title":[format!("Article {index}")],
            "published":{"date-parts":[[2026,1 + index % 12,1]]},"volume":"2","issue":format!("{}",1 + index % 12),
            "created":{"timestamp":created * 1_000}})
    }

    fn page(items: Vec<Value>, total: u64) -> CrossrefWorksPage {
        CrossrefWorksPage {
            items,
            total_results: total,
            next_cursor: Some("unchanging-cursor".to_string()),
        }
    }

    fn fixture_page(
        state: &CrossrefCheckpoint,
        count: u64,
        is_distributed: bool,
    ) -> CrossrefWorksPage {
        let first = 1_000_000_000;
        if matches!(state.phase, CrossrefPhase::Discover) {
            return page(vec![work(0, first)], count);
        }
        let CrossrefPhase::Collect {
            from,
            until,
            received,
            ..
        } = state.phase.clone()
        else {
            panic!("collection query")
        };
        let (start, end) = if is_distributed {
            (
                (from - first).max(0) as u64,
                ((until - first).max(-1) + 1) as u64,
            )
        } else if (from..=until).contains(&first) {
            (0, count)
        } else {
            (0, 0)
        };
        let start = start.min(count);
        let end = end.min(count).max(start);
        page(
            (start + received..end)
                .take(CROSSREF_ROWS)
                .map(|index| work(index, first + if is_distributed { index as i64 } else { 0 }))
                .collect(),
            end - start,
        )
    }

    fn collect_fixture(count: u64, is_distributed: bool) {
        let directory = tempfile::tempdir().unwrap();
        let initial = CrossrefCheckpoint::new(
            "1234-5679".to_string(),
            1_800_000_000,
            Some("2026-01-01".to_string()),
        )
        .unwrap();
        let mut cache = CrossrefWorkset::create(directory.path(), "catalog", initial).unwrap();
        let mut steps = 0;
        let mut cursor_pages = 0;
        while !matches!(cache.state.phase, CrossrefPhase::Ready) {
            let query = cache.state.query().unwrap();
            if let CrossrefQuery::Works {
                created_from,
                created_until,
                updated_from,
                updated_until,
                cursor,
            } = query
            {
                assert_eq!(updated_from.as_deref(), Some("2026-01-01"));
                assert_eq!(updated_until, Some(1_800_000_000));
                if cursor.is_some() {
                    assert_eq!(created_from, created_until);
                    cursor_pages += 1;
                }
            }
            let response = fixture_page(&cache.state, count, is_distributed);
            cache.accept(response).unwrap();
            steps += 1;
            assert!(steps < 2_000);
        }
        assert_eq!(cache.state.root_total, Some(count));
        assert_eq!(
            cache
                .connection
                .query_row::<u64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                .unwrap(),
            count
        );
        assert_eq!(cursor_pages == 0, is_distributed);
        let mut after = None;
        let mut keys = BTreeSet::new();
        let mut previous_issue = u64::MAX;
        loop {
            let emitted = cache.emit("9999-12-31", None, after.as_ref()).unwrap();
            assert!(emitted.works.len() <= 225);
            for work in &emitted.works {
                assert!(keys.insert(work["DOI"].as_str().unwrap().to_string()));
                let issue = work["issue"].as_str().unwrap().parse::<u64>().unwrap();
                assert!(issue <= previous_issue);
                previous_issue = issue;
            }
            if !emitted.has_more {
                break;
            }
            assert_ne!(after, emitted.after);
            after = emitted.after;
        }
        assert_eq!(keys.len() as u64, count);
    }

    #[test]
    fn large_created_partitions_have_no_ten_thousand_or_hundred_thousand_cutoff() {
        for count in [10_001, 100_001] {
            collect_fixture(count, true);
        }
    }

    #[test]
    fn dense_second_cursors_have_no_ten_thousand_or_hundred_thousand_cutoff() {
        for count in [10_001, 100_001] {
            collect_fixture(count, false);
        }
    }

    #[test]
    fn cursor_full_last_page_requires_an_observed_end_before_completing() {
        for has_unexpected_tail in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let initial =
                CrossrefCheckpoint::new("1234-5679".to_string(), 1_000_000_000, None).unwrap();
            let mut cache = CrossrefWorkset::create(directory.path(), "catalog", initial).unwrap();
            cache
                .accept(fixture_page(&cache.state, 1_125, false))
                .unwrap();
            cache
                .accept(fixture_page(&cache.state, 1_125, false))
                .unwrap();
            for _page in 0..5 {
                cache
                    .accept(fixture_page(&cache.state, 1_125, false))
                    .unwrap();
            }
            assert!(matches!(
                cache.state.phase,
                CrossrefPhase::Collect {
                    received: 1_125,
                    expected: Some(1_125),
                    ..
                }
            ));
            let confirmed = cache.checkpoint();
            drop(cache);
            let (mut cache, did_replay) =
                CrossrefWorkset::open(directory.path(), "catalog", &confirmed).unwrap();
            assert!(!did_replay);
            let tail = if has_unexpected_tail {
                vec![work(1_125, 1_000_000_000)]
            } else {
                Vec::new()
            };
            cache.accept(page(tail, 1_125)).unwrap();
            if has_unexpected_tail {
                assert!(matches!(
                    cache.state.phase,
                    CrossrefPhase::Collect {
                        received: 0,
                        retry: 1,
                        ..
                    }
                ));
            } else {
                assert!(matches!(cache.state.phase, CrossrefPhase::Ready));
            }
        }
    }

    #[test]
    fn crossref_order_utc_seconds_and_inclusive_splits_do_not_overlap() {
        let work = json!({"created":{"timestamp":1_047_289_910_000_i64,"date-time":"2003-03-10T17:51:50+08:00"}});
        assert_eq!(created_second(&work).unwrap(), 1_047_289_910);
        for (first, last) in [(0, 1), (0, 2), (-10, 5), (1_047_289_910, 1_800_000_000)] {
            let middle = split_second(first, last).unwrap();
            assert!(first <= middle && middle < last);
            assert_eq!((middle - first + 1) + (last - middle), last - first + 1);
        }
        assert!(split_second(i64::MIN, i64::MAX).is_err());
        assert!(split_second(1, 1).is_err());
    }

    #[test]
    fn crossref_order_preserves_date_precision_and_canonical_payload_identity() {
        let source = json!({"title":["No DOI"],"issued":{"date-parts":[[2026]]},"volume":"2","created":{"timestamp":0},"reference":["omit"],"resource":{"primary":{"URL":"https://private.invalid"}},"author":[{"given":"A","family":"B","affiliation":["omit"]}]});
        let payload = consumed_payload(source);
        let serialized = encode(&payload).unwrap();
        let key = work_key(&payload, &serialized);
        assert!(key.starts_with("payload:"));
        assert!(!serialized.contains("omit") && !serialized.contains("URL"));
        let order = crossref_order(&payload, &key).unwrap();
        assert_eq!(order.date, "2026-01-01");
        assert_eq!(crossref_date(&payload).as_deref(), Some("2026"));
        let mut changed = payload.clone();
        changed["title"] = json!(["Changed title"]);
        assert_ne!(work_key(&changed, &encode(&changed).unwrap()), key);
    }

    fn discovered_cache(directory: &Path) -> CrossrefWorkset {
        let state =
            CrossrefCheckpoint::new("1234-5679".to_string(), 2, Some("1970-01-01".to_string()))
                .unwrap();
        let mut cache = CrossrefWorkset::create(directory, "catalog", state).unwrap();
        cache.accept(page(vec![work(0, 1)], 1_001)).unwrap();
        cache
    }

    #[test]
    fn single_responses_within_the_page_budget_never_start_a_cursor() {
        for count in [0, 1, 224, 225] {
            let directory = tempfile::tempdir().unwrap();
            let mut cache = discovered_cache(directory.path());
            let state = cache
                .accept(page(
                    (0..count).map(|index| work(index, 1)).collect(),
                    count,
                ))
                .unwrap();
            assert!(matches!(state.phase, CrossrefPhase::Ready));
            assert_eq!(state.root_total, Some(count));
        }
    }

    #[test]
    fn partitions_over_the_page_budget_split_and_resume_dense_seconds() {
        for count in [226, 1_000, 1_001] {
            let directory = tempfile::tempdir().unwrap();
            let mut cache = discovered_cache(directory.path());
            let before_split = cache.checkpoint();
            let split = cache
                .accept(page(
                    (0..CROSSREF_ROWS as u64)
                        .map(|index| work(index, 1))
                        .collect(),
                    count,
                ))
                .unwrap();
            assert!(matches!(
                split.phase,
                CrossrefPhase::Collect {
                    from: 1,
                    until: 1,
                    cursor: None,
                    ..
                }
            ));
            drop(cache);
            let (mut cache, replayed) =
                CrossrefWorkset::open(directory.path(), "catalog", &before_split).unwrap();
            assert!(replayed);
            assert_eq!(cache.checkpoint(), split);
            assert_eq!(
                cache
                    .connection
                    .query_row::<u64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                    .unwrap(),
                0
            );
            cache
                .accept(page(
                    (0..CROSSREF_ROWS as u64)
                        .map(|index| work(index, 1))
                        .collect(),
                    count,
                ))
                .unwrap();
            assert!(
                matches!(&cache.state.phase, CrossrefPhase::Collect { cursor: Some(cursor), expected: Some(expected), received: 0, .. } if cursor == "*" && *expected == count)
            );
            cache
                .accept(page(
                    (0..CROSSREF_ROWS as u64)
                        .map(|index| work(index, 1))
                        .collect(),
                    count,
                ))
                .unwrap();
            let checkpoint: CrossrefCheckpoint =
                serde_json::from_str(&encode(&cache.checkpoint()).unwrap()).unwrap();
            drop(cache);
            let (mut cache, replayed) =
                CrossrefWorkset::open(directory.path(), "catalog", &checkpoint).unwrap();
            assert!(!replayed);
            assert_eq!(cache.checkpoint(), checkpoint);
            while let CrossrefPhase::Collect {
                from,
                until,
                received,
                ..
            } = cache.state.phase
            {
                let total = if (from..=until).contains(&1) {
                    count
                } else {
                    0
                };
                cache
                    .accept(page(
                        (received..total)
                            .take(CROSSREF_ROWS)
                            .map(|index| work(index, 1))
                            .collect(),
                        total,
                    ))
                    .unwrap();
            }
            assert!(matches!(cache.state.phase, CrossrefPhase::Ready));
            assert_eq!(cache.state.root_total, Some(count));
            assert_eq!(cache.state.frozen_at, before_split.frozen_at);
            assert_eq!(cache.state.updated_from, before_split.updated_from);
            assert_eq!(cache.state.generation, 0);
            assert_eq!(
                cache
                    .connection
                    .query_row::<u64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                    .unwrap(),
                count
            );
        }
    }

    #[test]
    fn legacy_completed_single_response_above_new_budget_is_not_recollected() {
        let directory = tempfile::tempdir().unwrap();
        let initial = CrossrefCheckpoint::new("1234-5679".to_string(), 1, None).unwrap();
        let mut cache = CrossrefWorkset::create(directory.path(), "catalog", initial).unwrap();
        cache.accept(page(vec![work(0, 1)], 500)).unwrap();
        cache
            .accept(page(
                (0..CROSSREF_ROWS as u64)
                    .map(|index| work(index, 1))
                    .collect(),
                500,
            ))
            .unwrap();
        while let CrossrefPhase::Collect { received, .. } = cache.state.phase {
            cache
                .accept(page(
                    (received..500)
                        .take(CROSSREF_ROWS)
                        .map(|index| work(index, 1))
                        .collect(),
                    500,
                ))
                .unwrap();
        }
        cache
            .connection
            .execute("UPDATE partitions SET cursor=NULL WHERE status='done'", [])
            .unwrap();
        let checkpoint = cache.checkpoint();
        drop(cache);
        let (cache, replayed) =
            CrossrefWorkset::open(directory.path(), "catalog", &checkpoint).unwrap();
        assert!(!replayed);
        assert_eq!(cache.checkpoint(), checkpoint);
        assert!(matches!(cache.state.phase, CrossrefPhase::Ready));
        assert!(cache.state.query().is_err());
        assert_eq!(cache.connection.query_row::<u64, _, _>("SELECT received FROM partitions WHERE id=1 AND status='done' AND cursor IS NULL", [], |row| row.get(0)).unwrap(), 500);
        assert_eq!(
            cache
                .connection
                .query_row::<u64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                .unwrap(),
            500
        );
        assert_eq!(
            cache.emit("9999-12-31", None, None).unwrap().works.len(),
            225
        );
    }

    #[test]
    fn empty_first_child_does_not_finish_before_a_dense_second_child() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = discovered_cache(directory.path());
        cache
            .accept(page(
                (0..CROSSREF_ROWS as u64)
                    .map(|index| work(index, 2))
                    .collect(),
                1_001,
            ))
            .unwrap();
        cache.accept(page(Vec::new(), 0)).unwrap();
        assert!(matches!(
            cache.state.phase,
            CrossrefPhase::Collect { from: 2, .. }
        ));
        cache
            .accept(page(
                (0..CROSSREF_ROWS as u64)
                    .map(|index| work(index, 2))
                    .collect(),
                1_001,
            ))
            .unwrap();
        while let CrossrefPhase::Collect { received, .. } = cache.state.phase {
            cache
                .accept(page(
                    (received..1_001)
                        .take(CROSSREF_ROWS)
                        .map(|index| work(index, 2))
                        .collect(),
                    1_001,
                ))
                .unwrap();
        }
        assert!(matches!(cache.state.phase, CrossrefPhase::Ready));
        assert_eq!(cache.state.root_total, Some(1_001));
    }

    #[test]
    fn cursor_conflicting_payload_retries_only_its_leaf_and_retains_validated_siblings() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = discovered_cache(directory.path());
        cache
            .accept(page(
                (0..CROSSREF_ROWS as u64)
                    .map(|index| work(index, 2))
                    .collect(),
                1_102,
            ))
            .unwrap();
        cache
            .accept(page(vec![work(5_000, 1), work(5_001, 1)], 2))
            .unwrap();
        cache
            .accept(page(
                (0..CROSSREF_ROWS as u64)
                    .map(|index| work(index, 2))
                    .collect(),
                1_100,
            ))
            .unwrap();
        cache
            .accept(page((0..225).map(|index| work(index, 2)).collect(), 1_100))
            .unwrap();
        let mut duplicate = work(0, 2);
        duplicate["title"] = json!(["Changed while paginating"]);
        let mut repeated = (225..449).map(|index| work(index, 2)).collect::<Vec<_>>();
        repeated.push(duplicate);
        let retry = cache.accept(page(repeated, 1_100)).unwrap();
        assert!(matches!(
            retry.phase,
            CrossrefPhase::Collect {
                from: 2,
                received: 0,
                retry: 1,
                ..
            }
        ));
        assert_eq!(retry.generation, 0);
        assert_eq!(retry.root_total, Some(1_102));
        assert_eq!(
            cache
                .connection
                .query_row::<i64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                .unwrap(),
            2
        );
        drop(cache);
        let (cache, replayed) = CrossrefWorkset::open(directory.path(), "catalog", &retry).unwrap();
        assert!(!replayed);
        assert_eq!(cache.state, retry);
    }

    #[test]
    fn sealed_selection_survives_a_lost_core_ack() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = discovered_cache(directory.path());
        cache.accept(page(vec![work(0, 1)], 1)).unwrap();
        let ready = cache.checkpoint();
        let group = cache.first_group().unwrap().unwrap();
        let sealed = cache
            .seal_selection(group.date.clone(), None, Some(group.anchor.clone()))
            .unwrap();
        drop(cache);
        let (cache, replayed) = CrossrefWorkset::open(directory.path(), "catalog", &ready).unwrap();
        assert!(replayed);
        assert_eq!(cache.state, sealed);
        assert_eq!(cache.state.candidate, Some(group.anchor));
    }

    #[test]
    fn crossref_order_leap_day_and_year_boundaries_use_utc_milliseconds() {
        for (milliseconds, expected) in [
            (1_709_164_799_999_i64, "2024-02-28T23:59:59"),
            (1_709_164_800_000, "2024-02-29T00:00:00"),
            (1_735_689_599_999, "2024-12-31T23:59:59"),
            (1_735_689_600_000, "2025-01-01T00:00:00"),
        ] {
            let second = created_second(&json!({"created":{"timestamp":milliseconds}})).unwrap();
            assert_eq!(
                chrono::DateTime::from_timestamp(second, 0)
                    .unwrap()
                    .format("%Y-%m-%dT%H:%M:%S")
                    .to_string(),
                expected
            );
        }
    }

    #[test]
    fn reparse_or_symlink_roots_are_rejected_without_touching_the_destination() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        let link = directory.path().join("link");
        fs::create_dir(&outside).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(output.status.success(), "junction fixture must be created");
        }
        let state = CrossrefCheckpoint::new("1234-5679".to_string(), 2, None).unwrap();
        assert!(CrossrefWorkset::create(&link, "catalog", state).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        fs::remove_file(&link).unwrap();
    }

    #[test]
    fn duplicate_or_truncated_leaf_retries_once_and_never_becomes_ready() {
        for is_duplicate in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut cache = discovered_cache(directory.path());
            let response = if is_duplicate {
                page(vec![work(1, 1), work(1, 1)], 2)
            } else {
                page(vec![work(1, 1)], 2)
            };
            let retry = cache.accept(response.clone()).unwrap();
            assert!(matches!(
                retry.phase,
                CrossrefPhase::Collect {
                    retry: 1,
                    expected: Some(2),
                    ..
                }
            ));
            drop(cache);
            let (mut restored, has_replayed) =
                CrossrefWorkset::open(directory.path(), "catalog", &retry).unwrap();
            assert!(!has_replayed);
            assert!(restored.accept(response).is_err());
            assert_eq!(restored.state, retry);
            assert_eq!(
                restored
                    .connection
                    .query_row::<i64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                    .unwrap(),
                0
            );
        }
    }

    #[test]
    fn parent_count_mismatch_has_one_persisted_generation_retry() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = discovered_cache(directory.path());
        for generation in 0..=1 {
            cache
                .accept(page(
                    (0..CROSSREF_ROWS as u64)
                        .map(|index| work(index, 1))
                        .collect(),
                    449,
                ))
                .unwrap();
            cache
                .accept(page((0..225).map(|index| work(index, 1)).collect(), 225))
                .unwrap();
            let response = page((225..450).map(|index| work(index, 2)).collect(), 225);
            if generation == 0 {
                let retry = cache.accept(response).unwrap();
                assert_eq!(retry.generation, 1);
                assert_eq!(retry.root_total, None);
                drop(cache);
                (cache, _) = CrossrefWorkset::open(directory.path(), "catalog", &retry).unwrap();
            } else {
                assert!(cache
                    .accept(response)
                    .unwrap_err()
                    .to_string()
                    .contains("journal count drift"));
            }
        }
    }

    #[test]
    fn cache_ahead_of_ack_replays_exact_step_and_does_not_double_count() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = discovered_cache(directory.path());
        let confirmed = cache.checkpoint();
        let next = cache.accept(page(vec![work(0, 1)], 1)).unwrap();
        drop(cache);
        for _ in 0..2 {
            let (cache, replayed) =
                CrossrefWorkset::open(directory.path(), "catalog", &confirmed).unwrap();
            assert!(replayed);
            assert_eq!(cache.checkpoint(), next);
            assert_eq!(
                cache
                    .connection
                    .query_row::<u64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                    .unwrap(),
                1
            );
        }
        let (cache, replayed) = CrossrefWorkset::open(directory.path(), "catalog", &next).unwrap();
        assert!(!replayed);
        let first = cache.emit("9999-12-31", None, None).unwrap();
        let again = cache.emit("9999-12-31", None, None).unwrap();
        assert_eq!(first.works, again.works);
    }

    #[test]
    fn missing_or_damaged_owned_cache_recollects_with_the_original_bounds() {
        for is_damaged in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut cache = discovered_cache(directory.path());
            let checkpoint = cache.accept(page(vec![work(0, 1)], 1)).unwrap();
            let path = cache.root.join(format!("{}.sqlite", checkpoint.token));
            drop(cache);
            if is_damaged {
                fs::write(&path, b"damaged database").unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
            let (cache, replayed) =
                CrossrefWorkset::open(directory.path(), "catalog", &checkpoint).unwrap();
            assert!(replayed);
            assert_ne!(cache.state.token, checkpoint.token);
            assert_eq!(cache.state.created_from, Some(1));
            assert_eq!(cache.state.frozen_at, 2);
            assert_eq!(cache.state.updated_from, checkpoint.updated_from);
            assert!(matches!(
                cache.state.phase,
                CrossrefPhase::Collect {
                    cursor: None,
                    received: 0,
                    ..
                }
            ));
        }
    }

    #[test]
    fn foreign_cache_tokens_contexts_and_paths_are_rejected_without_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let cache = discovered_cache(directory.path());
        let state = cache.checkpoint();
        let database = cache.root.join(format!("{}.sqlite", state.token));
        drop(cache);
        assert!(CrossrefWorkset::open(directory.path(), "another catalog", &state).is_err());
        assert!(database.exists());
        let mut malformed = state.clone();
        malformed.token = "../outside".to_string();
        assert!(CrossrefWorkset::open(directory.path(), "catalog", &malformed).is_err());
        assert!(database.exists());
        let connection = Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "application_id", 123)
            .unwrap();
        drop(connection);
        assert!(CrossrefWorkset::open(directory.path(), "catalog", &state).is_err());
        assert!(database.exists());
        fs::remove_file(directory.path().join(format!("{}.json", state.token))).unwrap();
        assert!(CrossrefWorkset::open(directory.path(), "catalog", &state).is_err());
        assert!(database.exists());
    }

    #[test]
    fn workset_capacity_is_enforced_and_a_full_disk_does_not_advance_the_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache = discovered_cache(directory.path());
        assert_eq!(
            cache
                .connection
                .pragma_query_value::<u64, _>(None, "max_page_count", |row| row.get(0))
                .unwrap(),
            MAXIMUM_PAGES
        );
        assert_eq!(
            cache
                .connection
                .pragma_query_value::<i64, _>(None, "cache_size", |row| row.get(0))
                .unwrap(),
            -4096
        );
        let count = cache
            .connection
            .pragma_query_value::<u64, _>(None, "page_count", |row| row.get(0))
            .unwrap();
        cache
            .connection
            .pragma_update(None, "max_page_count", count)
            .unwrap();
        let before = cache.checkpoint();
        let mut large = work(0, 1);
        large["abstract"] = Value::String("x".repeat(100_000));
        assert!(cache.accept(page(vec![large], 1)).is_err());
        assert_eq!(cache.checkpoint(), before);
        assert_eq!(
            cache
                .connection
                .query_row::<i64, _, _>("SELECT count(*) FROM works", [], |row| row.get(0))
                .unwrap(),
            0
        );
    }
}
