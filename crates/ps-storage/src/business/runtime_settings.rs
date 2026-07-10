//! Managed runtime setting repositories.

use super::shared::*;
use super::*;

#[derive(Debug, Clone, Copy)]
struct RuntimeConfigDefinition {
    field: &'static str,
    label: &'static str,
    input_type: &'static str,
    is_secret: bool,
    description: &'static str,
    default_value: &'static str,
}

const RUNTIME_CONFIG_DEFINITIONS: [RuntimeConfigDefinition; 7] = [
    RuntimeConfigDefinition {
        field: "openalex_api_key_pool",
        label: "OpenAlex API key pool",
        input_type: "password",
        is_secret: true,
        description: "OpenAlex authenticated request key pool.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "semantic_scholar_api_key_pool",
        label: "Semantic Scholar API key pool",
        input_type: "password",
        is_secret: true,
        description: "Comma- or semicolon-separated Semantic Scholar REST API keys.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "crossref_mailto_pool",
        label: "Crossref mailto pool",
        input_type: "email",
        is_secret: false,
        description: "Comma- or semicolon-separated Crossref contact emails.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "cors_allowed_origins",
        label: "CORS allowed origins",
        input_type: "text",
        is_secret: false,
        description: "Comma-separated browser origins allowed to send credentialed API requests.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "mcp_allowed_hosts",
        label: "MCP allowed hosts",
        input_type: "text",
        is_secret: false,
        description: "Comma-separated hosts accepted by the Streamable HTTP MCP endpoint.",
        default_value: "localhost,127.0.0.1,::1",
    },
    RuntimeConfigDefinition {
        field: "mcp_allowed_origins",
        label: "MCP allowed origins",
        input_type: "text",
        is_secret: false,
        description:
            "Comma-separated browser origins accepted by the Streamable HTTP MCP endpoint.",
        default_value: "",
    },
    RuntimeConfigDefinition {
        field: "secure_cookies",
        label: "Secure session cookies",
        input_type: "boolean",
        is_secret: false,
        description: "Whether session cookies include the Secure attribute.",
        default_value: "false",
    },
];
/// List managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Runtime setting payloads.
pub fn list_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let rows = read_runtime_setting_rows(&connection)?;
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            public_runtime_setting_from_definition(definition, rows.get(definition.field), codec)
        })
        .collect()
}

/// Load managed runtime settings for trusted backend consumers.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
///
/// # Returns
///
/// Effective values with secret fields decrypted in non-serializable types.
pub fn load_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSettingValue>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let rows = read_runtime_setting_rows(&connection)?;
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            internal_runtime_setting_from_definition(definition, rows.get(definition.field), codec)
        })
        .collect()
}

/// Upsert managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `values` - Values keyed by API field name; null clears secret fields.
///
/// # Returns
///
/// Updated runtime setting payloads.
pub fn upsert_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    values: &HashMap<String, Option<String>>,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = read_runtime_setting_rows(&transaction)?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?;
        for (field, update) in values {
            let definition = runtime_definition_by_field(field)
                .ok_or_else(|| BusinessRepositoryError::UnknownRuntimeSetting(field.clone()))?;
            let current = existing.get(field).map(|row| row.0.as_str());
            let mut value = if definition.is_secret {
                if let Some(stored) = current {
                    codec.decrypt(stored, &runtime_context(field))?;
                }
                match update {
                    None => String::new(),
                    Some(raw_value) if raw_value.trim().is_empty() => {
                        current.unwrap_or_default().to_string()
                    }
                    Some(raw_value) => codec.encrypt(raw_value.trim(), &runtime_context(field))?,
                }
            } else {
                update
                    .as_deref()
                    .ok_or_else(|| {
                        BusinessRepositoryError::NonSecretRuntimeSettingCannotBeCleared(
                            field.clone(),
                        )
                    })?
                    .trim()
                    .to_string()
            };
            if !definition.is_secret && definition.input_type == "boolean" {
                let default = definition.default_value.trim().eq_ignore_ascii_case("true");
                value = runtime_bool_to_text(&value, default)?;
            }
            statement.execute(params![definition.field, value, now])?;
        }
    }
    transaction.commit()?;
    list_runtime_settings(auth_db_path, codec)
}
fn read_runtime_setting_rows(
    connection: &Connection,
) -> Result<HashMap<String, (String, f64)>, BusinessRepositoryError> {
    let mut statement =
        connection.prepare("SELECT key, value, updated_at FROM runtime_settings")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    Ok(collect_rows(rows)?
        .into_iter()
        .map(|(key, value, updated_at)| (key, (value, updated_at)))
        .collect())
}

