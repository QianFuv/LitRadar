//! Offline, manifest-bound author repairs without content migrations or notifications.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, types::ValueRef, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::article_authors::decode_article_author_names;
use crate::index_maintenance::{
    acquire_maintenance_marker, ensure_target_inactive, interrupted_index_maintenance_state,
    maintenance_paths, validate_fts_membership, validate_sqlite_integrity,
};
use crate::search_text::{prepare_search_text, uses_simple_search};
use crate::StorageConfig;

type RepairResult<T> = Result<T, Box<dyn Error>>;
const DATABASE_NAME: &str = "chinese_journals.sqlite";

/// An explicitly reviewed replacement with exact original JSON and textual evidence.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorCorrection {
    /// Decimal identifier encoded as text to preserve all 64 bits in JSON consumers.
    pub article_id: String,
    /// Exact stored JSON, including its original encoding shape.
    pub before: String,
    /// Replacement JSON in the same author encoding shape.
    pub after: String,
    /// Human-readable classification or source supporting this replacement.
    pub reason: String,
}

/// A consistent database snapshot bound to a reviewed set of author corrections.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorRepairPlan {
    /// Manifest format version, independent of the content schema.
    pub format_version: u32,
    /// Content version; repairing never changes this version.
    pub schema_version: i64,
    /// Number of canonical articles, including unaffected articles.
    pub article_count: u64,
    /// Digest of schema and every authoritative non-author value.
    pub protected_sha256: String,
    /// Digest of every original article identifier and author JSON value.
    pub authors_before_sha256: String,
    /// Expected digest of every identifier and author value after repair.
    pub authors_after_sha256: String,
    /// Count and digest of search token instances belonging to unchanged articles.
    pub unchanged_search_sha256: String,
    /// Ordered replacements that still need applying to this particular snapshot.
    pub corrections: Vec<AuthorCorrection>,
}

/// Read one consistent snapshot and bind pending corrections without changing the database.
/// Already-applied corrections are omitted, making a repeated dry run a no-op.
pub fn plan_cnki_author_repair(
    config: &StorageConfig,
    corrections: &[AuthorCorrection],
) -> RepairResult<AuthorRepairPlan> {
    let mut connection = open_database(&config.index_dir().join(DATABASE_NAME), false)?;
    let transaction = connection.transaction()?;
    let schema_version = validate_database(&transaction)?;
    let mut pending = Vec::new();
    let mut identifiers = BTreeSet::new();
    for correction in corrections {
        let identifier = validate_correction(correction)?;
        if !identifiers.insert(identifier) {
            return Err("duplicate article correction".into());
        }
        let actual: String = transaction.query_row(
            "SELECT authors_json FROM articles WHERE article_id=?1",
            [identifier],
            |row| row.get(0),
        )?;
        if actual == correction.after {
            continue;
        }
        if actual != correction.before {
            return Err(format!("stale author correction for article {identifier}").into());
        }
        pending.push(correction.clone());
    }
    pending.sort_by_key(|correction| correction.article_id.parse::<i64>().unwrap());
    let replacements = replacement_map(&pending)?;
    let plan = AuthorRepairPlan {
        format_version: 1,
        schema_version,
        article_count: transaction
            .query_row("SELECT count(*) FROM articles", [], |row| row.get(0))?,
        protected_sha256: protected_digest(&transaction)?,
        authors_before_sha256: authors_digest(&transaction, &BTreeMap::new())?,
        authors_after_sha256: authors_digest(&transaction, &replacements)?,
        unchanged_search_sha256: unchanged_search_digest(&transaction, &pending)?,
        corrections: pending,
    };
    transaction.commit()?;
    Ok(plan)
}

