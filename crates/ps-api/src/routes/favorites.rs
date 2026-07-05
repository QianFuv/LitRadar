//! Favorites and folder route handlers.

use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ps_domain::{
    FavoriteAdd, FavoriteArticleResponse, FavoriteBatchCheckRequest, FavoriteBulkAdd,
    FavoriteBulkAddResult, FavoriteBulkMove, FavoriteBulkRemove, FavoriteBulkResult,
    FavoriteCheckResponse, FavoriteResponse, FavoriteTrackingResponse, FolderCreate, FolderRename,
    FolderResponse, OkResponse, TrackingSetRequest,
};
use ps_storage::BusinessRepositoryError;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

/// Query parameters for listing favorite articles.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FolderArticlesQuery {
    /// Maximum row count.
    limit: Option<i64>,
    /// Offset row count.
    offset: Option<i64>,
}

/// Query parameters for removing or checking favorites.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FavoriteDbQuery {
    /// Source database name.
    db_name: Option<String>,
}

/// Query parameters for checking one favorite.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FavoriteCheckQuery {
    /// Article identifier.
    article_id: i64,
    /// Source database name.
    db_name: Option<String>,
}

/// Query parameters for exporting favorites.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ExportQuery {
    /// Export format.
    format: Option<String>,
}

/// List all folders for the authenticated user.
#[utoipa::path(
    get,
    path = "/api/favorites/folders",
    tag = "favorites",
    responses((status = 200, description = "Favorite folders.", body = Vec<FolderResponse>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_folders(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<FolderResponse>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let folders = ps_storage::list_folders(state.storage_config().auth_db_path(), user.id)
        .map_err(map_business_error)?;
    Ok(Json(folders))
}

/// Create a new folder.
#[utoipa::path(
    post,
    path = "/api/favorites/folders",
    tag = "favorites",
    request_body = FolderCreate,
    responses((status = 200, description = "Created favorite folder.", body = FolderResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn create_folder(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<FolderCreate>,
) -> Result<Json<FolderResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let name = body.name.trim();
    validate_folder_name(name)?;
    let folder = ps_storage::create_folder(
        state.storage_config().auth_db_path(),
        user.id,
        name,
        body.is_tracking,
    )
    .map_err(map_business_error)?;
    Ok(Json(folder))
}

/// Rename an existing folder.
#[utoipa::path(
    put,
    path = "/api/favorites/folders/{folder_id}",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    request_body = FolderRename,
    responses((status = 200, description = "Folder renamed.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn rename_folder(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Json(body): Json<FolderRename>,
) -> Result<Json<OkResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let name = body.name.trim();
    validate_folder_name(name)?;
    let did_rename = ps_storage::rename_folder(
        state.storage_config().auth_db_path(),
        user.id,
        folder_id,
        name,
    )
    .map_err(map_business_error)?;
    if !did_rename {
        return Err(ApiError::not_found("Folder not found"));
    }
    Ok(Json(OkResponse { ok: true }))
}

/// Delete an existing folder.
#[utoipa::path(
    delete,
    path = "/api/favorites/folders/{folder_id}",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    responses((status = 200, description = "Folder deleted.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn delete_folder(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
) -> Result<Json<OkResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let did_delete =
        ps_storage::delete_folder(state.storage_config().auth_db_path(), user.id, folder_id)
            .map_err(map_business_error)?;
    if !did_delete {
        return Err(ApiError::not_found("Folder not found"));
    }
    Ok(Json(OkResponse { ok: true }))
}

/// Get the current tracking folder for the user.
#[utoipa::path(
    get,
    path = "/api/favorites/tracking",
    tag = "favorites",
    responses((status = 200, description = "Current favorite tracking folder.", body = FavoriteTrackingResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_tracking(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<FavoriteTrackingResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let folder = ps_storage::get_tracking_folder(state.storage_config().auth_db_path(), user.id)
        .map_err(map_business_error)?;
    Ok(Json(FavoriteTrackingResponse {
        folder_id: folder.as_ref().map(|item| item.id),
        folder_name: folder.map(|item| item.name),
    }))
}

/// Set a folder as the current tracking folder.
#[utoipa::path(
    put,
    path = "/api/favorites/tracking",
    tag = "favorites",
    request_body = TrackingSetRequest,
    responses((status = 200, description = "Tracking folder updated.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn set_tracking(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<TrackingSetRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let did_set = ps_storage::set_tracking_folder(
        state.storage_config().auth_db_path(),
        user.id,
        body.folder_id,
    )
    .map_err(map_business_error)?;
    if !did_set {
        return Err(ApiError::not_found("Folder not found"));
    }
    Ok(Json(OkResponse { ok: true }))
}

/// List favorited articles in a folder.
#[utoipa::path(
    get,
    path = "/api/favorites/folders/{folder_id}/articles",
    tag = "favorites",
    params(
        ("folder_id" = i64, Path, description = "Folder row identifier."),
        FolderArticlesQuery
    ),
    responses((status = 200, description = "Favorite articles.", body = Vec<FavoriteArticleResponse>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_folder_articles(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Query(query): Query<FolderArticlesQuery>,
) -> Result<Json<Vec<FavoriteArticleResponse>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);
    if !(1..=500).contains(&limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 500"));
    }
    if offset < 0 {
        return Err(ApiError::bad_request(
            "offset must be greater than or equal to 0",
        ));
    }
    let rows = ps_storage::list_favorite_articles(
        state.storage_config(),
        user.id,
        Some(folder_id),
        limit,
        offset,
    )
    .map_err(map_business_error)?;
    Ok(Json(rows))
}

/// Get the favorite count for a folder.
#[utoipa::path(
    get,
    path = "/api/favorites/folders/{folder_id}/count",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    responses((status = 200, description = "Favorite count.", body = serde_json::Value)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn folder_count(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let count = ps_storage::count_favorites(
        state.storage_config().auth_db_path(),
        user.id,
        Some(folder_id),
    )
    .map_err(map_business_error)?;
    Ok(Json(serde_json::json!({ "count": count })))
}

/// Export one folder's favorites in a citation format.
#[utoipa::path(
    get,
    path = "/api/favorites/folders/{folder_id}/export",
    tag = "favorites",
    params(
        ("folder_id" = i64, Path, description = "Folder row identifier."),
        ExportQuery
    ),
    responses((status = 200, description = "Citation export download.")),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn export_folder(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let folders = ps_storage::list_folders(state.storage_config().auth_db_path(), user.id)
        .map_err(map_business_error)?;
    let Some(folder) = folders.into_iter().find(|item| item.id == folder_id) else {
        return Err(ApiError::not_found("Folder not found"));
    };
    let articles = ps_storage::list_favorite_articles(
        state.storage_config(),
        user.id,
        Some(folder_id),
        100_000,
        0,
    )
    .map_err(map_business_error)?;
    let format = query.format.unwrap_or_else(|| "bibtex".to_string());
    let (content, media_type, extension) = match format.as_str() {
        "bibtex" => (to_bibtex(&articles), "application/x-bibtex", "bib"),
        "ris" => (
            to_ris(&articles),
            "application/x-research-info-systems",
            "ris",
        ),
        "endnote" => (to_endnote(&articles), "application/xml", "xml"),
        _ => return Err(ApiError::bad_request("Invalid export format")),
    };
    let filename = export_filename(&folder.name, extension);
    let mut response = content.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(media_type));
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|_| ApiError::internal_server_error())?,
    );
    Ok(response)
}

/// Add one favorite to a folder.
#[utoipa::path(
    post,
    path = "/api/favorites/folders/{folder_id}/articles",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    request_body = FavoriteAdd,
    responses((status = 200, description = "Created favorite row.", body = FavoriteResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn add_favorite(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Json(body): Json<FavoriteAdd>,
) -> Result<Json<FavoriteResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let favorite = ps_storage::add_favorite(
        state.storage_config().auth_db_path(),
        user.id,
        folder_id,
        &body,
    )
    .map_err(map_business_error)?;
    Ok(Json(favorite))
}

/// Remove one favorite from a folder.
#[utoipa::path(
    delete,
    path = "/api/favorites/folders/{folder_id}/articles/{article_id}",
    tag = "favorites",
    params(
        ("folder_id" = i64, Path, description = "Folder row identifier."),
        ("article_id" = i64, Path, description = "Article identifier."),
        FavoriteDbQuery
    ),
    responses((status = 200, description = "Favorite removed.", body = OkResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn remove_favorite(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((folder_id, article_id)): Path<(i64, i64)>,
    Query(query): Query<FavoriteDbQuery>,
) -> Result<Json<OkResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let did_remove = ps_storage::remove_favorite(
        state.storage_config().auth_db_path(),
        user.id,
        folder_id,
        article_id,
        query.db_name.as_deref().unwrap_or_default(),
    )
    .map_err(map_business_error)?;
    if !did_remove {
        return Err(ApiError::not_found("Favorite not found"));
    }
    Ok(Json(OkResponse { ok: true }))
}

/// Bulk add favorites to a folder.
#[utoipa::path(
    post,
    path = "/api/favorites/folders/{folder_id}/articles/bulk",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    request_body = FavoriteBulkAdd,
    responses((status = 200, description = "Bulk favorite add result.", body = FavoriteBulkAddResult)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn bulk_add(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Json(body): Json<FavoriteBulkAdd>,
) -> Result<Json<FavoriteBulkAddResult>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let added = ps_storage::bulk_add_favorites(
        state.storage_config().auth_db_path(),
        user.id,
        folder_id,
        &body.articles,
    )
    .map_err(map_business_error)?;
    Ok(Json(FavoriteBulkAddResult { added }))
}

/// Bulk remove favorites from a folder.
#[utoipa::path(
    post,
    path = "/api/favorites/folders/{folder_id}/articles/bulk-remove",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    request_body = FavoriteBulkRemove,
    responses((status = 200, description = "Bulk favorite remove result.", body = FavoriteBulkResult)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn bulk_remove(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Json(body): Json<FavoriteBulkRemove>,
) -> Result<Json<FavoriteBulkResult>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let count = ps_storage::bulk_remove_favorites(
        state.storage_config().auth_db_path(),
        user.id,
        folder_id,
        &body.articles,
    )
    .map_err(map_business_error)?;
    Ok(Json(FavoriteBulkResult { count }))
}

/// Bulk move favorites between folders.
#[utoipa::path(
    post,
    path = "/api/favorites/folders/{folder_id}/articles/bulk-move",
    tag = "favorites",
    params(("folder_id" = i64, Path, description = "Folder row identifier.")),
    request_body = FavoriteBulkMove,
    responses((status = 200, description = "Bulk favorite move result.", body = FavoriteBulkResult)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn bulk_move(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(folder_id): Path<i64>,
    Json(body): Json<FavoriteBulkMove>,
) -> Result<Json<FavoriteBulkResult>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let count = ps_storage::bulk_move_favorites(
        state.storage_config().auth_db_path(),
        user.id,
        folder_id,
        body.target_folder_id,
        &body.articles,
    )
    .map_err(map_business_error)?;
    Ok(Json(FavoriteBulkResult { count }))
}

/// Check which folders contain an article.
#[utoipa::path(
    get,
    path = "/api/favorites/check",
    tag = "favorites",
    params(FavoriteCheckQuery),
    responses((status = 200, description = "Favorite folder memberships.", body = Vec<FavoriteCheckResponse>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn check_favorite(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<FavoriteCheckQuery>,
) -> Result<Json<Vec<FavoriteCheckResponse>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let rows = ps_storage::is_favorited(
        state.storage_config().auth_db_path(),
        user.id,
        query.article_id,
        query.db_name.as_deref().unwrap_or_default(),
    )
    .map_err(map_business_error)?;
    Ok(Json(rows))
}

/// Batch check which folders contain articles.
#[utoipa::path(
    post,
    path = "/api/favorites/check/batch",
    tag = "favorites",
    request_body = FavoriteBatchCheckRequest,
    responses((status = 200, description = "Batch favorite folder memberships.", body = Vec<ps_domain::FavoriteBatchCheckResponse>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn check_favorites_batch(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<FavoriteBatchCheckRequest>,
) -> Result<Json<Vec<ps_domain::FavoriteBatchCheckResponse>>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let article_ids = body
        .article_ids
        .iter()
        .map(|article_id| article_id.value())
        .collect::<Vec<_>>();
    let rows = ps_storage::batch_is_favorited(
        state.storage_config().auth_db_path(),
        user.id,
        &article_ids,
        &body.db_name,
    )
    .map_err(map_business_error)?;
    Ok(Json(rows))
}

fn validate_folder_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::bad_request(
            "Folder name must be 1-100 characters",
        ));
    }
    Ok(())
}

fn map_business_error(error: BusinessRepositoryError) -> ApiError {
    match error {
        BusinessRepositoryError::DuplicateFolderName => {
            ApiError::conflict("Folder name already exists")
        }
        BusinessRepositoryError::FolderNotFound
        | BusinessRepositoryError::SourceFolderNotFound
        | BusinessRepositoryError::TargetFolderNotFound => ApiError::not_found(error.to_string()),
        BusinessRepositoryError::SourceAndTargetFoldersSame => {
            ApiError::bad_request(error.to_string())
        }
        _ => ApiError::internal_server_error(),
    }
}

fn export_filename(folder_name: &str, extension: &str) -> String {
    let safe_name = folder_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(['.', '_'])
        .to_string();
    let base_name = if safe_name.is_empty() {
        "favorites".to_string()
    } else {
        safe_name
    };
    format!("{base_name}.{extension}")
}

fn to_bibtex(articles: &[FavoriteArticleResponse]) -> String {
    articles
        .iter()
        .enumerate()
        .map(|(index, article)| {
            let key = article
                .doi
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or("favorite");
            format!(
                "@article{{{}{},\n  title = {{{}}},\n  author = {{{}}},\n  journal = {{{}}},\n  year = {{{}}},\n  doi = {{{}}}\n}}",
                sanitize_citation_key(key),
                index + 1,
                article.title.as_deref().unwrap_or(""),
                article.authors.as_deref().unwrap_or(""),
                article.journal_title.as_deref().unwrap_or(""),
                article.date.as_deref().unwrap_or(""),
                article.doi.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn to_ris(articles: &[FavoriteArticleResponse]) -> String {
    articles
        .iter()
        .map(|article| {
            format!(
                "TY  - JOUR\nTI  - {}\nAU  - {}\nJO  - {}\nPY  - {}\nDO  - {}\nER  -",
                article.title.as_deref().unwrap_or(""),
                article.authors.as_deref().unwrap_or(""),
                article.journal_title.as_deref().unwrap_or(""),
                article.date.as_deref().unwrap_or(""),
                article.doi.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn to_endnote(articles: &[FavoriteArticleResponse]) -> String {
    let records = articles
        .iter()
        .map(|article| {
            format!(
                "<record><titles><title>{}</title></titles><contributors><authors><author>{}</author></authors></contributors><dates><year>{}</year></dates><electronic-resource-num>{}</electronic-resource-num></record>",
                escape_xml(article.title.as_deref().unwrap_or("")),
                escape_xml(article.authors.as_deref().unwrap_or("")),
                escape_xml(article.date.as_deref().unwrap_or("")),
                escape_xml(article.doi.as_deref().unwrap_or(""))
            )
        })
        .collect::<String>();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><xml><records>{records}</records></xml>")
}

fn sanitize_citation_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
