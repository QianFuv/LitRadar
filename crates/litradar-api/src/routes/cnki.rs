//! Zhejiang Library CNKI session route handlers.

#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use litradar_domain::{
    CnkiLoginPollRequest, CnkiLoginPollResponse, CnkiLoginStartResponse, CnkiSessionStatusResponse,
    CnkiStatus,
};
use litradar_sources::{
    FixtureZjlibCnkiMode, FixtureZjlibCnkiTransport, LiveZjlibCnkiConfig, LiveZjlibCnkiTransport,
    ProviderProxy, ZhejiangLibraryCnkiClient, ZjlibCnkiError, ZJLIB_PROVIDER_NAME,
};
use litradar_storage::{CnkiRepositoryError, StorageConfig};
use serde_json::json;
use serde_json::Value as JsonValue;

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

const CNKI_NETWORK_QUEUE_TIMEOUT: Duration = Duration::from_secs(30);
const CNKI_NETWORK_TRANSPORT_TIMEOUT_SECONDS: u64 = 30;
const CNKI_LOGIN_START_FAILURE_MESSAGE: &str = "CNKI login start failed";
const CNKI_LOGIN_TIMEOUT_MESSAGE: &str = "CNKI login timed out";
const CNKI_LOGIN_FAILURE_MESSAGE: &str = "CNKI login failed";
const CNKI_WARMUP_FAILURE_MESSAGE: &str = "CNKI full-text session warm-up failed";

#[derive(Debug, Clone, Copy)]
enum CnkiUpstreamFailureKind {
    LoginStart,
    LoginTimeout,
    Login,
    Warmup,
}

