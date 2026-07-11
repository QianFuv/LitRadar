//! Notification settings and subscriber repositories.

use super::shared::*;
use super::*;

/// Get notification settings for a user.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `user_id` - Owner user identifier.
///
/// # Returns
///
/// Notification settings or None.
pub fn get_notification_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<Option<NotificationSettings>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT id, user_id, keywords, directions, selected_databases, delivery_method, \
             pushplus_token, pushplus_template, pushplus_topic, pushplus_channel, \
             sync_to_tracking_folder, ai_base_url, ai_api_key, ai_model, ai_system_prompt, \
             ai_backup_base_url, ai_backup_api_key, ai_backup_model, ai_backup_system_prompt, \
             ai_retry_attempts, enabled, created_at, updated_at \
            FROM notification_settings WHERE user_id = ?1",
            [user_id.value()],
            |row| notification_settings_from_row(row, codec),
        )
        .optional()
        .map_err(BusinessRepositoryError::from)?
        .transpose()
}

/// List all enabled notification subscribers with tracking folder metadata.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
///
/// # Returns
///
/// Enabled subscriber settings ordered by user id.
pub fn list_notification_subscribers(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<NotificationSubscriberInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let mut statement = connection.prepare(
        "SELECT ns.user_id, u.username, ns.keywords, ns.directions, ns.selected_databases, \
         ns.delivery_method, ns.pushplus_token, ns.pushplus_template, ns.pushplus_topic, \
         ns.pushplus_channel, ns.sync_to_tracking_folder, ns.ai_base_url, ns.ai_api_key, \
         ns.ai_model, ns.ai_system_prompt, ns.ai_backup_base_url, ns.ai_backup_api_key, \
         ns.ai_backup_model, ns.ai_backup_system_prompt, ns.ai_retry_attempts, \
         (SELECT id FROM folders f WHERE f.user_id = ns.user_id AND f.is_tracking = 1 LIMIT 1) \
             AS tracking_folder_id \
         FROM notification_settings ns JOIN users u ON u.id = ns.user_id \
         WHERE ns.enabled = 1 ORDER BY ns.user_id",
    )?;
    let rows = statement.query_map([], |row| notification_subscriber_from_row(row, codec))?;
    collect_nested_rows(rows)
}

/// Get one enabled notification subscriber with tracking folder metadata.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `user_id` - Subscriber user identifier.
///
/// # Returns
///
/// The enabled subscriber, or None when settings are missing or disabled.
pub fn get_notification_subscriber(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<Option<NotificationSubscriberInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT ns.user_id, u.username, ns.keywords, ns.directions, ns.selected_databases, \
             ns.delivery_method, ns.pushplus_token, ns.pushplus_template, ns.pushplus_topic, \
             ns.pushplus_channel, ns.sync_to_tracking_folder, ns.ai_base_url, ns.ai_api_key, \
             ns.ai_model, ns.ai_system_prompt, ns.ai_backup_base_url, ns.ai_backup_api_key, \
             ns.ai_backup_model, ns.ai_backup_system_prompt, ns.ai_retry_attempts, \
             (SELECT id FROM folders f WHERE f.user_id = ns.user_id AND f.is_tracking = 1 LIMIT 1) \
                 AS tracking_folder_id \
             FROM notification_settings ns JOIN users u ON u.id = ns.user_id \
             WHERE ns.enabled = 1 AND ns.user_id = ?1",
            [user_id.value()],
            |row| notification_subscriber_from_row(row, codec),
        )
        .optional()
        .map_err(BusinessRepositoryError::from)?
        .transpose()
}