fn public_runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
    codec: &SecretCodec,
) -> Result<RuntimeSettingInfo, BusinessRepositoryError> {
    let internal = internal_runtime_setting_from_definition(definition, row, codec)?;
    let has_value = !internal.value.trim().is_empty();
    Ok(RuntimeSettingInfo {
        field: definition.field.to_string(),
        label: definition.label.to_string(),
        description: definition.description.to_string(),
        input_type: definition.input_type.to_string(),
        is_secret: definition.is_secret,
        value: if definition.is_secret {
            String::new()
        } else {
            internal.value
        },
        has_value,
        masked_value: if definition.is_secret && has_value {
            "••••".to_string()
        } else {
            String::new()
        },
        source: internal.source,
        updated_at: internal.updated_at,
    })
}

fn internal_runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
    codec: &SecretCodec,
) -> Result<RuntimeSettingValue, BusinessRepositoryError> {
    let (stored, source, updated_at) = if let Some((value, updated_at)) = row {
        (value.as_str(), "database".to_string(), Some(*updated_at))
    } else {
        (definition.default_value, "default".to_string(), None)
    };
    let value = if definition.is_secret && row.is_some() {
        codec.decrypt(stored, &runtime_context(definition.field))?
    } else {
        stored.to_string()
    };
    Ok(RuntimeSettingValue {
        field: definition.field.to_string(),
        value,
        source,
        updated_at,
    })
}

fn runtime_definition_by_field(field: &str) -> Option<&'static RuntimeConfigDefinition> {
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .find(|definition| definition.field == field)
}

fn runtime_bool_to_text(value: &str, default: bool) -> Result<String, BusinessRepositoryError> {
    let text = value.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Ok(default.to_string());
    }
    if matches!(text.as_str(), "1" | "true" | "yes" | "on") {
        return Ok("true".to_string());
    }
    if matches!(text.as_str(), "0" | "false" | "no" | "off") {
        return Ok("false".to_string());
    }
    Err(BusinessRepositoryError::InvalidRuntimeBoolean(
        value.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::{migrate_auth_database, SecretCodec};

    #[test]
    fn runtime_settings_ignore_stale_env_keys_and_proxy_pool() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                ("OPENALEX_API_KEY_POOL", "env-key", 1.0_f64),
            )
            .expect("stale env-key row should insert");
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                ("PROXY_POOL", "proxy", 1.0_f64),
            )
            .expect("stale proxy row should insert");

        let codec = SecretCodec::from_key([8_u8; 32]);
        let settings =
            list_runtime_settings(&auth_db_path, &codec).expect("runtime settings should load");
        let fields = settings
            .iter()
            .map(|setting| setting.field.as_str())
            .collect::<Vec<_>>();

        assert_eq!(settings.len(), 7);
        assert!(fields.contains(&"openalex_api_key_pool"));
        assert!(fields.contains(&"secure_cookies"));
        assert!(!fields.contains(&"proxy_pool"));
        assert!(settings
            .iter()
            .all(|setting| setting.source == "database" || setting.source == "default"));
        let openalex = settings
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert_eq!(openalex.source, "default");
    }
    #[test]
    fn runtime_settings_reject_proxy_pool_and_normalize_boolean() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([8_u8; 32]);
        let mut values = HashMap::new();
        values.insert("secure_cookies".to_string(), Some("yes".to_string()));

        let settings = upsert_runtime_settings(&auth_db_path, &codec, &values)
            .expect("runtime settings should update");
        let secure_cookies = settings
            .iter()
            .find(|setting| setting.field == "secure_cookies")
            .expect("secure cookie setting should exist");

        assert_eq!(secure_cookies.value, "true");
        assert_eq!(secure_cookies.source, "database");

        values.clear();
        values.insert("proxy_pool".to_string(), Some("proxy".to_string()));
        let error = upsert_runtime_settings(&auth_db_path, &codec, &values)
            .expect_err("proxy pool should be rejected");

        assert!(matches!(
            error,
            BusinessRepositoryError::UnknownRuntimeSetting(field) if field == "proxy_pool"
        ));
    }

    #[test]
    fn runtime_credentials_are_encrypted_and_use_preserve_replace_clear_updates() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([23_u8; 32]);
        let values = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            Some("key-one,key-two".to_string()),
        )]);

        let public = upsert_runtime_settings(&auth_db_path, &codec, &values)
            .expect("secret runtime setting should update");
        let openalex = public
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert!(openalex.has_value);
        assert_eq!(openalex.masked_value, "••••");
        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted setting should load");
        assert!(raw.starts_with("psenc:v1:"));
        assert!(!raw.contains("key-one"));
        let internal = super::load_runtime_settings(&auth_db_path, &codec)
            .expect("trusted settings should decrypt");
        assert_eq!(
            internal
                .iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "key-one,key-two"
        );

        upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), Some(" ".to_string()))]),
        )
        .expect("blank secret should preserve");
        assert_eq!(
            super::load_runtime_settings(&auth_db_path, &codec)
                .expect("trusted settings should decrypt")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "key-one,key-two"
        );

        let cleared = upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), None)]),
        )
        .expect("null secret should clear");
        let openalex = cleared
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert!(!openalex.has_value);
        assert!(openalex.masked_value.is_empty());
    }
}
