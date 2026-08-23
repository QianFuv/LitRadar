//! Zhejiang Library CNKI session repository operations.

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_domain::{CnkiSessionStatusResponse, CnkiStatus, UserId};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value as JsonValue;

use crate::auth::{open_auth_connection, AuthRepositoryError};
use crate::secrets::cnki_context;
use crate::{SecretCodec, SecretError};

/// Persisted CNKI session state for one user.
#[derive(Clone, PartialEq)]
pub struct CnkiSessionData {
    /// Raw session JSON payload.
    pub session_data: JsonValue,
    /// Stored QR UUID.
    pub qr_uuid: String,
    /// Stored status label.
    pub status: CnkiStatus,
    /// Monotonic session operation generation.
    pub generation: i64,
}

impl fmt::Debug for CnkiSessionData {
    /// Format session metadata without exposing decrypted session state.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CnkiSessionData")
            .field("session_data", &"[REDACTED]")
            .field("qr_uuid", &self.qr_uuid)
            .field("status", &self.status)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Repository errors for CNKI session operations.
#[derive(Debug)]
pub enum CnkiRepositoryError {
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// JSON serialization or parsing failed.
    Json(serde_json::Error),
    /// Auth database setup failed.
    Auth(AuthRepositoryError),
    /// Secret encryption or decryption failed.
    Secret(SecretError),
}

impl fmt::Display for CnkiRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Auth(error) => write!(formatter, "{error}"),
            Self::Secret(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for CnkiRepositoryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Auth(error) => Some(error),
            Self::Secret(error) => Some(error),
        }
    }
}

impl From<rusqlite::Error> for CnkiRepositoryError {
    /// Convert SQLite errors into repository errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for CnkiRepositoryError {
    /// Convert JSON errors into repository errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<AuthRepositoryError> for CnkiRepositoryError {
    /// Convert auth repository errors into CNKI repository errors.
    fn from(error: AuthRepositoryError) -> Self {
        Self::Auth(error)
    }
}

impl From<SecretError> for CnkiRepositoryError {
    /// Convert secret errors into CNKI repository errors.
    fn from(error: SecretError) -> Self {
        Self::Secret(error)
    }
}