#[cfg(test)]
static CNKI_FIXTURE_MODE: OnceLock<Mutex<Option<FixtureZjlibCnkiMode>>> = OnceLock::new();

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
#[utoipa::path(
    get,
    path = "/api/cnki/session",
    tag = "cnki",
    responses((status = 200, description = "Current CNKI session status.", body = CnkiSessionStatusResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<CnkiSessionStatusResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let status = run_cnki(&state, move |storage, secret_codec| {
        litradar_storage::get_cnki_session_status(storage.auth_db_path(), &secret_codec, user.id)
    })
    .await?;
    Ok(Json(status))
}

/// Start a QR login session.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// QR login challenge and safe session status.
#[utoipa::path(
    post,
    path = "/api/cnki/login/start",
    tag = "cnki",
    responses(
        (status = 200, description = "CNKI QR login challenge.", body = CnkiLoginStartResponse),
        (status = 409, description = "The login operation was superseded by a newer start or clear request.")
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn start_login(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<CnkiLoginStartResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let user_id = user.id;
    let generation = run_cnki(&state, move |storage, secret_codec| {
        litradar_storage::reserve_cnki_session_operation(
            storage.auth_db_path(),
            &secret_codec,
            user_id,
        )
    })
    .await?;
    let fixture_mode = zjlib_fixture_mode();
    let provider_proxy = state
        .provider_proxy_selection()
        .for_provider(ZJLIB_PROVIDER_NAME);
    let login_result = state
        .run_upstream_blocking_with_queue_timeout(CNKI_NETWORK_QUEUE_TIMEOUT, move || {
            start_zjlib_login(fixture_mode, provider_proxy)
        })
        .await?;
    let (qr_login, session_data) = login_result
        .map_err(|error| cnki_upstream_error(&error, CnkiUpstreamFailureKind::LoginStart))?;
    let qr_uuid = qr_login.uuid.clone();
    let session = run_cnki(&state, move |storage, secret_codec| {
        litradar_storage::compare_and_swap_cnki_session(
            storage.auth_db_path(),
            &secret_codec,
            user_id,
            generation,
            None,
            &session_data,
            &CnkiStatus::WaitingScan,
            Some(&qr_uuid),
        )
    })
    .await?
    .ok_or_else(cnki_operation_superseded_error)?;
    Ok(Json(CnkiLoginStartResponse {
        uuid: qr_login.uuid,
        status: CnkiStatus::from(qr_login.status),
        qr_code: qr_login.qr_code,
        session,
    }))
}

/// Poll a QR login session.
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
#[utoipa::path(
    post,
    path = "/api/cnki/login/poll",
    tag = "cnki",
    request_body = CnkiLoginPollRequest,
    responses(
        (status = 200, description = "CNKI QR login polling result.", body = CnkiLoginPollResponse),
        (status = 409, description = "The login operation was superseded by a newer start or clear request.")
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn poll_login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CnkiLoginPollRequest>,
) -> Result<Json<CnkiLoginPollResponse>, ApiError> {
    validate_poll_request(&body)?;
    let (user, _) = require_current_user(&state, &headers).await?;
    let user_id = user.id;
    let row = run_cnki(&state, move |storage, secret_codec| {
        litradar_storage::get_cnki_session_data(storage.auth_db_path(), &secret_codec, user_id)
    })
    .await?
    .ok_or_else(|| {
        cnki_json_error(
            StatusCode::BAD_REQUEST,
            "cnki_login_not_started",
            "login",
            "CNKI QR login has not been started",
        )
    })?;
    if row.qr_uuid.trim().is_empty() {
        return Err(cnki_json_error(
            StatusCode::BAD_REQUEST,
            "cnki_login_not_started",
            "login",
            "CNKI QR login has not been started",
        ));
    }
    let expected_generation = row.generation;
    let qr_uuid = row.qr_uuid;
    let mut session_data = row.session_data;
    if let Some(object) = session_data.as_object_mut() {
        object
            .entry("qr_uuid".to_string())
            .or_insert_with(|| JsonValue::String(qr_uuid.clone()));
    }
    let fixture_mode = zjlib_fixture_mode();
    let provider_proxy = state
        .provider_proxy_selection()
        .for_provider(ZJLIB_PROVIDER_NAME);
    let timeout_seconds = body.timeout_seconds;
    let interval_seconds = body.interval_seconds;
    let poll_result = state
        .run_upstream_blocking_with_queue_timeout(CNKI_NETWORK_QUEUE_TIMEOUT, move || {
            poll_zjlib_login(
                fixture_mode,
                &session_data,
                timeout_seconds,
                interval_seconds,
                provider_proxy,
            )
        })
        .await?;
    let session_data = match poll_result {
        Ok(session_data) => session_data,
        Err(ZjlibPollError::Login(error)) if error.is_timeout() => {
            return Err(cnki_upstream_error(
                &error,
                CnkiUpstreamFailureKind::LoginTimeout,
            ));
        }
        Err(ZjlibPollError::Login(error)) => {
            return Err(cnki_upstream_error(&error, CnkiUpstreamFailureKind::Login));
        }
        Err(ZjlibPollError::Warmup(error)) => {
            return Err(cnki_upstream_error(&error, CnkiUpstreamFailureKind::Warmup));
        }
    };
    let session = run_cnki(&state, move |storage, secret_codec| {
        litradar_storage::compare_and_swap_cnki_session(
            storage.auth_db_path(),
            &secret_codec,
            user_id,
            expected_generation,
            Some(&qr_uuid),
            &session_data,
            &CnkiStatus::Active,
            session_data
                .get("qr_uuid")
                .and_then(JsonValue::as_str)
                .or(Some(qr_uuid.as_str())),
        )
    })
    .await?
    .ok_or_else(cnki_operation_superseded_error)?;
    Ok(Json(CnkiLoginPollResponse {
        status: CnkiStatus::from("COMPLETE"),
        session,
    }))
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
#[utoipa::path(
    delete,
    path = "/api/cnki/session",
    tag = "cnki",
    responses((status = 200, description = "Cleared CNKI session status.", body = CnkiSessionStatusResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn clear_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<CnkiSessionStatusResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let status = run_cnki(&state, move |storage, secret_codec| {
        litradar_storage::delete_cnki_session(storage.auth_db_path(), &secret_codec, user.id)?;
        litradar_storage::get_cnki_session_status(storage.auth_db_path(), &secret_codec, user.id)
    })
    .await?;
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

fn map_cnki_error(_error: CnkiRepositoryError) -> ApiError {
    ApiError::internal_server_error()
}

async fn run_cnki<Output, Work>(state: &ApiState, work: Work) -> Result<Output, ApiError>
where
    Work: FnOnce(StorageConfig, litradar_storage::SecretCodec) -> Result<Output, CnkiRepositoryError>
        + Send
        + 'static,
    Output: Send + 'static,
{
    let storage = state.storage_config().clone();
    let secret_codec = state.secret_codec().clone();
    state
        .run_blocking(move || work(storage, secret_codec))
        .await?
        .map_err(map_cnki_error)
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

fn cnki_upstream_error(_: &ZjlibCnkiError, kind: CnkiUpstreamFailureKind) -> ApiError {
    match kind {
        CnkiUpstreamFailureKind::LoginStart => cnki_json_error(
            StatusCode::BAD_GATEWAY,
            "cnki_login_start_failed",
            "login",
            CNKI_LOGIN_START_FAILURE_MESSAGE,
        ),
        CnkiUpstreamFailureKind::LoginTimeout => cnki_json_error(
            StatusCode::REQUEST_TIMEOUT,
            "cnki_login_timeout",
            "login",
            CNKI_LOGIN_TIMEOUT_MESSAGE,
        ),
        CnkiUpstreamFailureKind::Login => cnki_json_error(
            StatusCode::BAD_REQUEST,
            "cnki_login_failed",
            "login",
            CNKI_LOGIN_FAILURE_MESSAGE,
        ),
        CnkiUpstreamFailureKind::Warmup => cnki_json_error(
            StatusCode::BAD_GATEWAY,
            "cnki_warmup_failed",
            "warmup",
            CNKI_WARMUP_FAILURE_MESSAGE,
        ),
    }
}

fn cnki_operation_superseded_error() -> ApiError {
    cnki_json_error(
        StatusCode::CONFLICT,
        "cnki_login_superseded",
        "login",
        "CNKI login operation was superseded",
    )
}

fn zjlib_fixture_mode() -> Option<FixtureZjlibCnkiMode> {
    #[cfg(test)]
    {
        return cnki_fixture_mode()
            .lock()
            .expect("CNKI fixture mode lock should not be poisoned")
            .clone();
    }
    #[cfg(not(test))]
    {
        None
    }
}

#[cfg(test)]
fn cnki_fixture_mode() -> &'static Mutex<Option<FixtureZjlibCnkiMode>> {
    CNKI_FIXTURE_MODE.get_or_init(|| Mutex::new(None))
}

/// Set Zhejiang Library CNKI fixture transport mode for route tests.
///
/// # Arguments
///
/// * `mode` - Optional fixture transport mode.
#[cfg(test)]
pub(crate) fn set_fixture_mode_for_tests(mode: Option<FixtureZjlibCnkiMode>) {
    *cnki_fixture_mode()
        .lock()
        .expect("CNKI fixture mode lock should not be poisoned") = mode;
}

fn start_zjlib_login(
    fixture_mode: Option<FixtureZjlibCnkiMode>,
    provider_proxy: ProviderProxy,
) -> Result<(litradar_sources::ZjlibCnkiQrLogin, JsonValue), ZjlibCnkiError> {
    if let Some(mode) = fixture_mode {
        let mut client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(mode));
        let qr_login = client.start_qr_login()?;
        let session_data = client.to_state_data();
        return Ok((qr_login, session_data));
    }
    let transport = LiveZjlibCnkiTransport::new_with_proxy(live_zjlib_config(), provider_proxy)?;
    let mut client = ZhejiangLibraryCnkiClient::new(transport);
    let qr_login = client.start_qr_login()?;
    let session_data = client.to_state_data();
    Ok((qr_login, session_data))
}

fn poll_zjlib_login(
    fixture_mode: Option<FixtureZjlibCnkiMode>,
    session_data: &JsonValue,
    timeout_seconds: i64,
    interval_seconds: f64,
    provider_proxy: ProviderProxy,
) -> Result<JsonValue, ZjlibPollError> {
    if let Some(mode) = fixture_mode {
        let mut client = ZhejiangLibraryCnkiClient::from_state_data(
            FixtureZjlibCnkiTransport::new(mode),
            session_data,
        );
        client
            .poll_qr_login(timeout_seconds, interval_seconds)
            .map_err(ZjlibPollError::Login)?;
        client
            .warm_up_fulltext_session()
            .map_err(ZjlibPollError::Warmup)?;
        return Ok(client.to_state_data());
    }
    let transport = LiveZjlibCnkiTransport::new_with_proxy(live_zjlib_config(), provider_proxy)
        .map_err(ZjlibPollError::Login)?;
    let mut client = ZhejiangLibraryCnkiClient::from_state_data(transport, session_data);
    client
        .poll_qr_login(timeout_seconds, interval_seconds)
        .map_err(ZjlibPollError::Login)?;
    client
        .warm_up_fulltext_session()
        .map_err(ZjlibPollError::Warmup)?;
    Ok(client.to_state_data())
}

fn live_zjlib_config() -> LiveZjlibCnkiConfig {
    LiveZjlibCnkiConfig {
        timeout_seconds: CNKI_NETWORK_TRANSPORT_TIMEOUT_SECONDS,
        ..LiveZjlibCnkiConfig::default()
    }
}

enum ZjlibPollError {
    Login(ZjlibCnkiError),
    Warmup(ZjlibCnkiError),
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use litradar_sources::ZjlibCnkiError;

    use super::{cnki_operation_superseded_error, cnki_upstream_error, CnkiUpstreamFailureKind};
    use crate::response::ApiError;

    #[test]
    fn superseded_cnki_operations_use_the_stable_conflict_envelope() {
        match cnki_operation_superseded_error() {
            ApiError::JsonDetail { status, detail } => {
                assert_eq!(status, StatusCode::CONFLICT);
                assert_eq!(detail["code"], "cnki_login_superseded");
                assert_eq!(detail["phase"], "login");
                assert_eq!(detail["message"], "CNKI login operation was superseded");
            }
            error => panic!("unexpected CNKI error: {error:?}"),
        }
    }

    #[test]
    fn upstream_cnki_errors_use_fixed_client_messages() {
        const SENTINEL: &str = "UPSTREAM_ERROR_SENTINEL_BFF_TOKEN";
        let upstream_error = ZjlibCnkiError::Request(SENTINEL.to_string());
        let cases = [
            (
                CnkiUpstreamFailureKind::LoginStart,
                StatusCode::BAD_GATEWAY,
                "cnki_login_start_failed",
                "login",
                "CNKI login start failed",
            ),
            (
                CnkiUpstreamFailureKind::LoginTimeout,
                StatusCode::REQUEST_TIMEOUT,
                "cnki_login_timeout",
                "login",
                "CNKI login timed out",
            ),
            (
                CnkiUpstreamFailureKind::Login,
                StatusCode::BAD_REQUEST,
                "cnki_login_failed",
                "login",
                "CNKI login failed",
            ),
            (
                CnkiUpstreamFailureKind::Warmup,
                StatusCode::BAD_GATEWAY,
                "cnki_warmup_failed",
                "warmup",
                "CNKI full-text session warm-up failed",
            ),
        ];

        for (kind, expected_status, expected_code, expected_phase, expected_message) in cases {
            match cnki_upstream_error(&upstream_error, kind) {
                ApiError::JsonDetail { status, detail } => {
                    assert_eq!(status, expected_status);
                    assert_eq!(detail["code"], expected_code);
                    assert_eq!(detail["phase"], expected_phase);
                    assert_eq!(detail["message"], expected_message);
                    assert!(!detail.to_string().contains(SENTINEL));
                }
                error => panic!("unexpected CNKI error: {error:?}"),
            }
        }
    }
}