/// Create or update notification settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `user_id` - Owner user identifier.
/// * `settings` - Normalized notification settings.
///
/// # Returns
///
/// Updated notification settings.
pub fn upsert_notification_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
    settings: &NotificationSettingsUpdate,
) -> Result<NotificationSettings, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let keywords = serde_json::to_string(&settings.keywords)?;
    let directions = serde_json::to_string(&settings.directions)?;
    let selected_databases = serde_json::to_string(&settings.selected_databases)?;
    let current_secrets = connection
        .query_row(
            "SELECT pushplus_token, ai_api_key, ai_backup_api_key \
             FROM notification_settings WHERE user_id = ?1",
            [user_id.value()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((pushplus_token, ai_api_key, ai_backup_api_key)) = current_secrets.as_ref() {
        codec.decrypt(
            pushplus_token,
            &notification_context(user_id.value(), "pushplus_token"),
        )?;
        codec.decrypt(
            ai_api_key,
            &notification_context(user_id.value(), "ai_api_key"),
        )?;
        codec.decrypt(
            ai_backup_api_key,
            &notification_context(user_id.value(), "ai_backup_api_key"),
        )?;
    }
    let pushplus_token = resolve_notification_secret(
        codec,
        user_id,
        "pushplus_token",
        &settings.pushplus_token,
        current_secrets.as_ref().map(|values| values.0.as_str()),
    )?;
    let ai_api_key = resolve_notification_secret(
        codec,
        user_id,
        "ai_api_key",
        &settings.ai_api_key,
        current_secrets.as_ref().map(|values| values.1.as_str()),
    )?;
    let ai_backup_api_key = resolve_notification_secret(
        codec,
        user_id,
        "ai_backup_api_key",
        &settings.ai_backup_api_key,
        current_secrets.as_ref().map(|values| values.2.as_str()),
    )?;
    connection.execute(
        "INSERT INTO notification_settings \
         (user_id, keywords, directions, selected_databases, delivery_method, \
          pushplus_token, pushplus_template, pushplus_topic, pushplus_channel, \
          sync_to_tracking_folder, ai_base_url, ai_api_key, ai_model, ai_system_prompt, \
          ai_backup_base_url, ai_backup_api_key, ai_backup_model, ai_backup_system_prompt, \
          ai_retry_attempts, enabled, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22) \
         ON CONFLICT(user_id) DO UPDATE SET \
          keywords = excluded.keywords, directions = excluded.directions, \
          selected_databases = excluded.selected_databases, delivery_method = excluded.delivery_method, \
          pushplus_token = excluded.pushplus_token, pushplus_template = excluded.pushplus_template, \
          pushplus_topic = excluded.pushplus_topic, pushplus_channel = excluded.pushplus_channel, \
          sync_to_tracking_folder = excluded.sync_to_tracking_folder, \
          ai_base_url = excluded.ai_base_url, ai_api_key = excluded.ai_api_key, \
          ai_model = excluded.ai_model, ai_system_prompt = excluded.ai_system_prompt, \
          ai_backup_base_url = excluded.ai_backup_base_url, ai_backup_api_key = excluded.ai_backup_api_key, \
          ai_backup_model = excluded.ai_backup_model, \
          ai_backup_system_prompt = excluded.ai_backup_system_prompt, \
          ai_retry_attempts = excluded.ai_retry_attempts, enabled = excluded.enabled, \
          updated_at = excluded.updated_at",
        params![
            user_id.value(),
            keywords,
            directions,
            selected_databases,
            settings.delivery_method,
            pushplus_token,
            settings.pushplus_template,
            settings.pushplus_topic,
            settings.pushplus_channel,
            settings.sync_to_tracking_folder as i64,
            settings.ai_base_url,
            ai_api_key,
            settings.ai_model,
            settings.ai_system_prompt,
            settings.ai_backup_base_url,
            ai_backup_api_key,
            settings.ai_backup_model,
            settings.ai_backup_system_prompt,
            settings.ai_retry_attempts,
            settings.enabled as i64,
            now,
            now
        ],
    )?;
    get_notification_settings(auth_db_path, codec, user_id)?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

fn resolve_notification_secret(
    codec: &SecretCodec,
    user_id: UserId,
    field: &str,
    update: &Option<Option<String>>,
    existing: Option<&str>,
) -> Result<String, BusinessRepositoryError> {
    match update {
        None => Ok(existing.unwrap_or_default().to_string()),
        Some(None) => Ok(String::new()),
        Some(Some(value)) if value.trim().is_empty() => {
            Ok(existing.unwrap_or_default().to_string())
        }
        Some(Some(value)) => codec
            .encrypt(value.trim(), &notification_context(user_id.value(), field))
            .map_err(BusinessRepositoryError::from),
    }
}
fn notification_settings_from_row(
    row: &rusqlite::Row<'_>,
    codec: &SecretCodec,
) -> rusqlite::Result<Result<NotificationSettings, BusinessRepositoryError>> {
    Ok((|| {
        let user_id = UserId(row.get(1)?);
        Ok(NotificationSettings {
            id: row.get(0)?,
            user_id,
            keywords: parse_string_list(row.get::<_, String>(2)?),
            directions: parse_string_list(row.get::<_, String>(3)?),
            selected_databases: parse_string_list(row.get::<_, String>(4)?),
            delivery_method: row.get(5)?,
            pushplus_token: codec.decrypt(
                &row.get::<_, String>(6)?,
                &notification_context(user_id.value(), "pushplus_token"),
            )?,
            pushplus_template: row.get(7)?,
            pushplus_topic: row.get(8)?,
            pushplus_channel: row.get(9)?,
            sync_to_tracking_folder: row.get::<_, i64>(10)? != 0,
            ai_base_url: row.get(11)?,
            ai_api_key: codec.decrypt(
                &row.get::<_, String>(12)?,
                &notification_context(user_id.value(), "ai_api_key"),
            )?,
            ai_model: row.get(13)?,
            ai_system_prompt: row.get(14)?,
            ai_backup_base_url: row.get(15)?,
            ai_backup_api_key: codec.decrypt(
                &row.get::<_, String>(16)?,
                &notification_context(user_id.value(), "ai_backup_api_key"),
            )?,
            ai_backup_model: row.get(17)?,
            ai_backup_system_prompt: row.get(18)?,
            ai_retry_attempts: row.get::<_, i64>(19)?.clamp(
                litradar_domain::NOTIFICATION_AI_RETRY_ATTEMPTS_MIN,
                litradar_domain::NOTIFICATION_AI_RETRY_ATTEMPTS_MAX,
            ),
            enabled: row.get::<_, i64>(20)? != 0,
            created_at: row.get(21)?,
            updated_at: row.get(22)?,
        })
    })())
}

fn notification_subscriber_from_row(
    row: &rusqlite::Row<'_>,
    codec: &SecretCodec,
) -> rusqlite::Result<Result<NotificationSubscriberInfo, BusinessRepositoryError>> {
    let user_id = row.get::<_, i64>(0)?;
    Ok((|| {
        Ok(NotificationSubscriberInfo {
            subscriber_id: user_id.to_string(),
            user_id,
            name: row.get(1)?,
            keywords: parse_string_list(row.get::<_, String>(2)?),
            directions: parse_string_list(row.get::<_, String>(3)?),
            selected_databases: parse_string_list(row.get::<_, String>(4)?),
            delivery_method: row.get(5)?,
            pushplus_token: codec.decrypt(
                &row.get::<_, String>(6)?,
                &notification_context(user_id, "pushplus_token"),
            )?,
            template: optional_trimmed(row.get::<_, String>(7)?),
            topic: optional_trimmed(row.get::<_, String>(8)?),
            channel: optional_trimmed(row.get::<_, String>(9)?),
            sync_to_tracking_folder: row.get::<_, i64>(10)? != 0,
            ai_base_url: optional_trimmed(row.get::<_, String>(11)?),
            ai_api_key: optional_trimmed(codec.decrypt(
                &row.get::<_, String>(12)?,
                &notification_context(user_id, "ai_api_key"),
            )?),
            ai_model: optional_trimmed(row.get::<_, String>(13)?),
            ai_system_prompt: optional_trimmed(row.get::<_, String>(14)?),
            ai_backup_base_url: optional_trimmed(row.get::<_, String>(15)?),
            ai_backup_api_key: optional_trimmed(codec.decrypt(
                &row.get::<_, String>(16)?,
                &notification_context(user_id, "ai_backup_api_key"),
            )?),
            ai_backup_model: optional_trimmed(row.get::<_, String>(17)?),
            ai_backup_system_prompt: optional_trimmed(row.get::<_, String>(18)?),
            ai_retry_attempts: row.get::<_, i64>(19)?.clamp(
                litradar_domain::NOTIFICATION_AI_RETRY_ATTEMPTS_MIN,
                litradar_domain::NOTIFICATION_AI_RETRY_ATTEMPTS_MAX,
            ),
            tracking_folder_id: row.get(20)?,
        })
    })())
}

fn parse_string_list(value: String) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(&value).unwrap_or_default()
}

fn optional_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn collect_nested_rows<T>(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Result<T, BusinessRepositoryError>>,
    >,
) -> Result<Vec<T>, BusinessRepositoryError> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row??);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use litradar_domain::{NotificationSettingsResponse, NotificationSettingsUpdate};
    use tempfile::tempdir;

    use super::*;
    use crate::{migrate_auth_database, SecretCodec, SecretError};

    #[test]
    fn notification_credentials_are_encrypted_masked_preserved_and_cleared_explicitly() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let user = crate::bootstrap_admin(&auth_db_path, "secret-user", "hash", "salt", 1.0)
            .expect("fixture user should bootstrap");
        let codec = SecretCodec::from_key([19_u8; 32]);
        let settings = NotificationSettingsUpdate {
            keywords: vec!["systems".to_string()],
            directions: vec!["security".to_string()],
            selected_databases: Vec::new(),
            delivery_method: "pushplus".to_string(),
            pushplus_token: Some(Some("push-secret-value".to_string())),
            pushplus_template: "markdown".to_string(),
            pushplus_topic: String::new(),
            pushplus_channel: "wechat".to_string(),
            sync_to_tracking_folder: false,
            ai_base_url: "https://ai.example/v1".to_string(),
            ai_api_key: Some(Some("primary-secret-value".to_string())),
            ai_model: "fixture-model".to_string(),
            ai_system_prompt: String::new(),
            ai_backup_base_url: "https://backup.example/v1".to_string(),
            ai_backup_api_key: Some(Some("backup-secret-value".to_string())),
            ai_backup_model: "backup-model".to_string(),
            ai_backup_system_prompt: String::new(),
            ai_retry_attempts: 3,
            enabled: true,
        };

        let stored = super::upsert_notification_settings(&auth_db_path, &codec, user.id, &settings)
            .expect("notification settings should persist");
        assert_eq!(stored.pushplus_token, "push-secret-value");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        let raw = connection
            .query_row(
                "SELECT pushplus_token, ai_api_key, ai_backup_api_key \
                 FROM notification_settings WHERE user_id = ?1",
                [user.id.value()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("encrypted settings should load");
        for ciphertext in [&raw.0, &raw.1, &raw.2] {
            assert!(ciphertext.starts_with("litradarenc:v1:"));
            assert!(!ciphertext.contains("secret-value"));
        }
        let response = NotificationSettingsResponse::from(&stored);
        let response_json = serde_json::to_string(&response).expect("response should serialize");
        assert!(response.has_pushplus_token);
        assert_eq!(response.pushplus_token_mask, "••••");
        assert!(!response_json.contains("push-secret-value"));
        assert!(!response_json.contains("litradarenc:v1:"));

        let mut preserve = settings.clone();
        preserve.pushplus_token = Some(Some("   ".to_string()));
        preserve.ai_api_key = None;
        preserve.ai_backup_api_key = None;
        let preserved =
            super::upsert_notification_settings(&auth_db_path, &codec, user.id, &preserve)
                .expect("blank and omitted secrets should preserve");
        assert_eq!(preserved.pushplus_token, "push-secret-value");
        assert_eq!(preserved.ai_api_key, "primary-secret-value");

        preserve.pushplus_token = Some(None);
        let cleared =
            super::upsert_notification_settings(&auth_db_path, &codec, user.id, &preserve)
                .expect("explicit null should clear");
        assert!(cleared.pushplus_token.is_empty());
        assert_eq!(cleared.ai_api_key, "primary-secret-value");
    }

    #[test]
    fn notification_retry_attempts_are_normalized_on_read() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let user = crate::bootstrap_admin(&auth_db_path, "retry-user", "hash", "salt", 1.0)
            .expect("fixture user should bootstrap");
        let codec = SecretCodec::from_key([23_u8; 32]);
        let settings = serde_json::from_str::<NotificationSettingsUpdate>("{}")
            .expect("default notification settings should deserialize");
        super::upsert_notification_settings(&auth_db_path, &codec, user.id, &settings)
            .expect("notification settings should persist");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");

        for (stored_attempts, expected_attempts) in [(-1_i64, 1_i64), (i64::MAX, 10_i64)] {
            connection
                .execute(
                    "UPDATE notification_settings SET ai_retry_attempts = ?1 WHERE user_id = ?2",
                    params![stored_attempts, user.id.value()],
                )
                .expect("retry attempts fixture should update");

            let loaded = super::get_notification_settings(&auth_db_path, &codec, user.id)
                .expect("notification settings should load")
                .expect("notification settings should exist");
            let subscribers = super::list_notification_subscribers(&auth_db_path, &codec)
                .expect("notification subscribers should load");
            let raw_attempts = connection
                .query_row(
                    "SELECT ai_retry_attempts FROM notification_settings WHERE user_id = ?1",
                    [user.id.value()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("raw retry attempts should load");

            assert_eq!(loaded.ai_retry_attempts, expected_attempts);
            assert_eq!(subscribers.len(), 1);
            assert_eq!(subscribers[0].ai_retry_attempts, expected_attempts);
            assert_eq!(raw_attempts, stored_attempts);
        }
    }

    #[test]
    fn scoped_notification_subscriber_loading_isolates_secret_decryption() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let target_user = crate::bootstrap_admin(&auth_db_path, "target-user", "hash", "salt", 1.0)
            .expect("target user should bootstrap");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users \
                 (username, password_hash, salt, is_admin, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 0, ?4, ?4)",
                ("unrelated-user", "hash", "salt", 2.0_f64),
            )
            .expect("unrelated user should be inserted");
        let unrelated_user_id = UserId(connection.last_insert_rowid());
        let codec = SecretCodec::from_key([29_u8; 32]);
        let settings = notification_subscriber_settings();
        super::upsert_notification_settings(&auth_db_path, &codec, target_user.id, &settings)
            .expect("target settings should persist");
        super::upsert_notification_settings(&auth_db_path, &codec, unrelated_user_id, &settings)
            .expect("unrelated settings should persist");
        connection
            .execute(
                "UPDATE notification_settings SET ai_api_key = 'litradarenc:v1:bad' \
                 WHERE user_id = ?1",
                [unrelated_user_id.value()],
            )
            .expect("unrelated ciphertext should be corrupted");

        let target = super::get_notification_subscriber(&auth_db_path, &codec, target_user.id)
            .expect("healthy target lookup should succeed")
            .expect("healthy target should exist");
        assert_eq!(target.user_id, target_user.id.value());
        assert_eq!(target.pushplus_token, "target-push-token");
        assert_eq!(target.ai_api_key.as_deref(), Some("target-ai-key"));
        assert_eq!(
            target.ai_backup_api_key.as_deref(),
            Some("target-backup-key")
        );
        assert_eq!(target.ai_retry_attempts, 3);

        assert!(
            super::get_notification_subscriber(&auth_db_path, &codec, UserId(i64::MAX))
                .expect("missing target lookup should succeed")
                .is_none()
        );
        connection
            .execute(
                "UPDATE notification_settings SET enabled = 0 WHERE user_id = ?1",
                [target_user.id.value()],
            )
            .expect("target settings should be disabled");
        assert!(
            super::get_notification_subscriber(&auth_db_path, &codec, target_user.id)
                .expect("disabled target lookup should succeed")
                .is_none()
        );
        connection
            .execute(
                "UPDATE notification_settings SET enabled = 1 WHERE user_id = ?1",
                [target_user.id.value()],
            )
            .expect("target settings should be enabled");

        let target_error =
            super::get_notification_subscriber(&auth_db_path, &codec, unrelated_user_id)
                .expect_err("corrupt target should fail closed");
        assert!(matches!(
            &target_error,
            BusinessRepositoryError::Secret(SecretError::Authentication)
        ));
        assert_eq!(
            target_error.to_string(),
            "Stored secret authentication failed"
        );
        assert!(!target_error.to_string().contains("litradarenc:v1:bad"));

        let all_subscribers_error = super::list_notification_subscribers(&auth_db_path, &codec)
            .expect_err("all-subscriber loading should remain fail closed");
        assert!(matches!(
            all_subscribers_error,
            BusinessRepositoryError::Secret(SecretError::Authentication)
        ));
    }

    fn notification_subscriber_settings() -> NotificationSettingsUpdate {
        NotificationSettingsUpdate {
            keywords: vec!["systems".to_string()],
            directions: vec!["security".to_string()],
            selected_databases: Vec::new(),
            delivery_method: "pushplus".to_string(),
            pushplus_token: Some(Some("target-push-token".to_string())),
            pushplus_template: "markdown".to_string(),
            pushplus_topic: String::new(),
            pushplus_channel: "wechat".to_string(),
            sync_to_tracking_folder: false,
            ai_base_url: "https://ai.example/v1".to_string(),
            ai_api_key: Some(Some("target-ai-key".to_string())),
            ai_model: "fixture-model".to_string(),
            ai_system_prompt: String::new(),
            ai_backup_base_url: "https://backup.example/v1".to_string(),
            ai_backup_api_key: Some(Some("target-backup-key".to_string())),
            ai_backup_model: "backup-model".to_string(),
            ai_backup_system_prompt: String::new(),
            ai_retry_attempts: 3,
            enabled: true,
        }
    }
}
