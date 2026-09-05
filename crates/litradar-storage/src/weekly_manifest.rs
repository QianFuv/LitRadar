//! Shared seven-day manifest discovery for weekly queries and manual delivery.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::params_from_iter;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::{open_sqlite_connection, IndexRepositoryError, StorageConfig};

const MANIFEST_CACHE_CAPACITY: usize = 64;
const MANIFEST_CACHE_ARTICLE_LIMIT: usize = 1_000_000;
const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(60);

/// Non-content diagnostics for bounded manifest cache verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeeklyManifestCacheStats {
    /// Retained parsed publications, including valid empty publications.
    pub entries: usize,
    /// Retained notifiable article identifiers.
    pub article_ids: usize,
    /// Reads that required an uncached parse attempt, including failed attempts.
    pub parse_attempts: u64,
}

/// Thread-safe parsed publication cache with entry, identifier, and lifetime bounds.
#[derive(Default)]
pub struct WeeklyManifestCache {
    state: Mutex<ManifestCacheState>,
}

#[derive(Default)]
struct ManifestCacheState {
    entries: HashMap<PathBuf, ManifestCacheEntry>,
    article_ids: usize,
    parse_attempts: u64,
    access_sequence: u64,
}

struct ManifestCacheEntry {
    fingerprint: ManifestFingerprint,
    manifest: Option<Arc<WeeklyManifest>>,
    loaded_at: Instant,
    last_access: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ManifestFingerprint {
    length: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}

impl ManifestCacheState {
    fn remove(&mut self, path: &Path) {
        if let Some(entry) = self.entries.remove(path) {
            self.article_ids -= entry
                .manifest
                .as_ref()
                .map_or(0, |value| value.article_ids.len());
        }
    }

    fn prune(&mut self, now: Instant) {
        let expired = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                now.saturating_duration_since(entry.loaded_at) >= MANIFEST_CACHE_TTL
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in expired {
            self.remove(&path);
        }
    }
}

impl WeeklyManifestCache {
    /// Return retained entry and identifier counts plus cumulative parse attempts.
    pub fn stats(&self) -> WeeklyManifestCacheStats {
        let mut state = self
            .state
            .lock()
            .expect("manifest cache should not be poisoned");
        state.prune(Instant::now());
        WeeklyManifestCacheStats {
            entries: state.entries.len(),
            article_ids: state.article_ids,
            parse_attempts: state.parse_attempts,
        }
    }

    fn read(&self, path: &Path) -> Result<Option<WeeklyManifest>, IndexRepositoryError> {
        self.read_at(path, Instant::now())
    }

    fn read_at(
        &self,
        path: &Path,
        now: Instant,
    ) -> Result<Option<WeeklyManifest>, IndexRepositoryError> {
        let key = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                let mut state = self
                    .state
                    .lock()
                    .expect("manifest cache should not be poisoned");
                state.entries.clear();
                state.article_ids = 0;
                return Err(error.into());
            }
        };
        let fingerprint = match manifest_fingerprint(&key) {
            Ok(value) => value,
            Err(error) => {
                self.state
                    .lock()
                    .expect("manifest cache should not be poisoned")
                    .remove(&key);
                return Err(error);
            }
        };
        let cached = {
            let mut state = self
                .state
                .lock()
                .expect("manifest cache should not be poisoned");
            state.prune(now);
            state.access_sequence = state.access_sequence.wrapping_add(1);
            let sequence = state.access_sequence;
            if let Some(entry) = state
                .entries
                .get_mut(&key)
                .filter(|entry| entry.fingerprint == fingerprint)
            {
                entry.last_access = sequence;
                Some(entry.manifest.clone())
            } else {
                state.remove(&key);
                state.parse_attempts += 1;
                None
            }
        };
        if let Some(manifest) = cached {
            return Ok(manifest.as_deref().cloned());
        }
        let parsed = parse_weekly_manifest(read_weekly_manifest_payload(&key)?);
        let article_count = parsed
            .as_ref()
            .map_or(0, |manifest| manifest.article_ids.len());
        if article_count > MANIFEST_CACHE_ARTICLE_LIMIT
            || manifest_fingerprint(&key).ok().as_ref() != Some(&fingerprint)
        {
            return Ok(parsed);
        }
        let manifest = parsed.map(Arc::new);
        {
            let mut state = self
                .state
                .lock()
                .expect("manifest cache should not be poisoned");
            state.prune(now);
            state.remove(&key);
            while state.entries.len() >= MANIFEST_CACHE_CAPACITY
                || state.article_ids + article_count > MANIFEST_CACHE_ARTICLE_LIMIT
            {
                let oldest = state
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_access)
                    .map(|(path, _)| path.clone());
                if let Some(oldest) = oldest {
                    state.remove(&oldest);
                } else {
                    break;
                }
            }
            state.access_sequence = state.access_sequence.wrapping_add(1);
            let last_access = state.access_sequence;
            state.article_ids += article_count;
            state.entries.insert(
                key,
                ManifestCacheEntry {
                    fingerprint,
                    manifest: manifest.clone(),
                    loaded_at: now,
                    last_access,
                },
            );
        }
        Ok(manifest.as_deref().cloned())
    }
}

