//! Zhejiang Library CNKI session route handlers.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use ps_domain::{
    CnkiLoginPollRequest, CnkiLoginPollResponse, CnkiLoginStartResponse, CnkiSessionStatusResponse,
};
use serde_json::json;

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

const REPLAY_MODE_ENV: &str = "PAPER_SCANNER_CNKI_REPLAY_MODE";
const REPLAY_START_SUCCESS: &str = "start_success";
const REPLAY_POLL_SUCCESS: &str = "poll_success";
const REPLAY_TIMEOUT: &str = "timeout";
const REPLAY_WARMUP_FAILURE: &str = "warmup_failure";
const REPLAY_START_FAILURE: &str = "start_failure";
const DEFAULT_QR_UUID: &str = "qr-rust-offline";
const DEFAULT_QR_STATUS: &str = "WAITING_SCAN";
const DEFAULT_QR_CODE: &str = "https://qr.test/qr-rust-offline.png";

/// Return the current user's CNKI session status.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Safe CNKI session status.
pub(crate) async fn get_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<CnkiSessionStatusResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let status =
        ps_storage::get_cnki_session_status(state.storage_config().auth_db_path(), user.id)
            .map_err(map_cnki_error)?;
    Ok(Json(status))
}

/// Start a replayed QR login session.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// QR login challenge and safe session status.
pub(crate) async fn start_login(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<CnkiLoginStartResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    match replay_mode().as_deref() {
        Some(
            REPLAY_START_SUCCESS | REPLAY_POLL_SUCCESS | REPLAY_TIMEOUT | REPLAY_WARMUP_FAILURE,
        ) => {
            let session_data = json!({
                "qr_uuid": DEFAULT_QR_UUID,
                "cookies": [],
            });
            let session = ps_storage::upsert_cnki_session(
                state.storage_config().auth_db_path(),
                user.id,
                &session_data,
                "waiting_scan",
                Some(DEFAULT_QR_UUID),
            )
            .map_err(map_cnki_error)?;
            Ok(Json(CnkiLoginStartResponse {
                uuid: DEFAULT_QR_UUID.to_string(),
                status: DEFAULT_QR_STATUS.to_string(),
                qr_code: DEFAULT_QR_CODE.to_string(),
                session,
            }))
        }
        Some(REPLAY_START_FAILURE) | Some(_) | None => Err(cnki_json_error(
            StatusCode::BAD_GATEWAY,
            "cnki_login_start_failed",
            "login",
            "CNKI login start replay is not configured",
        )),
    }
}

/// Poll an offline-replay QR login session.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `body` - Polling parameters.
///
/// # Returns
///
/// Polling result and safe session status.
pub(crate) async fn poll_login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CnkiLoginPollRequest>,
) -> Result<Json<CnkiLoginPollResponse>, ApiError> {
    validate_poll_request(&body)?;
    let (user, _) = require_current_user(&state, &headers)?;
    let current =
        ps_storage::get_cnki_session_status(state.storage_config().auth_db_path(), user.id)
            .map_err(map_cnki_error)?;
    if !current.configured || current.status == "empty" {
        return Err(cnki_json_error(
            StatusCode::BAD_REQUEST,
            "cnki_login_not_started",
            "login",
            "CNKI QR login has not been started",
        ));
    }
    match replay_mode().as_deref() {
        Some(REPLAY_POLL_SUCCESS) => {
            let token = build_unsigned_jwt((current_unix_time() + 3600.0).floor() as i64);
            let session_data = json!({
                "bff_user_token": token,
                "qr_uuid": DEFAULT_QR_UUID,
                "cookies": [
                    {"name": "userToken", "value": "SECRET_COOKIE_VALUE"},
                    {"name": "vpn358_sid", "value": "SECRET_VPN_VALUE"}
                ],
                "final_zyproxy_url": "https://cnki.elib.test/kns55/"
            });
            let session = ps_storage::upsert_cnki_session(
                state.storage_config().auth_db_path(),
                user.id,
                &session_data,
                "active",
                Some(DEFAULT_QR_UUID),
            )
            .map_err(map_cnki_error)?;
            Ok(Json(CnkiLoginPollResponse {
                status: "COMPLETE".to_string(),
                session,
            }))
        }
        Some(REPLAY_WARMUP_FAILURE) => Err(cnki_json_error(
            StatusCode::BAD_GATEWAY,
            "cnki_warmup_failed",
            "warmup",
            "Share warm-up failed",
        )),
        Some(REPLAY_TIMEOUT)
        | Some(REPLAY_START_SUCCESS)
        | Some(REPLAY_START_FAILURE)
        | Some(_)
        | None => Err(cnki_json_error(
            StatusCode::REQUEST_TIMEOUT,
            "cnki_login_timeout",
            "login",
            &format!(
                "Timed out waiting for QR scan after {} seconds.",
                body.timeout_seconds
            ),
        )),
    }
}

/// Clear the current user's CNKI session.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Empty safe CNKI session status.
pub(crate) async fn clear_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<CnkiSessionStatusResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    ps_storage::delete_cnki_session(state.storage_config().auth_db_path(), user.id)
        .map_err(map_cnki_error)?;
    let status =
        ps_storage::get_cnki_session_status(state.storage_config().auth_db_path(), user.id)
            .map_err(map_cnki_error)?;
    Ok(Json(status))
}

fn validate_poll_request(body: &CnkiLoginPollRequest) -> Result<(), ApiError> {
    if !(1..=600).contains(&body.timeout_seconds) {
        return Err(ApiError::bad_request(
            "timeout_seconds must be between 1 and 600",
        ));
    }
    if !(0.1..=10.0).contains(&body.interval_seconds) {
        return Err(ApiError::bad_request(
            "interval_seconds must be between 0.1 and 10.0",
        ));
    }
    Ok(())
}

fn map_cnki_error(_error: ps_storage::CnkiRepositoryError) -> ApiError {
    ApiError::internal_server_error()
}

fn cnki_json_error(status: StatusCode, code: &str, phase: &str, message: &str) -> ApiError {
    ApiError::json_detail(
        status,
        json!({
            "code": code,
            "phase": phase,
            "message": message,
        }),
    )
}

fn replay_mode() -> Option<String> {
    std::env::var(REPLAY_MODE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn build_unsigned_jwt(expires_at: i64) -> String {
    format!(
        "{}.{}.",
        encode_base64_url(br#"{"alg":"none"}"#),
        encode_base64_url(format!(r#"{{"exp":{expires_at}}}"#).as_bytes()),
    )
}

fn encode_base64_url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let second = bytes.get(index + 1).copied().unwrap_or(0);
        let third = bytes.get(index + 2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if index + 1 < bytes.len() {
            encoded.push(ALPHABET[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        }
        if index + 2 < bytes.len() {
            encoded.push(ALPHABET[(third & 0b0011_1111) as usize] as char);
        }
        index += 3;
    }
    encoded
}

fn current_unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}
