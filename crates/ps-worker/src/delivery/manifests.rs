//! Manual weekly change-manifest discovery and parsing.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManualWeeklyManifest {
    pub(super) db_name: String,
    pub(super) path: PathBuf,
}

pub(super) fn manual_weekly_manifests(
    project_root: &Path,
    selected_databases: &[String],
) -> Result<Vec<ManualWeeklyManifest>, DeliveryError> {
    let push_state_dir = project_root.join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in
        fs::read_dir(push_state_dir).map_err(|error| DeliveryError::Manual(error.to_string()))?
    {
        let path = entry
            .map_err(|error| DeliveryError::Manual(error.to_string()))?
            .path();
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".changes.json"))
        {
            continue;
        }
        let payload = read_manual_manifest_payload(&path)?;
        let Some(db_name) = manual_manifest_db_name(&payload) else {
            continue;
        };
        if !is_database_selected(selected_databases, &db_name) {
            continue;
        }
        if !manual_manifest_has_notifiable_articles(&payload) {
            continue;
        }
        manifests.push(ManualWeeklyManifest { db_name, path });
    }
    manifests.sort_by(|left, right| {
        left.db_name
            .cmp(&right.db_name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(manifests)
}

#[derive(Debug, Deserialize)]
struct ManualManifestPayload {
    db_name: Option<String>,
    db_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    notifiable_article_ids: Vec<i64>,
}

fn read_manual_manifest_payload(path: &Path) -> Result<ManualManifestPayload, DeliveryError> {
    let reader = std::io::BufReader::new(
        fs::File::open(path).map_err(|error| DeliveryError::Manual(error.to_string()))?,
    );
    serde_json::from_reader(reader).map_err(|error| DeliveryError::Manual(error.to_string()))
}

fn manual_manifest_db_name(payload: &ManualManifestPayload) -> Option<String> {
    let value = payload.db_name.as_deref().or(payload.db_path.as_deref())?;
    normalize_db_name(value)
}

fn manual_manifest_has_notifiable_articles(payload: &ManualManifestPayload) -> bool {
    !payload.notifiable_article_ids.is_empty()
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

fn normalize_db_name(value: &str) -> Option<String> {
    let filename = Path::new(value.trim()).file_name()?.to_str()?;
    if filename.is_empty() {
        None
    } else if filename.ends_with(".sqlite") {
        Some(filename.to_string())
    } else {
        Some(format!("{filename}.sqlite"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn manual_weekly_manifests_parse_only_needed_fields() {
        let root = tempdir().expect("temp dir should be created");
        let push_state_dir = root.path().join("data").join("push_state");
        fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
        fs::write(
            push_state_dir.join("alpha.changes.json"),
            r#"{"db_name":"alpha","notifiable_article_ids":[10,"11",null],"summary":{"issues":[{"added_article_ids":[10,11,12]}]}}"#,
        )
        .expect("alpha manifest should write");
        fs::write(
            push_state_dir.join("beta.changes.json"),
            r#"{"db_name":"beta","notifiable_article_ids":["12"],"summary":{"added_article_ids":[12]}}"#,
        )
        .expect("beta manifest should write");
        fs::write(
            push_state_dir.join("runtime.json"),
            r#"{"status":"completed"}"#,
        )
        .expect("runtime state should write");

        let manifests =
            manual_weekly_manifests(root.path(), &[]).expect("manual weekly manifests should load");

        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].db_name, "alpha.sqlite");
        assert_eq!(
            manifests[0]
                .path
                .file_name()
                .and_then(|value| value.to_str()),
            Some("alpha.changes.json")
        );
    }
}