fn manifest_fingerprint(path: &Path) -> Result<ManifestFingerprint, IndexRepositoryError> {
    let metadata = fs::metadata(path)?;
    Ok(ManifestFingerprint {
        length: metadata.len(),
        modified: metadata.modified()?,
        created: metadata.created().ok(),
    })
}

/// Parsed notifiable article membership and its original delivery identity.
#[derive(Debug, Clone, PartialEq)]
pub struct WeeklyManifest {
    /// Canonical index database filename.
    pub db_name: String,
    /// Original source run identifier when supplied.
    pub run_id: Option<String>,
    /// Source generation time used for window membership.
    pub generated_at: DateTime<Utc>,
    /// Deduplicated notifiable identifiers in source order.
    pub article_ids: Vec<i64>,
}

/// Return the inclusive start of a seven-day UTC manifest window.
pub fn weekly_window_start(window_end: DateTime<Utc>) -> DateTime<Utc> {
    let window_delta = TimeDelta::try_days(7).expect("seven-day duration should be valid");
    window_end
        .checked_sub_signed(window_delta)
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Read current and immutable history manifests within one fixed window.
///
/// # Arguments
///
/// * `config` - Repository storage paths.
/// * `window_start` - Inclusive earliest generation time.
/// * `window_end` - Inclusive latest generation time.
///
/// # Returns
///
/// Source snapshots ordered by generation time, with duplicate publications removed.
pub fn load_weekly_manifests(
    config: &StorageConfig,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    load_weekly_manifests_inner(config, window_start, window_end, None)
}

/// Read a fixed weekly window while reusing unchanged parsed publications.
///
/// # Arguments
///
/// * `config` - Repository storage paths.
/// * `window_start` - Inclusive earliest generation time.
/// * `window_end` - Inclusive latest generation time.
/// * `cache` - Process-local bounded parser cache.
///
/// # Returns
///
/// The same ordered source snapshots as an uncached read.
pub fn load_weekly_manifests_with_cache(
    config: &StorageConfig,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    cache: &WeeklyManifestCache,
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    load_weekly_manifests_inner(config, window_start, window_end, Some(cache))
}

fn load_weekly_manifests_inner(
    config: &StorageConfig,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    cache: Option<&WeeklyManifestCache>,
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    let mut seen = HashSet::new();
    for path in weekly_manifest_paths(&push_state_dir)? {
        let parsed = match cache {
            Some(cache) => cache.read(&path)?,
            None => parse_weekly_manifest(read_weekly_manifest_payload(&path)?),
        };
        let Some(manifest) = parsed else {
            continue;
        };
        if manifest.generated_at >= window_start
            && manifest.generated_at <= window_end
            && seen.insert((
                manifest.db_name.clone(),
                manifest.run_id.clone(),
                manifest.generated_at,
                manifest.article_ids.clone(),
            ))
        {
            manifests.push(manifest);
        }
    }
    manifests.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| left.db_name.cmp(&right.db_name))
            .then_with(|| left.run_id.cmp(&right.run_id))
            .then_with(|| left.article_ids.cmp(&right.article_ids))
    });
    Ok(manifests)
}

fn weekly_manifest_paths(push_state_dir: &Path) -> Result<Vec<PathBuf>, IndexRepositoryError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(push_state_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && is_change_manifest_path(&entry.path()) {
            paths.push(entry.path());
        }
    }
    let history_directory = push_state_dir.join("history");
    if history_directory.exists() {
        for catalog_entry in fs::read_dir(history_directory)? {
            let catalog_entry = catalog_entry?;
            if !catalog_entry.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(catalog_entry.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() && is_managed_history_manifest_path(&entry.path()) {
                    paths.push(entry.path());
                }
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_change_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".changes.json"))
}

fn is_managed_history_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_suffix(".changes.json"))
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

#[derive(Debug, Deserialize)]
pub(crate) struct WeeklyManifestPayload {
    db_name: Option<String>,
    generated_at: Option<String>,
    run_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    notifiable_article_ids: Vec<i64>,
}

fn read_weekly_manifest_payload(
    path: &Path,
) -> Result<WeeklyManifestPayload, IndexRepositoryError> {
    let reader = std::io::BufReader::new(fs::File::open(path)?);
    Ok(serde_json::from_reader(reader)?)
}