/// Apply a bound plan under the shared maintenance guard and a SQLite write transaction.
/// A new, verified SQLite backup is required before the first mutation. Errors roll back;
/// uncertain commit or rollback outcomes retain the maintenance marker for operator recovery.
pub fn apply_cnki_author_repair(
    config: &StorageConfig,
    plan: &AuthorRepairPlan,
    backup_path: &Path,
) -> RepairResult<()> {
    validate_backup_path(config, backup_path)?;
    if interrupted_index_maintenance_state(config)?.is_some() {
        return Err("interrupted index maintenance requires recovery first".into());
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    ensure_target_inactive(config, now)?;
    let paths = maintenance_paths(config)?;
    acquire_maintenance_marker(&paths, now)?;
    let mut should_retain_marker = false;
    let result = (|| {
        ensure_target_inactive(config, now)?;
        let database_path = config.index_dir().join(DATABASE_NAME);
        let mut connection = open_database(&database_path, true)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let operation = (|| {
            validate_plan_state(&transaction, plan, false)?;
            create_backup(&database_path, backup_path)?;
            let backup = open_database(backup_path, false)?;
            validate_plan_state(&backup, plan, false)?;
            let uses_simple = uses_simple_search(&transaction)?;
            for correction in &plan.corrections {
                let identifier = validate_correction(correction)?;
                let changed = transaction.execute(
                    "UPDATE articles SET authors_json=?1 WHERE article_id=?2 AND authors_json=?3",
                    params![correction.after, identifier, correction.before],
                )?;
                if changed != 1 {
                    return Err("author changed after repair planning".into());
                }
                replace_search_row(&transaction, identifier, uses_simple)?;
            }
            validate_plan_state(&transaction, plan, true)?;
            validate_search_projection(&transaction, plan)?;
            transaction.execute(
                "INSERT INTO article_search(article_search) VALUES('integrity-check')",
                [],
            )?;
            Ok::<_, Box<dyn Error>>(())
        })();
        if let Err(error) = operation {
            if let Err(rollback_error) = transaction.rollback() {
                should_retain_marker = true;
                return Err(rollback_error.into());
            }
            return Err(error);
        }
        should_retain_marker = true;
        transaction.commit()?;
        verify_cnki_author_repair(config, plan, backup_path)?;
        Ok(())
    })();
    if result.is_ok() || !should_retain_marker {
        fs::remove_file(&paths.marker)?;
    }
    result
}

/// Verify persisted repaired authors and all protected data against the retained backup.
pub fn verify_cnki_author_repair(
    config: &StorageConfig,
    plan: &AuthorRepairPlan,
    backup_path: &Path,
) -> RepairResult<()> {
    let mut connection = open_database(&config.index_dir().join(DATABASE_NAME), false)?;
    let transaction = connection.transaction()?;
    validate_plan_state(&transaction, plan, true)?;
    validate_search_projection(&transaction, plan)?;
    let backup = open_database(backup_path, false)?;
    validate_plan_state(&backup, plan, false)?;
    for correction in &plan.corrections {
        let actual: String = transaction.query_row(
            "SELECT authors_json FROM articles WHERE article_id=?1",
            [validate_correction(correction)?],
            |row| row.get(0),
        )?;
        if actual != correction.after {
            return Err("persisted author correction differs from the plan".into());
        }
    }
    transaction.commit()?;
    Ok(())
}

fn open_database(path: &Path, writable: bool) -> RepairResult<Connection> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("author repair requires a regular existing database".into());
    }
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let connection = Connection::open_with_flags(path, flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    crate::sqlite::load_index_tokenizer(&connection)?;
    Ok(connection)
}

fn validate_database(connection: &Connection) -> RepairResult<i64> {
    let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !matches!(version, 7 | 9) {
        return Err("author repair supports content schema v7 and v9 only".into());
    }
    crate::index_schema::validate_index_schema_structure(connection, version)?;
    validate_sqlite_integrity(connection, DATABASE_NAME)?;
    validate_fts_membership(connection, DATABASE_NAME)?;
    Ok(version)
}

fn validate_correction(correction: &AuthorCorrection) -> RepairResult<i64> {
    let identifier: i64 = correction.article_id.parse()?;
    if identifier.to_string() != correction.article_id || correction.reason.trim().is_empty() {
        return Err("correction requires a canonical article identifier and reason".into());
    }
    let before: serde_json::Value = serde_json::from_str(&correction.before)?;
    let after: serde_json::Value = serde_json::from_str(&correction.after)?;
    let before_names = decode_article_author_names(&correction.before)?;
    let after_names = decode_article_author_names(&correction.after)?;
    if after_names.iter().any(|name| name.trim().is_empty()) {
        return Err("empty author names must be omitted from the array".into());
    }
    let before_items = before.as_array().ok_or("authors must be an array")?;
    let after_items = after.as_array().ok_or("authors must be an array")?;
    if !before_names.is_empty()
        && !after_names.is_empty()
        && before_items[0].is_string() != after_items[0].is_string()
    {
        return Err("repair must preserve the author JSON encoding shape".into());
    }
    for item in before_items.iter().chain(after_items) {
        if let Some(object) = item.as_object() {
            if object.len() != 1 || !object.contains_key("display_name") {
                return Err("author objects contain unsupported additional metadata".into());
            }
        }
    }
    Ok(identifier)
}

