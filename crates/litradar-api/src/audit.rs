//! Durable security audit helpers shared by authenticated routes.

use axum::extract::Extension;
use litradar_storage::{
    append_security_audit_event, report_security_audit_persistence_failure, SecurityAuditEvent,
};
use tower_http::request_id::RequestId;

use crate::response::ApiError;
use crate::state::ApiState;

/// Return the server-generated request identifier or an empty local marker.
pub(crate) fn request_id_text(request_id: Option<&Extension<RequestId>>) -> String {
    request_id
        .and_then(|Extension(request_id)| request_id.header_value().to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// Persist a rejection or limiter event outside a business mutation transaction.
pub(crate) async fn persist_security_audit_event(
    state: &ApiState,
    event: SecurityAuditEvent,
) -> Result<(), ApiError> {
    let auth_db_path = state.storage_config().auth_db_path().to_path_buf();
    match state
        .run_blocking(move || append_security_audit_event(auth_db_path, &event))
        .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Err(ApiError::service_unavailable()),
        Err(_) => {
            report_security_audit_persistence_failure("executor_unavailable");
            Err(ApiError::service_unavailable())
        }
    }
}
