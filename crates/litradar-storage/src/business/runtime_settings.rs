//! Managed runtime setting repositories.

use std::collections::{BTreeMap, BTreeSet};

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

const RUNTIME_CONFIG_DEFINITIONS: [RuntimeConfigDefinition; 11] = [
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
        description: "Comma-separated exact HTTP(S) origins for credentialed API requests; paths, wildcard, user-info, query, fragment, and null are rejected. Changes apply after API restart.",
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
        description: "Comma-separated exact HTTP(S) origins accepted by the Streamable HTTP MCP endpoint; null is also supported. Changes apply after API restart.",
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
    RuntimeConfigDefinition {
        field: "index_provider_routes",
        label: "Index provider routes",
        input_type: "text",
        is_secret: false,
        description: "JSON object mapping each catalog stem to one registered indexing provider.",
        default_value: "{\"ccf_computer_journals\":\"scholarly\",\"chinese_journals\":\"cnki\",\"english_journals\":\"scholarly\"}",
    },
    RuntimeConfigDefinition {
        field: "article_detail_provider_order",
        label: "Article detail provider order",
        input_type: "text",
        is_secret: false,
        description: "Ordered comma-separated providers for live article detail resolution.",
        default_value: "scholarly,cnki",
    },
    RuntimeConfigDefinition {
        field: "article_abstract_provider_order",
        label: "Article abstract provider order",
        input_type: "text",
        is_secret: false,
        description: "Ordered comma-separated providers for live article abstract-page resolution.",
        default_value: "scholarly,cnki",
    },
    RuntimeConfigDefinition {
        field: "article_fulltext_provider_order",
        label: "Article full-text provider order",
        input_type: "text",
        is_secret: false,
        description: "Ordered comma-separated providers for live article full-text resolution.",
        default_value: "zjlib_cnki",
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
/// * `secret_pool_updates` - Incremental secret-pool mutations keyed by API field name.
///
/// # Returns
///
/// Updated runtime setting payloads.
pub fn upsert_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    values: &HashMap<String, Option<String>>,
    secret_pool_updates: &HashMap<String, RuntimeSecretPoolUpdate>,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = read_runtime_setting_rows(&transaction)?;
    let fields = values
        .keys()
        .chain(secret_pool_updates.keys())
        .cloned()
        .collect::<HashSet<_>>();
    {
        let mut statement = transaction.prepare(
            "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?;
        for field in fields {
            let definition = runtime_definition_by_field(&field)
                .ok_or_else(|| BusinessRepositoryError::UnknownRuntimeSetting(field.clone()))?;
            let current =
                internal_runtime_setting_from_definition(definition, existing.get(&field), codec)?
                    .value;
            let mut value = if let Some(update) = values.get(&field) {
                if definition.is_secret {
                    match update {
                        None => String::new(),
                        Some(raw_value) if raw_value.trim().is_empty() => current,
                        Some(raw_value) => raw_value.trim().to_string(),
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
                }
            } else {
                current
            };
            if let Some(pool_update) = secret_pool_updates.get(&field) {
                value = apply_secret_pool_update(definition, &value, pool_update, codec)?;
            }
            if !definition.is_secret && definition.input_type == "boolean" {
                let default = definition.default_value.trim().eq_ignore_ascii_case("true");
                value = runtime_bool_to_text(&value, default)?;
            }
            if !definition.is_secret {
                value = normalize_runtime_setting_value(definition, &value)?;
            }
            let stored_value = if definition.is_secret {
                codec.encrypt(&value, &runtime_context(&field))?
            } else {
                value
            };
            statement.execute(params![definition.field, stored_value, now])?;
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
    let secret_items = runtime_secret_items(definition, &internal.value, codec)?;
    let has_value = if is_secret_pool(definition) {
        !secret_items.is_empty()
    } else {
        !internal.value.trim().is_empty()
    };
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
        secret_items,
        source: internal.source,
        updated_at: internal.updated_at,
    })
}

fn apply_secret_pool_update(
    definition: &RuntimeConfigDefinition,
    value: &str,
    update: &RuntimeSecretPoolUpdate,
    codec: &SecretCodec,
) -> Result<String, BusinessRepositoryError> {
    if !is_secret_pool(definition) {
        return Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(
            definition.field.to_string(),
        ));
    }
    let mut pool = runtime_pool_from_text(value);
    let mut removals = HashSet::new();
    for reference in &update.remove {
        let item = codec
            .decrypt(reference, &runtime_secret_item_context(definition.field))
            .map_err(|_| {
                BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(
                    definition.field.to_string(),
                )
            })?;
        if item.is_empty() || !pool.iter().any(|candidate| candidate == &item) {
            return Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(
                definition.field.to_string(),
            ));
        }
        removals.insert(item);
    }
    pool.retain(|item| !removals.contains(item));
    for addition in &update.add {
        for item in runtime_pool_from_text(addition) {
            if !pool.iter().any(|candidate| candidate == &item) {
                pool.push(item);
            }
        }
    }
    Ok(pool.join("\n"))
}

fn runtime_secret_items(
    definition: &RuntimeConfigDefinition,
    value: &str,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSecretItemInfo>, BusinessRepositoryError> {
    if !is_secret_pool(definition) {
        return Ok(Vec::new());
    }
    runtime_pool_from_text(value)
        .into_iter()
        .map(|item| {
            Ok(RuntimeSecretItemInfo {
                reference: codec.encrypt(&item, &runtime_secret_item_context(definition.field))?,
                masked_value: mask_runtime_secret_item(&item),
            })
        })
        .collect()
}

fn runtime_pool_from_text(value: &str) -> Vec<String> {
    let mut pool = Vec::new();
    for part in value.split([',', ';', '\n']) {
        let item = part.trim();
        if !item.is_empty() && !pool.iter().any(|candidate| candidate == item) {
            pool.push(item.to_string());
        }
    }
    pool
}

fn mask_runtime_secret_item(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= 5 {
        return "*".repeat(characters.len());
    }
    format!(
        "{}{}",
        characters.iter().take(5).collect::<String>(),
        "*".repeat(characters.len() - 5)
    )
}

fn is_secret_pool(definition: &RuntimeConfigDefinition) -> bool {
    definition.is_secret && definition.field.ends_with("_pool")
}

fn runtime_secret_item_context(field: &str) -> String {
    format!("{}:pool-item-reference", runtime_context(field))
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
    let mut value = if definition.is_secret && row.is_some() {
        codec.decrypt(stored, &runtime_context(definition.field))?
    } else {
        stored.to_string()
    };
    if !definition.is_secret {
        value = normalize_runtime_setting_value(definition, &value)?;
    }
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

fn normalize_runtime_setting_value(
    definition: &RuntimeConfigDefinition,
    value: &str,
) -> Result<String, BusinessRepositoryError> {
    match definition.field {
        "index_provider_routes" => normalize_index_provider_routes(value),
        "article_detail_provider_order"
        | "article_abstract_provider_order"
        | "article_fulltext_provider_order" => normalize_provider_order(definition.field, value),
        _ => Ok(value.to_string()),
    }
}

fn normalize_index_provider_routes(value: &str) -> Result<String, BusinessRepositoryError> {
    let routes = serde_json::from_str::<BTreeMap<String, String>>(value)?;
    if routes.is_empty() {
        return Err(invalid_runtime_setting(
            "index_provider_routes",
            "at least one catalog route is required",
        ));
    }
    for (catalog, provider) in &routes {
        if !is_runtime_name(catalog) {
            return Err(invalid_runtime_setting(
                "index_provider_routes",
                "catalog stems must use lowercase ASCII names",
            ));
        }
        if !is_runtime_name(provider) {
            return Err(invalid_runtime_setting(
                "index_provider_routes",
                "provider names must use lowercase ASCII names",
            ));
        }
    }
    Ok(serde_json::to_string(&routes)?)
}

fn normalize_provider_order(field: &str, value: &str) -> Result<String, BusinessRepositoryError> {
    if value.trim().is_empty() {
        return Ok(String::new());
    }
    let mut providers = Vec::new();
    let mut seen = BTreeSet::new();
    for part in value.split(',') {
        let provider = part.trim();
        if !is_runtime_name(provider) {
            return Err(invalid_runtime_setting(
                field,
                "provider order must contain lowercase ASCII names",
            ));
        }
        if !seen.insert(provider.to_string()) {
            return Err(invalid_runtime_setting(
                field,
                "provider order must not contain duplicates",
            ));
        }
        providers.push(provider.to_string());
    }
    Ok(providers.join(","))
}

fn is_runtime_name(value: &str) -> bool {
    (2..=128).contains(&value.len())
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'.' | b'_' | b'-' => index > 0,
            _ => false,
        })
}

fn invalid_runtime_setting(field: &str, detail: &str) -> BusinessRepositoryError {
    BusinessRepositoryError::Json(serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("invalid {field}: {detail}"),
    )))
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

        assert_eq!(settings.len(), 11);
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
    fn runtime_provider_routes_and_orders_are_validated_and_normalized() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([31_u8; 32]);
        let values = HashMap::from([
            (
                "index_provider_routes".to_string(),
                Some(
                    "{ \"english_journals\": \"scholarly\", \"chinese_journals\": \"cnki\" }"
                        .to_string(),
                ),
            ),
            (
                "article_detail_provider_order".to_string(),
                Some("scholarly, cnki".to_string()),
            ),
            (
                "article_abstract_provider_order".to_string(),
                Some(String::new()),
            ),
        ]);

        let settings = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect("provider settings should update");
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.field == "index_provider_routes")
                .expect("route setting should exist")
                .value,
            "{\"chinese_journals\":\"cnki\",\"english_journals\":\"scholarly\"}"
        );
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.field == "article_detail_provider_order")
                .expect("detail order should exist")
                .value,
            "scholarly,cnki"
        );
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.field == "article_abstract_provider_order")
                .expect("abstract order should exist")
                .value,
            ""
        );

        for invalid in [
            ("index_provider_routes", "{\"chinese_journals\":\"CNKI\"}"),
            ("article_detail_provider_order", "scholarly,scholarly"),
            ("article_fulltext_provider_order", "zjlib cnki"),
        ] {
            let error = upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::from([(invalid.0.to_string(), Some(invalid.1.to_string()))]),
                &HashMap::new(),
            )
            .expect_err("invalid provider setting should fail");
            assert!(matches!(error, BusinessRepositoryError::Json(_)));
        }
    }
    #[test]
    fn runtime_settings_reject_proxy_pool_and_normalize_boolean() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([8_u8; 32]);
        let mut values = HashMap::new();
        values.insert("secure_cookies".to_string(), Some("yes".to_string()));

        let settings = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect("runtime settings should update");
        let secure_cookies = settings
            .iter()
            .find(|setting| setting.field == "secure_cookies")
            .expect("secure cookie setting should exist");

        assert_eq!(secure_cookies.value, "true");
        assert_eq!(secure_cookies.source, "database");

        values.clear();
        values.insert("proxy_pool".to_string(), Some("proxy".to_string()));
        let error = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
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

        let public = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect("secret runtime setting should update");
        let openalex = public
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert!(openalex.has_value);
        assert_eq!(openalex.masked_value, "••••");
        assert_eq!(openalex.secret_items.len(), 2);
        assert_eq!(openalex.secret_items[0].masked_value, "key-o**");
        assert_eq!(openalex.secret_items[1].masked_value, "key-t**");
        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted setting should load");
        assert!(raw.starts_with("litradarenc:v1:"));
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
            &HashMap::new(),
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
            &HashMap::new(),
        )
        .expect("null secret should clear");
        let openalex = cleared
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert!(!openalex.has_value);
        assert!(openalex.masked_value.is_empty());
        assert!(openalex.secret_items.is_empty());
    }

    #[test]
    fn runtime_secret_pool_updates_are_exact_atomic_and_secret_safe() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([29_u8; 32]);
        let initial_values = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            Some("abcde-one,abcde-two,tiny".to_string()),
        )]);
        let initial =
            upsert_runtime_settings(&auth_db_path, &codec, &initial_values, &HashMap::new())
                .expect("initial secret pool should update");
        let openalex = initial
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");

        assert_eq!(openalex.secret_items.len(), 3);
        assert_eq!(openalex.secret_items[0].masked_value, "abcde****");
        assert_eq!(openalex.secret_items[1].masked_value, "abcde****");
        assert_eq!(openalex.secret_items[2].masked_value, "****");
        assert!(!format!("{openalex:?}").contains(&openalex.secret_items[0].reference));

        let first_reference = openalex.secret_items[0].reference.clone();
        let second_reference = openalex.secret_items[1].reference.clone();
        let pool_updates = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["abcde-three; abcde-two\nnew-key".to_string()],
                remove: vec![first_reference.clone()],
            },
        )]);
        upsert_runtime_settings(&auth_db_path, &codec, &HashMap::new(), &pool_updates)
            .expect("incremental pool update should succeed");

        let internal = load_runtime_settings(&auth_db_path, &codec)
            .expect("updated secret pool should decrypt");
        let updated_value = &internal
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist")
            .value;
        assert_eq!(updated_value, "abcde-two\ntiny\nabcde-three\nnew-key");
        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted setting should load");
        assert!(raw.starts_with("litradarenc:v1:"));
        assert!(!raw.contains("abcde-two"));

        let stale_update = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["must-not-commit".to_string()],
                remove: vec![first_reference],
            },
        )]);
        let stale_error =
            upsert_runtime_settings(&auth_db_path, &codec, &HashMap::new(), &stale_update)
                .expect_err("stale item reference should fail");
        assert!(matches!(
            stale_error,
            BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field)
                if field == "openalex_api_key_pool"
        ));
        assert_eq!(
            load_runtime_settings(&auth_db_path, &codec)
                .expect("failed update should roll back")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "abcde-two\ntiny\nabcde-three\nnew-key"
        );

        let tampered_update = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: Vec::new(),
                remove: vec![format!("{second_reference}A")],
            },
        )]);
        assert!(matches!(
            upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::new(),
                &tampered_update,
            ),
            Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field))
                if field == "openalex_api_key_pool"
        ));

        let cross_field_update = HashMap::from([(
            "semantic_scholar_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: Vec::new(),
                remove: vec![second_reference],
            },
        )]);
        assert!(matches!(
            upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::new(),
                &cross_field_update,
            ),
            Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field))
                if field == "semantic_scholar_api_key_pool"
        ));

        let non_secret_update = HashMap::from([(
            "crossref_mailto_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["admin@example.test".to_string()],
                remove: Vec::new(),
            },
        )]);
        assert!(matches!(
            upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::new(),
                &non_secret_update,
            ),
            Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field))
                if field == "crossref_mailto_pool"
        ));

        let replacement = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["replacement-key".to_string()],
                remove: Vec::new(),
            },
        )]);
        upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), None)]),
            &replacement,
        )
        .expect("clear then add should replace the pool");
        assert_eq!(
            load_runtime_settings(&auth_db_path, &codec)
                .expect("replacement pool should decrypt")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "replacement-key"
        );
    }
}