fn replacement_map(corrections: &[AuthorCorrection]) -> RepairResult<BTreeMap<i64, &str>> {
    let mut replacements = BTreeMap::new();
    for correction in corrections {
        if replacements
            .insert(validate_correction(correction)?, correction.after.as_str())
            .is_some()
        {
            return Err("duplicate article correction".into());
        }
    }
    Ok(replacements)
}

fn validate_plan_state(
    connection: &Connection,
    plan: &AuthorRepairPlan,
    repaired: bool,
) -> RepairResult<()> {
    if plan.format_version != 1 || validate_database(connection)? != plan.schema_version {
        return Err("repair plan format or schema does not match".into());
    }
    let replacements = replacement_map(&plan.corrections)?;
    let count: u64 = connection.query_row("SELECT count(*) FROM articles", [], |row| row.get(0))?;
    let expected = if repaired {
        &plan.authors_after_sha256
    } else {
        &plan.authors_before_sha256
    };
    if count != plan.article_count
        || protected_digest(connection)? != plan.protected_sha256
        || authors_digest(connection, &BTreeMap::new())? != *expected
    {
        return Err("database snapshot differs from the bound author repair plan".into());
    }
    if !repaired && authors_digest(connection, &replacements)? != plan.authors_after_sha256 {
        return Err("repair plan replacement digest is inconsistent".into());
    }
    Ok(())
}

fn validate_backup_path(config: &StorageConfig, path: &Path) -> RepairResult<()> {
    let parent = path
        .parent()
        .ok_or("backup requires an existing parent directory")?
        .canonicalize()?;
    let filename = path.file_name().ok_or("backup requires a filename")?;
    let resolved = parent.join(filename);
    for directory in [
        config.index_dir(),
        config.index_control_dir(),
        config.meta_dir(),
    ] {
        if directory.exists() && resolved.starts_with(directory.canonicalize()?) {
            return Err("backup must be outside managed database directories".into());
        }
    }
    let auth = config.auth_db_path();
    if let Some(auth_parent) = auth.parent() {
        if parent == auth_parent.canonicalize()? {
            let auth_name = auth
                .file_name()
                .ok_or("invalid auth path")?
                .to_string_lossy();
            let filename = filename.to_string_lossy();
            let matches_name = |expected: &str| {
                if cfg!(windows) {
                    filename.eq_ignore_ascii_case(expected)
                } else {
                    filename == expected
                }
            };
            if matches_name(&auth_name)
                || ["-wal", "-shm", "-journal"]
                    .iter()
                    .any(|suffix| matches_name(&format!("{auth_name}{suffix}")))
            {
                return Err("backup must not overwrite a live database or SQLite sidecar".into());
            }
        }
    }
    Ok(())
}

