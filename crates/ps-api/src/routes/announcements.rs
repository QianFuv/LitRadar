//! Public announcement route handler.

use axum::extract::State;
use axum::Json;
use ps_domain::AnnouncementInfo;
use ps_storage::list_active_announcements;

use crate::response::ApiError;
use crate::state::ApiState;

/// List enabled announcements from the existing auth database.
///
/// # Arguments
///
/// * `state` - Shared API state containing storage paths.
///
/// # Returns
///
/// JSON announcement list ordered like the Python API.
#[utoipa::path(
    get,
    path = "/api/announcements",
    tag = "announcements",
    responses((status = 200, description = "Enabled announcements.", body = Vec<AnnouncementInfo>))
)]
pub(crate) async fn get_announcements(
    State(state): State<ApiState>,
) -> Result<Json<Vec<AnnouncementInfo>>, ApiError> {
    let announcements = list_active_announcements(state.storage_config().auth_db_path())
        .map_err(|_| ApiError::internal_server_error())?;
    Ok(Json(announcements))
}