pub(crate) fn parse_weekly_manifest(payload: WeeklyManifestPayload) -> Option<WeeklyManifest> {
    let db_name = payload.db_name.as_deref().and_then(normalize_db_name)?;
    let mut seen = HashSet::new();
    let mut article_ids = Vec::new();
    for item in payload.notifiable_article_ids {
        if seen.insert(item) {
            article_ids.push(item);
        }
    }
    if article_ids.is_empty() {
        return None;
    }
    let generated_at = payload
        .generated_at
        .as_deref()
        .or(payload.run_id.as_deref())
        .and_then(parse_manifest_datetime)?;
    Some(WeeklyManifest {
        db_name,
        run_id: payload.run_id,
        generated_at,
        article_ids,
    })
}

fn deserialize_json_i64_list<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = JsonValue::deserialize(deserializer)?;
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(JsonValue::as_i64).collect())
}

pub(crate) fn normalize_db_name(value: &str) -> Option<String> {
    let filename = Path::new(value.trim()).file_name()?.to_str()?;
    if filename.is_empty() {
        None
    } else if filename.ends_with(".sqlite") {
        Some(filename.to_string())
    } else {
        Some(format!("{filename}.sqlite"))
    }
}

pub(crate) fn parse_iso_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn parse_manifest_datetime(value: &str) -> Option<DateTime<Utc>> {
    parse_iso_datetime(value).or_else(|| {
        let value = value.trim();
        let timestamp = value.parse::<i64>().ok()?;
        if timestamp.to_string() != value {
            return None;
        }
        DateTime::<Utc>::from_timestamp(timestamp, 0)
    })
}

/// Resolve window membership to existing articles without changing source run identities.
///
/// # Arguments
///
/// * `config` - Repository storage paths.
/// * `window_end` - Fixed inclusive end of the seven-day window.
/// * `selected_databases` - Canonical database names; an empty slice selects all.
///
/// # Returns
///
/// Nonempty source snapshots containing only articles also visible to weekly queries.
pub fn load_available_weekly_manifests(
    config: &StorageConfig,
    window_end: DateTime<Utc>,
    selected_databases: &[String],
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    let mut manifests = load_weekly_manifests(config, weekly_window_start(window_end), window_end)?;
    manifests.retain(|manifest| {
        selected_databases.is_empty() || selected_databases.contains(&manifest.db_name)
    });
    let mut requested_ids: HashMap<String, HashSet<i64>> = HashMap::new();
    for manifest in &mut manifests {
        manifest.run_id = manifest.run_id.take().and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        });
        if manifest.run_id.is_none() {
            manifest.run_id = Some(format!(
                "weekly-{}",
                litradar_domain::stable_sqlite_id(
                    format!(
                        "{}:{}:{:?}",
                        manifest.db_name, manifest.generated_at, manifest.article_ids
                    ),
                    "weekly-manifest",
                )
            ));
        }
        requested_ids
            .entry(manifest.db_name.clone())
            .or_default()
            .extend(manifest.article_ids.iter().copied());
    }
    let mut available_ids = HashMap::new();
    for (db_name, article_ids) in requested_ids {
        let db_path = config.index_dir().join(&db_name);
        if !db_path.exists() {
            continue;
        }
        let connection = open_sqlite_connection(db_path)?;
        let article_ids = article_ids.into_iter().collect::<Vec<_>>();
        let mut existing = HashSet::new();
        for chunk in article_ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let mut statement = connection.prepare(&format!(
                "SELECT l.article_id FROM article_listing l \
                 JOIN journals j ON j.journal_id = l.journal_id \
                 WHERE l.article_id IN ({placeholders})"
            ))?;
            let rows = statement.query_map(params_from_iter(chunk), |row| row.get::<_, i64>(0))?;
            for row in rows {
                existing.insert(row?);
            }
        }
        available_ids.insert(db_name, existing);
    }
    for manifest in &mut manifests {
        manifest.article_ids.retain(|article_id| {
            available_ids
                .get(&manifest.db_name)
                .is_some_and(|ids| ids.contains(article_id))
        });
    }
    manifests.retain(|manifest| !manifest.article_ids.is_empty());
    Ok(manifests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekly_manifest_cache_expires_without_wall_clock_sleep() {
        let directory = tempfile::tempdir().expect("temporary source should exist");
        let path = directory.path().join("fixture.changes.json");
        fs::write(&path, r#"{"db_name":"fixture.sqlite","generated_at":"2026-09-04T00:00:00Z","notifiable_article_ids":[1]}"#)
            .expect("source should write");
        let cache = WeeklyManifestCache::default();
        let now = Instant::now();
        cache.read_at(&path, now).expect("source should cache");
        cache
            .read_at(&path, now + Duration::from_secs(59))
            .expect("fresh entry should reuse");
        assert_eq!(cache.stats().parse_attempts, 1);
        cache
            .read_at(&path, now + MANIFEST_CACHE_TTL)
            .expect("expired entry should reread");
        assert_eq!(cache.stats().parse_attempts, 2);
    }
}
