//! Zhejiang Library CNKI session repository operations.

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::{CnkiSessionStatusResponse, UserId};
use rusqlite::{params, OptionalExtension};
use serde_json::Value as JsonValue;

use crate::auth::{open_auth_connection, AuthRepositoryError};

/// Repository errors for CNKI session operations.
#[derive(Debug)]
pub enum CnkiRepositoryError {
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// JSON serialization or parsing failed.
    Json(serde_json::Error),
    /// Auth database setup failed.
    Auth(AuthRepositoryError),
}

impl fmt::Display for CnkiRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::Auth(error) => write!(formatter, "{error}"),
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

/// Return the safe CNKI session status for one user.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// Safe session status.
pub fn get_cnki_session_status(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<CnkiSessionStatusResponse, CnkiRepositoryError> {
    let row = get_cnki_session_row(auth_db_path, user_id)?;
    Ok(summarize_cnki_session(row.as_ref(), current_unix_time()))
}

/// Store a CNKI session row and return its safe status.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
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
    user_id: UserId,
    session_data: &JsonValue,
    status: &str,
    qr_uuid: Option<&str>,
) -> Result<CnkiSessionStatusResponse, CnkiRepositoryError> {
    let now = current_unix_time();
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
    let session_json = serde_json::to_string(session_data)?;
    let connection = open_auth_connection(auth_db_path)?;
    let created_at = connection
        .query_row(
            "SELECT created_at FROM cnki_sessions WHERE user_id = ?1",
            [user_id.value()],
            |row| row.get::<_, f64>(0),
        )
        .optional()?
        .unwrap_or(now);
    connection.execute(
        "INSERT INTO cnki_sessions \
         (user_id, session_json, qr_uuid, status, token_expires_at, created_at, updated_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL) \
         ON CONFLICT(user_id) DO UPDATE SET \
         session_json = excluded.session_json, qr_uuid = excluded.qr_uuid, \
         status = excluded.status, token_expires_at = excluded.token_expires_at, \
         updated_at = excluded.updated_at",
        params![
            user_id.value(),
            session_json,
            resolved_qr_uuid,
            status,
            token_expires_at,
            created_at,
            now,
        ],
    )?;
    let row = CnkiSessionRow {
        session_json,
        qr_uuid: resolved_qr_uuid,
        status: status.to_string(),
        updated_at: Some(now),
        last_used_at: None,
    };
    Ok(summarize_cnki_session(Some(&row), now))
}

/// Delete one user's CNKI session.
///
/// # Arguments
///
/// * `auth_db_path` - Auth database path.
/// * `user_id` - User identifier.
///
/// # Returns
///
/// True when a row was deleted.
pub fn delete_cnki_session(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<bool, CnkiRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    let count = connection.execute(
        "DELETE FROM cnki_sessions WHERE user_id = ?1",
        [user_id.value()],
    )?;
    Ok(count > 0)
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
        "UPDATE cnki_sessions SET last_used_at = ?1 WHERE user_id = ?2",
        params![current_unix_time(), user_id.value()],
    )?;
    Ok(count > 0)
}

fn get_cnki_session_row(
    auth_db_path: impl AsRef<Path>,
    user_id: UserId,
) -> Result<Option<CnkiSessionRow>, CnkiRepositoryError> {
    let connection = open_auth_connection(auth_db_path)?;
    connection
        .query_row(
            "SELECT session_json, qr_uuid, status, updated_at, last_used_at \
             FROM cnki_sessions WHERE user_id = ?1",
            [user_id.value()],
            |row| {
                Ok(CnkiSessionRow {
                    session_json: row.get(0)?,
                    qr_uuid: row.get(1)?,
                    status: row.get(2)?,
                    updated_at: row.get(3)?,
                    last_used_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(CnkiRepositoryError::from)
}

fn summarize_cnki_session(row: Option<&CnkiSessionRow>, now: f64) -> CnkiSessionStatusResponse {
    let Some(row) = row else {
        return CnkiSessionStatusResponse {
            configured: false,
            status: "empty".to_string(),
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
    let status = if has_bff_user_token {
        if expires_at.is_some_and(|value| value <= now) {
            "expired".to_string()
        } else {
            "active".to_string()
        }
    } else if nonempty(&row.qr_uuid).is_some() {
        "waiting_scan".to_string()
    } else {
        nonempty(&row.status).unwrap_or("empty").to_string()
    };
    CnkiSessionStatusResponse {
        configured: status != "empty",
        status,
        has_bff_user_token,
        expires_at,
        seconds_remaining,
        cookie_names: cookie_names(&session_data),
        updated_at: row.updated_at,
        last_used_at: row.last_used_at,
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

#[derive(Debug, Clone)]
struct CnkiSessionRow {
    session_json: String,
    qr_uuid: String,
    status: String,
    updated_at: Option<f64>,
    last_used_at: Option<f64>,
}
