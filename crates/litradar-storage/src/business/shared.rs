//! Shared business repository connection and row helpers.

use super::*;

/// Normalize database names using Python-compatible filename semantics.
///
/// # Arguments
///
/// * `db_names` - Raw database names.
///
/// # Returns
///
/// Normalized `.sqlite` filenames in first-seen order.
pub fn normalize_database_names(db_names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for db_name in db_names {
        let Some(filename) = Path::new(db_name.trim())
            .file_name()
            .and_then(|value| value.to_str())
        else {
            continue;
        };
        if filename.is_empty() {
            continue;
        }
        let candidate = if filename.ends_with(".sqlite") {
            filename.to_string()
        } else {
            format!("{filename}.sqlite")
        };
        if seen.insert(candidate.clone()) {
            normalized.push(candidate);
        }
    }
    normalized
}

/// List available index database filenames.
///
/// # Arguments
///
/// * `config` - Storage path configuration.
///
/// # Returns
///
/// Sorted database filenames.
pub fn list_available_database_names(
    config: &StorageConfig,
) -> Result<Vec<String>, BusinessRepositoryError> {
    Ok(config
        .list_index_databases()
        .map_err(|error| BusinessRepositoryError::Io(std::io::Error::other(error)))?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .collect())
}

/// Count weekly article ids from push-state change manifests.
///
/// # Arguments
///
/// * `config` - Storage path configuration.
/// * `selected_databases` - Normalized selected database names; empty means all.
///
/// # Returns
///
/// Number of unique weekly article/database pairs.
pub fn count_weekly_articles(
    config: &StorageConfig,
    selected_databases: &[String],
) -> Result<usize, BusinessRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(0);
    }
    let mut seen = HashSet::new();
    for entry in fs::read_dir(push_state_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json")
            || !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".changes.json"))
        {
            continue;
        }
        let manifest = read_weekly_article_count_manifest(&path)?;
        let Some(db_name) = manifest
            .db_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if !is_database_selected(selected_databases, &db_name) {
            continue;
        }
        for article_id in manifest
            .notifiable_article_ids
            .into_iter()
            .chain(manifest.backfill_article_ids)
        {
            seen.insert((db_name.clone(), article_id));
        }
    }
    Ok(seen.len())
}

#[derive(Debug, Deserialize)]
struct WeeklyArticleCountManifest {
    db_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    notifiable_article_ids: Vec<i64>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    backfill_article_ids: Vec<i64>,
}

fn read_weekly_article_count_manifest(
    path: &Path,
) -> Result<WeeklyArticleCountManifest, BusinessRepositoryError> {
    let reader = std::io::BufReader::new(fs::File::open(path)?);
    Ok(serde_json::from_reader(reader)?)
}

fn deserialize_json_i64_list<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(Value::as_i64).collect())
}

pub(super) fn open_business_connection(
    path: impl AsRef<Path>,
) -> Result<Connection, BusinessRepositoryError> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    Ok(open_sqlite_connection(path)?)
}

fn is_database_selected(selected_databases: &[String], db_name: &str) -> bool {
    let normalized_target = normalize_database_names(&[db_name.to_string()]);
    if normalized_target.is_empty() {
        return false;
    }
    selected_databases.is_empty() || selected_databases.contains(&normalized_target[0])
}

pub(super) fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, BusinessRepositoryError> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub(super) fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::StorageConfig;

    #[test]
    fn tracking_weekly_article_count_reads_only_needed_manifest_fields() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let config = StorageConfig::from_project_root(temp_dir.path());
        let push_state_dir = config.project_root().join("data").join("push_state");
        fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
        fs::write(
            push_state_dir.join("fixture.changes.json"),
            r#"{"db_name":"fixture.sqlite","notifiable_article_ids":[10,10,"11",null],"backfill_article_ids":[12,"13"],"summary":{"issues":[{"added_article_ids":[10,11,12,13]}]}}"#,
        )
        .expect("manifest should write");
        fs::write(
            push_state_dir.join("missing-db.changes.json"),
            r#"{"notifiable_article_ids":[99],"summary":{"added_article_ids":[99]}}"#,
        )
        .expect("missing db manifest should write");

        let all_count =
            count_weekly_articles(&config, &[]).expect("weekly article count should load");
        let selected_count = count_weekly_articles(&config, &["fixture.sqlite".to_string()])
            .expect("selected weekly article count should load");
        let unselected_count = count_weekly_articles(&config, &["other.sqlite".to_string()])
            .expect("unselected weekly article count should load");

        assert_eq!(all_count, 2);
        assert_eq!(selected_count, 2);
        assert_eq!(unselected_count, 0);
    }
}