/// Return the safe CNKI session status for one user.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Safe session status.
pub fn get_cnki_session_status(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<CnkiSessionStatusResponse, CnkiRepositoryError> {
    let row = get_cnki_session_row(auth_db_path, codec, user_id)?;
    Ok(summarize_cnki_session(row.as_ref(), current_unix_time()))
}

/// Return persisted CNKI session data for one user.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Raw session data, QR UUID, and status when present.
pub fn get_cnki_session_data(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<Option<CnkiSessionData>, CnkiRepositoryError> {
    let row = get_cnki_session_row(auth_db_path, codec, user_id)?;
    row.map(|row| {
        Ok(CnkiSessionData {
            session_data: serde_json::from_str(&row.session_json)?,
            qr_uuid: row.qr_uuid,
            status: CnkiStatus::from(row.status),
            generation: row.generation,
        })
    })
    .transpose()
}

/// Return effective active CNKI session data for one user.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Raw session data only when the effective status is active.
pub fn get_active_cnki_session_data(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<Option<CnkiSessionData>, CnkiRepositoryError> {
    let row = get_cnki_session_row(auth_db_path, codec, user_id)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let session_data = serde_json::from_str(&row.session_json)?;
    let status = effective_cnki_status(&session_data, &row, current_unix_time());
    if status != CnkiStatus::Active {
        return Ok(None);
    }
    Ok(Some(CnkiSessionData {
        session_data,
        qr_uuid: row.qr_uuid,
        status,
        generation: row.generation,
    }))
}

/// Reserve a new generation before starting a CNKI network operation.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Monotonic generation reserved for one completion attempt.
pub fn reserve_cnki_session_operation(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<i64, CnkiRepositoryError> {
    let now = current_unix_time();
    let empty_session_json = codec.encrypt("{}", &cnki_context(user_id.value()))?;
    let connection = open_auth_connection(auth_db_path)?;
    connection
        .query_row(
            r#"
            INSERT INTO cnki_sessions (
                user_id, session_json, qr_uuid, status, token_expires_at,
                created_at, updated_at, last_used_at, generation
            )
            VALUES (?1, ?2, '', 'empty', NULL, ?3, ?3, NULL, 1)
            ON CONFLICT(user_id) DO UPDATE SET
                qr_uuid = '',
                generation = cnki_sessions.generation + 1
            RETURNING generation
            "#,
            params![user_id.value(), empty_session_json, now],
            |row| row.get(0),
        )
        .map_err(CnkiRepositoryError::from)
}

/// Store a CNKI completion only when its observed generation is still current.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
/// * `expected_generation` - Generation reserved or loaded before network work.
/// * `expected_qr_uuid` - Optional QR UUID that must still identify the row.
/// * `session_data` - JSON session payload.
/// * `status` - Persisted status label.
/// * `qr_uuid` - Optional QR UUID override.
///
/// # Returns
///
/// Safe status on success, or None when a newer operation superseded this completion.
#[allow(clippy::too_many_arguments)]
pub fn compare_and_swap_cnki_session(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
    expected_generation: i64,
    expected_qr_uuid: Option<&str>,
    session_data: &JsonValue,
    status: &CnkiStatus,
    qr_uuid: Option<&str>,
) -> Result<Option<CnkiSessionStatusResponse>, CnkiRepositoryError> {
    let now = current_unix_time();
    let (encrypted_session_json, token_expires_at, mut row) =
        prepare_cnki_session_row(codec, user_id, session_data, status, qr_uuid, now)?;
    let connection = open_auth_connection(auth_db_path)?;
    let updated = connection.execute(
        r#"
        UPDATE cnki_sessions
        SET session_json = ?1,
            qr_uuid = ?2,
            status = ?3,
            token_expires_at = ?4,
            updated_at = ?5,
            generation = generation + 1
        WHERE user_id = ?6
          AND generation = ?7
          AND (?8 IS NULL OR qr_uuid = ?8)
        "#,
        params![
            encrypted_session_json,
            row.qr_uuid,
            row.status,
            token_expires_at,
            now,
            user_id.value(),
            expected_generation,
            expected_qr_uuid,
        ],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    row.generation = expected_generation.saturating_add(1);
    Ok(Some(summarize_cnki_session(Some(&row), now)))
}

/// Store a CNKI session as a new atomic generation and return its safe status.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
/// * `session_data` - JSON session payload.
/// * `status` - Persisted status label.
/// * `qr_uuid` - Optional QR UUID override.
///
/// # Returns
///
/// Safe session status after upsert.
pub fn upsert_cnki_session(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
    session_data: &JsonValue,
    status: &CnkiStatus,
    qr_uuid: Option<&str>,
) -> Result<CnkiSessionStatusResponse, CnkiRepositoryError> {
    let now = current_unix_time();
    let (session_json, token_expires_at, mut row) =
        prepare_cnki_session_row(codec, user_id, session_data, status, qr_uuid, now)?;
    let connection = open_auth_connection(auth_db_path)?;
    let generation = connection.query_row(
        r#"
        INSERT INTO cnki_sessions (
            user_id, session_json, qr_uuid, status, token_expires_at,
            created_at, updated_at, last_used_at, generation
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, NULL, 1)
        ON CONFLICT(user_id) DO UPDATE SET
            session_json = excluded.session_json,
            qr_uuid = excluded.qr_uuid,
            status = excluded.status,
            token_expires_at = excluded.token_expires_at,
            updated_at = excluded.updated_at,
            generation = cnki_sessions.generation + 1
        RETURNING generation
        "#,
        params![
            user_id.value(),
            session_json,
            row.qr_uuid,
            row.status,
            token_expires_at,
            now,
        ],
        |result| result.get(0),
    )?;
    row.generation = generation;
    Ok(summarize_cnki_session(Some(&row), now))
}

/// Clear one user's CNKI session while fencing every older completion.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `codec` - Deployment secret codec.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// True when a visible session existed before the tombstone was stored.
pub fn delete_cnki_session(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<bool, CnkiRepositoryError> {
    let now = current_unix_time();
    let empty_session_json = codec.encrypt("{}", &cnki_context(user_id.value()))?;
    let mut connection = open_auth_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let did_exist = transaction
        .query_row(
            "SELECT status <> 'empty' FROM cnki_sessions WHERE user_id = ?1",
            [user_id.value()],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    transaction.execute(
        r#"
        INSERT INTO cnki_sessions (
            user_id, session_json, qr_uuid, status, token_expires_at,
            created_at, updated_at, last_used_at, generation
        )
        VALUES (?1, ?2, '', 'empty', NULL, ?3, ?3, NULL, 1)
        ON CONFLICT(user_id) DO UPDATE SET
            session_json = excluded.session_json,
            qr_uuid = '',
            status = 'empty',
            token_expires_at = NULL,
            updated_at = excluded.updated_at,
            last_used_at = NULL,
            generation = cnki_sessions.generation + 1
        "#,
        params![user_id.value(), empty_session_json, now],
    )?;
    transaction.commit()?;
    Ok(did_exist)
}

/// Record that a user's CNKI session was used.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// True when a row was updated.
pub fn touch_cnki_session_used(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<bool, CnkiRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    let count = connection.execute(
        "UPDATE cnki_sessions SET last_used_at = ?1 WHERE user_id = ?2 AND status <> 'empty'",
        params![current_unix_time(), user_id.value()],
    )?;
    Ok(count > 0)
}

fn get_cnki_session_row(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<Option<CnkiSessionRow>, CnkiRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    let row = connection
        .query_row(
            "SELECT session_json, qr_uuid, status, updated_at, last_used_at, generation \
             FROM cnki_sessions WHERE user_id = ?1 AND status <> 'empty'",
            [user_id.value()],
            |row| {
                Ok(CnkiSessionRow {
                    session_json: row.get(0)?,
                    qr_uuid: row.get(1)?,
                    status: row.get(2)?,
                    updated_at: row.get(3)?,
                    last_used_at: row.get(4)?,
                    generation: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(CnkiRepositoryError::from)?;
    row.map(|mut row| {
        row.session_json = codec.decrypt(&row.session_json, &cnki_context(user_id.value()))?;
        Ok(row)
    })
    .transpose()
}

fn summarize_cnki_session(row: Option<&CnkiSessionRow>, now: f64) -> CnkiSessionStatusResponse {
    let Some(row) = row else {
        return CnkiSessionStatusResponse {
            configured: false,
            status: CnkiStatus::Empty,
            has_bff_user_token: false,
            expires_at: None,
            seconds_remaining: None,
            cookie_names: Vec::new(),
            updated_at: None,
            last_used_at: None,
        };
    };
    let session_data = decode_session_json(&row.session_json);
    let token = session_data
        .get("bff_user_token")
        .and_then(JsonValue::as_str)
        .and_then(nonempty);
    let expires_at = token.and_then(parse_jwt_expiration);
    let has_bff_user_token = token.is_some();
    let seconds_remaining = expires_at.map(|value| (value - now).max(0.0).floor() as i64);
    let status = effective_cnki_status(&session_data, row, now);
    CnkiSessionStatusResponse {
        configured: status != CnkiStatus::Empty,
        status,
        has_bff_user_token,
        expires_at,
        seconds_remaining,
        cookie_names: cookie_names(&session_data),
        updated_at: row.updated_at,
        last_used_at: row.last_used_at,
    }
}

fn effective_cnki_status(session_data: &JsonValue, row: &CnkiSessionRow, now: f64) -> CnkiStatus {
    let token = session_data
        .get("bff_user_token")
        .and_then(JsonValue::as_str)
        .and_then(nonempty);
    let expires_at = token.and_then(parse_jwt_expiration);
    if token.is_some() {
        if expires_at.is_some_and(|value| value <= now) {
            CnkiStatus::Expired
        } else {
            CnkiStatus::Active
        }
    } else if nonempty(&row.qr_uuid).is_some() {
        CnkiStatus::WaitingScan
    } else {
        CnkiStatus::from(nonempty(&row.status).unwrap_or("empty"))
    }
}

fn decode_session_json(value: &str) -> JsonValue {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::json!({}))
}

fn cookie_names(session_data: &JsonValue) -> Vec<String> {
    session_data
        .get("cookies")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(JsonValue::as_str))
                .filter_map(nonempty)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_jwt_expiration(token: &str) -> Option<f64> {
    let payload = token.split('.').nth(1)?;
    let bytes = decode_base64_url(payload)?;
    let value = serde_json::from_slice::<JsonValue>(&bytes).ok()?;
    value.get("exp").and_then(JsonValue::as_f64)
}

fn decode_base64_url(value: &str) -> Option<Vec<u8>> {
    let mut bit_buffer = 0_u32;
    let mut bit_count = 0_u8;
    let mut output = Vec::new();
    for byte in value.bytes().filter(|byte| *byte != b'=') {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;
        bit_buffer = (bit_buffer << 6) | digit;
        bit_count += 6;
        while bit_count >= 8 {
            bit_count -= 8;
            output.push(((bit_buffer >> bit_count) & 0xff) as u8);
        }
    }
    Some(output)
}

fn current_unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

fn nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn prepare_cnki_session_row(
    codec: &SecretCodec,
    user_id: UserId,
    session_data: &JsonValue,
    status: &CnkiStatus,
    qr_uuid: Option<&str>,
    now: f64,
) -> Result<(String, Option<f64>, CnkiSessionRow), CnkiRepositoryError> {
    let token_expires_at = session_data
        .get("bff_user_token")
        .and_then(JsonValue::as_str)
        .and_then(parse_jwt_expiration);
    let resolved_qr_uuid = qr_uuid
        .and_then(nonempty)
        .map(str::to_string)
        .or_else(|| {
            session_data
                .get("qr_uuid")
                .and_then(JsonValue::as_str)
                .and_then(nonempty)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let plaintext_session_json = serde_json::to_string(session_data)?;
    let encrypted_session_json =
        codec.encrypt(&plaintext_session_json, &cnki_context(user_id.value()))?;
    Ok((
        encrypted_session_json,
        token_expires_at,
        CnkiSessionRow {
            session_json: plaintext_session_json,
            qr_uuid: resolved_qr_uuid,
            status: status.as_str().to_string(),
            updated_at: Some(now),
            last_used_at: None,
            generation: 0,
        },
    ))
}

#[derive(Debug, Clone)]
struct CnkiSessionRow {
    session_json: String,
    qr_uuid: String,
    status: String,
    updated_at: Option<f64>,
    last_used_at: Option<f64>,
    generation: i64,
}

#[cfg(test)]
mod tests {
    use litradar_domain::{CnkiStatus, UserId};
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        compare_and_swap_cnki_session, delete_cnki_session, get_active_cnki_session_data,
        get_cnki_session_data, get_cnki_session_status, reserve_cnki_session_operation,
        touch_cnki_session_used, upsert_cnki_session,
    };
    use crate::auth::initialize_auth_database;
    use crate::SecretCodec;

    #[test]
    fn cnki_session_data_preserves_raw_state_but_status_hides_secrets() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let user_id = UserId(7);
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users (id, username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                (user_id.value(), "cnki-user", "hash", "salt"),
            )
            .expect("user fixture should insert");
        let session_data = json!({
            "bff_user_token": "header.payload.signature",
            "qr_uuid": "qr-fixture",
            "cookies": [
                {"name": "userToken", "value": "SECRET_TOKEN_COOKIE"},
                {"name": "vpn358_sid", "value": "SECRET_VPN_COOKIE"}
            ],
        });
        let codec = SecretCodec::from_key([9_u8; 32]);

        upsert_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            &session_data,
            &CnkiStatus::Active,
            Some("qr-fixture"),
        )
        .expect("session should upsert");
        let raw_session = get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("session data should load")
            .expect("session data should exist");
        let safe_status = get_cnki_session_status(&auth_db_path, &codec, user_id)
            .expect("session status should load");
        let safe_json = serde_json::to_string(&safe_status).expect("status should serialize");

        assert_eq!(raw_session.qr_uuid, "qr-fixture");
        assert_eq!(raw_session.generation, 1);
        assert_eq!(
            raw_session.session_data["cookies"][0]["value"],
            "SECRET_TOKEN_COOKIE"
        );
        assert_eq!(safe_status.cookie_names, ["userToken", "vpn358_sid"]);
        assert!(!safe_json.contains("SECRET_TOKEN_COOKIE"));
        assert!(!safe_json.contains("SECRET_VPN_COOKIE"));
        let stored: String = Connection::open(&auth_db_path)
            .expect("auth database should reopen")
            .query_row(
                "SELECT session_json FROM cnki_sessions WHERE user_id = ?1",
                [user_id.value()],
                |row| row.get(0),
            )
            .expect("stored session should load");
        assert!(stored.starts_with("litradarenc:v1:"));
        assert!(!stored.contains("SECRET_TOKEN_COOKIE"));
    }

    #[test]
    fn active_cnki_session_loader_applies_effective_expiration() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let user_id = UserId(11);
        Connection::open(&auth_db_path)
            .expect("auth database should open")
            .execute(
                "INSERT INTO users (id, username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                (user_id.value(), "cnki-expiry-user", "hash", "salt"),
            )
            .expect("user fixture should insert");
        let codec = SecretCodec::from_key([13_u8; 32]);

        upsert_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            &json!({"bff_user_token": "header.eyJleHAiOjF9.signature"}),
            &CnkiStatus::Active,
            None,
        )
        .expect("expired session should upsert");
        assert_eq!(
            get_cnki_session_status(&auth_db_path, &codec, user_id)
                .expect("expired status should load")
                .status,
            CnkiStatus::Expired
        );
        assert!(get_active_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("expired active session lookup should succeed")
            .is_none());

        upsert_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            &json!({"bff_user_token": "header.eyJleHAiOjQxMDI0NDQ4MDB9.signature"}),
            &CnkiStatus::Active,
            None,
        )
        .expect("future session should upsert");
        assert_eq!(
            get_cnki_session_status(&auth_db_path, &codec, user_id)
                .expect("future status should load")
                .status,
            CnkiStatus::Active
        );
        assert!(get_active_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("future active session lookup should succeed")
            .is_some());

        upsert_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            &json!({"bff_user_token": "legacy-token-without-exp"}),
            &CnkiStatus::Active,
            None,
        )
        .expect("legacy session should upsert");
        assert_eq!(
            get_cnki_session_status(&auth_db_path, &codec, user_id)
                .expect("legacy status should load")
                .status,
            CnkiStatus::Active
        );
        assert!(get_active_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("legacy active session lookup should succeed")
            .is_some());
    }

    #[test]
    fn cnki_generations_reject_superseded_start_and_cleared_poll_completions() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let user_id = UserId(8);
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users (id, username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                (user_id.value(), "cnki-generation-user", "hash", "salt"),
            )
            .expect("user fixture should insert");
        drop(connection);
        let codec = SecretCodec::from_key([10_u8; 32]);

        let start_a = reserve_cnki_session_operation(&auth_db_path, &codec, user_id)
            .expect("first start should reserve");
        let start_b = reserve_cnki_session_operation(&auth_db_path, &codec, user_id)
            .expect("second start should reserve");
        assert!(start_b > start_a);
        assert!(compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            start_a,
            None,
            &json!({"qr_uuid": "qr-a", "secret": "STALE_START_SECRET"}),
            &CnkiStatus::WaitingScan,
            Some("qr-a"),
        )
        .expect("stale start completion should run")
        .is_none());
        assert!(get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("tombstone should be hidden")
            .is_none());

        let stored_start = compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            start_b,
            None,
            &json!({"qr_uuid": "qr-b", "cookies": []}),
            &CnkiStatus::WaitingScan,
            Some("qr-b"),
        )
        .expect("current start completion should run")
        .expect("current start completion should store");
        assert_eq!(stored_start.status, CnkiStatus::WaitingScan);
        let polling = get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("polling session should load")
            .expect("polling session should exist");
        assert_eq!(polling.qr_uuid, "qr-b");
        assert!(polling.generation > start_b);

        assert!(delete_cnki_session(&auth_db_path, &codec, user_id)
            .expect("clear should commit a tombstone"));
        assert!(compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            polling.generation,
            Some(&polling.qr_uuid),
            &json!({
                "qr_uuid": "qr-b",
                "bff_user_token": "STALE_POLL_TOKEN",
                "cookies": [{"name": "userToken", "value": "STALE_POLL_COOKIE"}]
            }),
            &CnkiStatus::Active,
            Some("qr-b"),
        )
        .expect("stale poll completion should run")
        .is_none());
        assert!(get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("cleared session should load")
            .is_none());
        let status = get_cnki_session_status(&auth_db_path, &codec, user_id)
            .expect("cleared status should load");
        assert_eq!(status.status, CnkiStatus::Empty);
        assert!(!status.configured);
        assert!(!touch_cnki_session_used(&auth_db_path, user_id)
            .expect("tombstone touch should be a no-op"));

        let (stored_status, stored_generation, stored_session): (String, i64, String) =
            Connection::open(&auth_db_path)
                .expect("auth database should reopen")
                .query_row(
                    "SELECT status, generation, session_json FROM cnki_sessions WHERE user_id = ?1",
                    [user_id.value()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("tombstone should remain");
        assert_eq!(stored_status, "empty");
        assert!(stored_generation > polling.generation);
        assert!(stored_session.starts_with("litradarenc:v1:"));
        assert_eq!(
            codec
                .decrypt(
                    &stored_session,
                    &crate::secrets::cnki_context(user_id.value())
                )
                .expect("tombstone should decrypt"),
            "{}"
        );
    }

    #[test]
    fn cnki_poll_completion_requires_the_loaded_qr_uuid() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let user_id = UserId(9);
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users (id, username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                (user_id.value(), "cnki-qr-user", "hash", "salt"),
            )
            .expect("user fixture should insert");
        drop(connection);
        let codec = SecretCodec::from_key([11_u8; 32]);
        let generation = reserve_cnki_session_operation(&auth_db_path, &codec, user_id)
            .expect("start should reserve");
        compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            generation,
            None,
            &json!({"qr_uuid": "qr-current"}),
            &CnkiStatus::WaitingScan,
            Some("qr-current"),
        )
        .expect("start completion should run")
        .expect("start completion should store");
        let current = get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("session should load")
            .expect("session should exist");

        assert!(compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            current.generation,
            Some("qr-stale"),
            &json!({"qr_uuid": "qr-stale", "bff_user_token": "STALE_QR_TOKEN"}),
            &CnkiStatus::Active,
            Some("qr-stale"),
        )
        .expect("mismatched QR completion should run")
        .is_none());
        assert_eq!(
            get_cnki_session_data(&auth_db_path, &codec, user_id)
                .expect("session should reload")
                .expect("session should remain"),
            current
        );
    }

    #[test]
    fn cnki_reservation_invalidates_old_qr_without_clearing_active_session() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        initialize_auth_database(&auth_db_path).expect("auth database should initialize");
        let user_id = UserId(10);
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO users (id, username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                (user_id.value(), "cnki-reserve-user", "hash", "salt"),
            )
            .expect("user fixture should insert");
        drop(connection);
        let codec = SecretCodec::from_key([12_u8; 32]);

        let start_a = reserve_cnki_session_operation(&auth_db_path, &codec, user_id)
            .expect("first start should reserve");
        compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            start_a,
            None,
            &json!({"qr_uuid": "qr-a", "cookies": []}),
            &CnkiStatus::WaitingScan,
            Some("qr-a"),
        )
        .expect("first start completion should run")
        .expect("first QR should store");
        let poll_a = get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("first QR should load")
            .expect("first QR should exist");

        let start_b = reserve_cnki_session_operation(&auth_db_path, &codec, user_id)
            .expect("replacement start should reserve");
        let reserved = get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("reserved row should load")
            .expect("existing session material should remain");
        assert_eq!(reserved.generation, start_b);
        assert!(reserved.qr_uuid.is_empty());
        assert_eq!(reserved.session_data["qr_uuid"], "qr-a");
        assert!(compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            poll_a.generation,
            Some(&poll_a.qr_uuid),
            &json!({"qr_uuid": "qr-a", "bff_user_token": "STALE_POLL_TOKEN"}),
            &CnkiStatus::Active,
            Some("qr-a"),
        )
        .expect("stale poll completion should run")
        .is_none());

        compare_and_swap_cnki_session(
            &auth_db_path,
            &codec,
            user_id,
            start_b,
            None,
            &json!({"qr_uuid": "qr-b", "bff_user_token": "ACTIVE_TOKEN"}),
            &CnkiStatus::Active,
            Some("qr-b"),
        )
        .expect("replacement completion should run")
        .expect("replacement completion should store");
        let start_c = reserve_cnki_session_operation(&auth_db_path, &codec, user_id)
            .expect("another start should reserve");
        let active = get_cnki_session_data(&auth_db_path, &codec, user_id)
            .expect("active session should load")
            .expect("active session should remain while start is pending");
        assert_eq!(active.generation, start_c);
        assert!(active.qr_uuid.is_empty());
        assert_eq!(active.status, CnkiStatus::Active);
        assert_eq!(active.session_data["bff_user_token"], "ACTIVE_TOKEN");
    }
}