fn validate_search_projection(
    connection: &Connection,
    plan: &AuthorRepairPlan,
) -> RepairResult<()> {
    let uses_simple = uses_simple_search(connection)?;
    let tokenizer = if uses_simple {
        "simple 0"
    } else {
        "unicode61 remove_diacritics 2"
    };
    connection.execute_batch(&format!("CREATE TEMP TABLE repair_ids(article_id INTEGER PRIMARY KEY);
        CREATE VIRTUAL TABLE temp.repair_expected USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,content='',contentless_delete=1,tokenize='{tokenizer}');
        CREATE VIRTUAL TABLE temp.repair_actual_terms USING fts5vocab(main,article_search,instance);
        CREATE VIRTUAL TABLE temp.repair_expected_terms USING fts5vocab(temp,repair_expected,instance);"))?;
    let result = (|| {
        let mut insert_id = connection.prepare("INSERT INTO temp.repair_ids VALUES(?1)")?;
        for correction in &plan.corrections {
            insert_id.execute([validate_correction(correction)?])?;
        }
        let mut source = connection.prepare("SELECT a.article_id,a.title,a.abstract_text,a.doi,a.pmid,a.authors_json,j.title FROM articles a JOIN temp.repair_ids r ON a.article_id=r.article_id JOIN journals j ON j.journal_id=a.journal_id")?;
        let mut rows = source.query([])?;
        let mut insert = connection.prepare("INSERT INTO temp.repair_expected(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title) VALUES(?1,?1,?2,?3,?4,?5,?6,?7)")?;
        while let Some(row) = rows.next()? {
            let authors = decode_article_author_names(&row.get::<_, String>(5)?)?.join("; ");
            insert.execute(params![
                row.get::<_, i64>(0)?,
                prepare_search_text(row.get_ref(1)?.as_str()?, uses_simple),
                prepare_search_text(
                    row.get::<_, Option<String>>(2)?
                        .as_deref()
                        .unwrap_or_default(),
                    uses_simple
                ),
                prepare_search_text(
                    row.get::<_, Option<String>>(3)?
                        .as_deref()
                        .unwrap_or_default(),
                    uses_simple
                ),
                prepare_search_text(
                    row.get::<_, Option<String>>(4)?
                        .as_deref()
                        .unwrap_or_default(),
                    uses_simple
                ),
                prepare_search_text(&authors, uses_simple),
                prepare_search_text(row.get_ref(6)?.as_str()?, uses_simple)
            ])?;
        }
        let (unchanged, actual) = partitioned_search_digest(connection)?;
        if unchanged != plan.unchanged_search_sha256 {
            return Err("search tokens of unaffected articles changed".into());
        }
        let expected = search_terms_digest(
            connection,
            "SELECT term,doc,col,offset FROM temp.repair_expected_terms",
        )?;
        if actual != expected {
            return Err("repaired search tokens differ from canonical article text".into());
        }
        Ok::<_, Box<dyn Error>>(())
    })();
    connection.execute_batch("DROP TABLE temp.repair_actual_terms; DROP TABLE temp.repair_expected_terms; DROP TABLE temp.repair_expected; DROP TABLE temp.repair_ids;")?;
    result
}

fn unchanged_search_digest(
    connection: &Connection,
    corrections: &[AuthorCorrection],
) -> RepairResult<String> {
    connection.execute_batch("CREATE TEMP TABLE repair_ids(article_id INTEGER PRIMARY KEY); CREATE VIRTUAL TABLE temp.repair_actual_terms USING fts5vocab(main,article_search,instance);")?;
    let result = (|| {
        let mut insert = connection.prepare("INSERT INTO temp.repair_ids VALUES(?1)")?;
        for correction in corrections {
            insert.execute([validate_correction(correction)?])?;
        }
        Ok::<_, Box<dyn Error>>(partitioned_search_digest(connection)?.0)
    })();
    connection.execute_batch("DROP TABLE temp.repair_actual_terms; DROP TABLE temp.repair_ids")?;
    result
}

fn partitioned_search_digest(connection: &Connection) -> RepairResult<(String, (u64, [u8; 32]))> {
    let mut digests = [(0u64, [0u8; 32]); 2];
    let mut statement = connection.prepare("SELECT term,doc,col,offset,doc IN (SELECT article_id FROM temp.repair_ids) FROM temp.repair_actual_terms")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let partition = usize::from(row.get::<_, bool>(4)?);
        let mut hasher = Sha256::new();
        for column in 0..4 {
            hash_value(&mut hasher, row.get_ref(column)?);
        }
        for (target, byte) in digests[partition].1.iter_mut().zip(hasher.finalize()) {
            *target ^= byte;
        }
        digests[partition].0 += 1;
    }
    Ok((
        format!("{}:{}", digests[0].0, hex::encode(digests[0].1)),
        digests[1],
    ))
}

fn search_terms_digest(connection: &Connection, query: &str) -> RepairResult<(u64, [u8; 32])> {
    let mut combined = [0u8; 32];
    let mut count = 0u64;
    let mut statement = connection.prepare(query)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let mut hasher = Sha256::new();
        for column in 0..4 {
            hash_value(&mut hasher, row.get_ref(column)?);
        }
        for (target, byte) in combined.iter_mut().zip(hasher.finalize()) {
            *target ^= byte;
        }
        count += 1;
    }
    Ok((count, combined))
}

fn create_backup(source: &Path, destination: &Path) -> RepairResult<()> {
    let reservation = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    reservation.sync_all()?;
    drop(reservation);
    let source = open_database(source, false)?;
    let mut target = Connection::open_with_flags(destination, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
    backup.run_to_completion(1024, Duration::from_millis(1), None)?;
    drop(backup);
    crate::sqlite::load_index_tokenizer(&target)?;
    target.execute_batch("PRAGMA journal_mode=DELETE")?;
    validate_database(&target)?;
    drop(target);
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(destination)?
        .sync_all()?;
    Ok(())
}

fn replace_search_row(
    connection: &Connection,
    identifier: i64,
    uses_simple: bool,
) -> RepairResult<()> {
    let (title, abstract_text, doi, pmid, authors, journal): (String, Option<String>, Option<String>, Option<String>, String, String) = connection.query_row(
        "SELECT a.title,a.abstract_text,a.doi,a.pmid,a.authors_json,j.title FROM articles a JOIN journals j ON a.journal_id=j.journal_id WHERE a.article_id=?1",
        [identifier], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?;
    let authors = decode_article_author_names(&authors)?.join("; ");
    connection.execute("DELETE FROM article_search WHERE rowid=?1", [identifier])?;
    connection.execute(
        "INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title) VALUES(?1,?1,?2,?3,?4,?5,?6,?7)",
        params![identifier,prepare_search_text(&title,uses_simple),prepare_search_text(abstract_text.as_deref().unwrap_or_default(),uses_simple),prepare_search_text(doi.as_deref().unwrap_or_default(),uses_simple),prepare_search_text(pmid.as_deref().unwrap_or_default(),uses_simple),prepare_search_text(&authors,uses_simple),prepare_search_text(&journal,uses_simple)],
    )?;
    Ok(())
}

fn hash_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    fn append(hasher: &mut Sha256, tag: u8, bytes: &[u8]) {
        hasher.update([tag]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    match value {
        ValueRef::Null => append(hasher, 0, &[]),
        ValueRef::Integer(value) => append(hasher, 1, &value.to_le_bytes()),
        ValueRef::Real(value) => append(hasher, 2, &value.to_bits().to_le_bytes()),
        ValueRef::Text(value) => append(hasher, 3, value),
        ValueRef::Blob(value) => append(hasher, 4, value),
    }
}

fn authors_digest(
    connection: &Connection,
    replacements: &BTreeMap<i64, &str>,
) -> RepairResult<String> {
    let mut hasher = Sha256::new();
    let mut statement =
        connection.prepare("SELECT article_id,authors_json FROM articles ORDER BY article_id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let identifier: i64 = row.get(0)?;
        hash_value(&mut hasher, row.get_ref(0)?);
        hash_value(
            &mut hasher,
            replacements
                .get(&identifier)
                .map_or(row.get_ref(1)?, |value| ValueRef::Text(value.as_bytes())),
        );
    }
    Ok(hex::encode(hasher.finalize()))
}

fn protected_digest(connection: &Connection) -> RepairResult<String> {
    let mut hasher = Sha256::new();
    let mut schema = connection
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")?;
    let mut rows = schema.query([])?;
    while let Some(row) = rows.next()? {
        for column in 0..4 {
            hash_value(&mut hasher, row.get_ref(column)?);
        }
    }
    let mut statement = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT GLOB 'article_search*' ORDER BY name")?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for table in tables {
        hash_value(&mut hasher, ValueRef::Text(table.as_bytes()));
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let mut info = connection.prepare(&format!("PRAGMA table_info({quoted})"))?;
        let columns = info
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let selected = columns
            .iter()
            .filter(|(name, _)| table != "articles" || name != "authors_json")
            .map(|(name, _)| format!("\"{}\"", name.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        let mut keys = columns
            .iter()
            .filter(|(_, order)| *order > 0)
            .collect::<Vec<_>>();
        keys.sort_by_key(|(_, order)| *order);
        let order = if keys.is_empty() {
            "rowid".to_string()
        } else {
            keys.iter()
                .map(|(name, _)| format!("\"{}\"", name.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut data = connection.prepare(&format!(
            "SELECT {} FROM {quoted} ORDER BY {order}",
            selected.join(",")
        ))?;
        let mut rows = data.query([])?;
        while let Some(row) = rows.next()? {
            hasher.update([255]);
            for column in 0..selected.len() {
                hash_value(&mut hasher, row.get_ref(column)?);
            }
        }
    }
    Ok(hex::encode(hasher.finalize()))
}
