//! Index database read route handlers.

use axum::extract::{Path, Query, RawQuery, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
#[cfg(test)]
use litradar_sources::FixtureZjlibCnkiMode;
use litradar_storage::{
    ArticleListParams, DatabaseResolutionError, IndexRepositoryError, IssueListParams,
    JournalListParams, StorageConfig,
};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::article_access::{
    article_access_response, resolve_article_full_text, resolve_article_redirect,
    RedirectActionKind,
};
use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

/// Query parameters that only select an index database.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DbQuery {
    /// Database name or filename under `data/index`.
    db: Option<String>,
}

/// Journal list query parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct JournalQuery {
    /// Database name or filename under `data/index`.
    db: Option<String>,
    /// Area filter.
    area: Option<String>,
    /// Has-articles filter.
    has_articles: Option<bool>,
    /// Publication year filter.
    year: Option<i64>,
    /// Sort expression.
    sort: Option<String>,
    /// Page size.
    limit: Option<i64>,
    /// Offset row count.
    offset: Option<i64>,
}

/// Issue list query parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct IssueQuery {
    /// Database name or filename under `data/index`.
    db: Option<String>,
    /// Journal identifier filter.
    journal_id: Option<i64>,
    /// Publication year filter.
    year: Option<i64>,
    /// Sort expression.
    sort: Option<String>,
    /// Page size.
    limit: Option<i64>,
    /// Offset row count.
    offset: Option<i64>,
}

/// List index database filenames.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// JSON list of database filenames.
#[utoipa::path(
    get,
    path = "/api/meta/databases",
    tag = "index",
    responses((status = 200, description = "Index database filenames.", body = Vec<String>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_databases(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, ApiError> {
    require_current_user(&state, &headers).await?;
    let databases = run_index(&state, move |storage| {
        litradar_storage::list_index_database_names(&storage)
    })
    .await?;
    Ok(Json(databases))
}

/// List journal area counts.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `query` - Database selector.
///
/// # Returns
///
/// Journal area counts.
#[utoipa::path(
    get,
    path = "/api/meta/areas",
    tag = "index",
    params(DbQuery),
    responses((status = 200, description = "Journal area counts.", body = Vec<litradar_domain::ValueCount>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_areas(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<litradar_domain::ValueCount>>, ApiError> {
    require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let rows = run_index(&state, move |storage| {
        litradar_storage::list_areas(&storage, db.as_deref())
    })
    .await?;
    Ok(Json(rows))
}

/// List journal selector options.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `query` - Database selector.
///
/// # Returns
///
/// Journal option records.
#[utoipa::path(
    get,
    path = "/api/meta/journals",
    tag = "index",
    params(DbQuery),
    responses((status = 200, description = "Journal selector options.", body = Vec<litradar_domain::JournalOption>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_journal_options(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<litradar_domain::JournalOption>>, ApiError> {
    require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let rows = run_index(&state, move |storage| {
        litradar_storage::list_journal_options(&storage, db.as_deref())
    })
    .await?;
    Ok(Json(rows))
}

/// List publication year summaries.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `query` - Database selector.
///
/// # Returns
///
/// Year summary records.
#[utoipa::path(
    get,
    path = "/api/years",
    tag = "index",
    params(DbQuery),
    responses((status = 200, description = "Publication year summaries.", body = Vec<litradar_domain::YearSummary>)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_years(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<litradar_domain::YearSummary>>, ApiError> {
    require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let rows = run_index(&state, move |storage| {
        litradar_storage::list_years(&storage, db.as_deref())
    })
    .await?;
    Ok(Json(rows))
}

/// List journals with filters and offset pagination.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `query` - Journal list filters.
///
/// # Returns
///
/// Paginated journals.
#[utoipa::path(
    get,
    path = "/api/journals",
    tag = "index",
    params(JournalQuery),
    responses((status = 200, description = "Paginated journals.", body = litradar_domain::JournalPage)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_journals(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<JournalQuery>,
) -> Result<Json<litradar_domain::JournalPage>, ApiError> {
    require_current_user(&state, &headers).await?;
    let params = JournalListParams {
        area: query.area,
        has_articles: query.has_articles,
        year: query.year,
        sort: query.sort,
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
    };
    let db = query.db.and_then(nonempty_owned);
    let page = run_index(&state, move |storage| {
        litradar_storage::list_journals(&storage, db.as_deref(), &params)
    })
    .await?;
    Ok(Json(page))
}

/// Return one journal record.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `journal_id` - Journal identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// Journal record.
#[utoipa::path(
    get,
    path = "/api/journals/{journal_id}",
    tag = "index",
    params(
        ("journal_id" = i64, Path, description = "Journal identifier."),
        DbQuery
    ),
    responses((status = 200, description = "Journal record.", body = litradar_domain::JournalRecord)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_journal(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(journal_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<litradar_domain::JournalRecord>, ApiError> {
    require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let row = run_index(&state, move |storage| {
        litradar_storage::get_journal(&storage, db.as_deref(), journal_id)
    })
    .await?;
    Ok(Json(row))
}

/// List issues with filters and offset pagination.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `query` - Issue list filters.
///
/// # Returns
///
/// Paginated issues.
#[utoipa::path(
    get,
    path = "/api/issues",
    tag = "index",
    params(IssueQuery),
    responses((status = 200, description = "Paginated issues.", body = litradar_domain::IssuePage)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_issues(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<IssueQuery>,
) -> Result<Json<litradar_domain::IssuePage>, ApiError> {
    require_current_user(&state, &headers).await?;
    let params = IssueListParams {
        journal_id: query.journal_id,
        year: query.year,
        sort: query.sort,
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
    };
    let db = query.db.and_then(nonempty_owned);
    let page = run_index(&state, move |storage| {
        litradar_storage::list_issues(&storage, db.as_deref(), &params)
    })
    .await?;
    Ok(Json(page))
}

/// Return one issue record.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `issue_id` - Issue identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// Issue record.
#[utoipa::path(
    get,
    path = "/api/issues/{issue_id}",
    tag = "index",
    params(
        ("issue_id" = i64, Path, description = "Issue identifier."),
        DbQuery
    ),
    responses((status = 200, description = "Issue record.", body = litradar_domain::IssueRecord)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_issue(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(issue_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<litradar_domain::IssueRecord>, ApiError> {
    require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let row = run_index(&state, move |storage| {
        litradar_storage::get_issue(&storage, db.as_deref(), issue_id)
    })
    .await?;
    Ok(Json(row))
}

/// List weekly article updates grouped by database and journal.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
///
/// # Returns
///
/// Weekly update response.
#[utoipa::path(
    get,
    path = "/api/weekly-updates",
    tag = "index",
    responses((status = 200, description = "Weekly article updates.", body = litradar_domain::WeeklyUpdatesResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_weekly_updates(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<litradar_domain::WeeklyUpdatesResponse>, ApiError> {
    require_current_user(&state, &headers).await?;
    let payload = run_index(&state, move |storage| {
        litradar_storage::get_weekly_updates(&storage)
    })
    .await?;
    Ok(Json(payload))
}

/// List articles with filters, FTS, and cursor pagination.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `raw_query` - Raw query string so repeated fields remain available.
///
/// # Returns
///
/// Paginated articles.
#[utoipa::path(
    get,
    path = "/api/articles",
    tag = "index",
    params(
        ("db" = Option<String>, Query, description = "Database name or filename under data/index."),
        ("journal_id" = Vec<i64>, Query, description = "Repeated journal identifier filters."),
        ("area" = Vec<String>, Query, description = "Repeated area filters."),
        ("issue_id" = Option<i64>, Query, description = "Issue identifier filter."),
        ("year" = Option<i64>, Query, description = "Publication year filter."),
        ("in_press" = Option<bool>, Query, description = "In-press filter."),
        ("open_access" = Option<bool>, Query, description = "Open-access filter."),
        ("date_from" = Option<String>, Query, description = "Start date filter."),
        ("date_to" = Option<String>, Query, description = "End date filter."),
        ("doi" = Option<String>, Query, description = "DOI filter."),
        ("pmid" = Option<String>, Query, description = "PubMed identifier filter."),
        ("q" = Option<String>, Query, description = "Full-text query."),
        ("sort" = Option<String>, Query, description = "Sort expression."),
        ("limit" = Option<i64>, Query, description = "Page size."),
        ("offset" = Option<i64>, Query, description = "Offset row count."),
        ("cursor" = Option<String>, Query, description = "Keyset cursor."),
        ("include_total" = Option<bool>, Query, description = "Whether to include total row count.")
    ),
    responses((status = 200, description = "Paginated articles.", body = litradar_domain::ArticlePage)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn list_articles(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<litradar_domain::ArticlePage>, ApiError> {
    require_current_user(&state, &headers).await?;
    let (db, params) = parse_article_query(raw_query.as_deref())?;
    let page = run_index(&state, move |storage| {
        litradar_storage::list_articles(&storage, db.as_deref(), &params)
    })
    .await?;
    Ok(Json(page))
}

/// Return one article record.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `article_id` - Article identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// Article record.
#[utoipa::path(
    get,
    path = "/api/articles/{article_id}",
    tag = "index",
    params(
        ("article_id" = i64, Path, description = "Article identifier."),
        DbQuery
    ),
    responses((status = 200, description = "Article record.", body = litradar_domain::ArticleRecord)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_article(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<litradar_domain::ArticleRecord>, ApiError> {
    require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let row = run_index(&state, move |storage| {
        litradar_storage::get_article(&storage, db.as_deref(), article_id)
    })
    .await?;
    Ok(Json(row))
}

/// Return access actions for one article.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `article_id` - Article identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// Article access response.
#[utoipa::path(
    get,
    path = "/api/articles/{article_id}/access",
    tag = "index",
    params(
        ("article_id" = i64, Path, description = "Article identifier."),
        DbQuery
    ),
    responses((status = 200, description = "Article access actions.", body = litradar_domain::ArticleAccessResponse)),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn get_article_access(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<litradar_domain::ArticleAccessResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    run_index(&state, move |storage| {
        litradar_storage::get_article_locator(&storage, db.as_deref(), article_id)
    })
    .await?;
    let payload = article_access_response(&state, user.id).await?;
    Ok(Json(payload))
}

/// Resolve and redirect to an article detail page online.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `article_id` - Article identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// No-store temporary redirect.
#[utoipa::path(
    get,
    path = "/api/articles/{article_id}/detail",
    tag = "index",
    params(("article_id" = i64, Path, description = "Article identifier."), DbQuery),
    responses((status = 307, description = "Temporary online detail redirect.")),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn redirect_article_detail(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Response, ApiError> {
    redirect_article_action(
        state,
        headers,
        article_id,
        query,
        RedirectActionKind::Detail,
    )
    .await
}

/// Resolve and redirect to an article abstract page online.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `article_id` - Article identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// No-store temporary redirect.
#[utoipa::path(
    get,
    path = "/api/articles/{article_id}/abstract",
    tag = "index",
    params(("article_id" = i64, Path, description = "Article identifier."), DbQuery),
    responses((status = 307, description = "Temporary online abstract-page redirect.")),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn redirect_article_abstract(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Response, ApiError> {
    redirect_article_action(
        state,
        headers,
        article_id,
        query,
        RedirectActionKind::Abstract,
    )
    .await
}

async fn redirect_article_action(
    state: ApiState,
    headers: HeaderMap,
    article_id: i64,
    query: DbQuery,
    kind: RedirectActionKind,
) -> Result<Response, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let article = run_index(&state, move |storage| {
        litradar_storage::get_article_locator(&storage, db.as_deref(), article_id)
    })
    .await?;
    let redirect = resolve_article_redirect(&state, article, user.id, kind).await?;
    no_store_redirect(&redirect.location)
}

/// Redirect to an article full-text target.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `article_id` - Article identifier.
/// * `query` - Database selector.
///
/// # Returns
///
/// Temporary redirect response.
#[utoipa::path(
    get,
    path = "/api/articles/{article_id}/fulltext",
    tag = "index",
    params(
        ("article_id" = i64, Path, description = "Article identifier."),
        DbQuery
    ),
    responses(
        (status = 200, description = "Full-text file download."),
        (status = 307, description = "Temporary redirect to a full-text target.")
    ),
    security(("bearer_auth" = []), ("session_cookie" = []))
)]
pub(crate) async fn redirect_article_fulltext(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Response, ApiError> {
    let (user, _) = require_current_user(&state, &headers).await?;
    let db = query.db.and_then(nonempty_owned);
    let article = run_index(&state, move |storage| {
        litradar_storage::get_article_locator(&storage, db.as_deref(), article_id)
    })
    .await?;
    match resolve_article_full_text(&state, article, user.id).await? {
        litradar_domain::ArticleFullTextResolution::Redirect(redirect) => {
            no_store_redirect(&redirect.location)
        }
        litradar_domain::ArticleFullTextResolution::Document(document) => {
            let mut response = document.bytes.into_response();
            response
                .headers_mut()
                .insert(CONTENT_TYPE, header_value(&document.content_type)?);
            if let Some(filename) = document.filename {
                response.headers_mut().insert(
                    CONTENT_DISPOSITION,
                    header_value(&format!(
                        "attachment; filename*=UTF-8''{}",
                        percent_encode_filename(&filename)
                    ))?,
                );
            }
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
            Ok(response)
        }
    }
}

/// Set Zhejiang Library CNKI fixture transport mode for index route tests.
///
/// # Arguments
///
/// * `mode` - Optional fixture transport mode.
#[cfg(test)]
pub(crate) fn set_fixture_mode_for_tests(mode: Option<FixtureZjlibCnkiMode>) {
    crate::article_access::set_full_text_fixture_mode(mode);
}

fn no_store_redirect(location: &str) -> Result<Response, ApiError> {
    Ok((
        StatusCode::TEMPORARY_REDIRECT,
        [
            (LOCATION, header_value(location)?),
            (CACHE_CONTROL, HeaderValue::from_static("private, no-store")),
        ],
    )
        .into_response())
}

fn parse_article_query(
    raw_query: Option<&str>,
) -> Result<(Option<String>, ArticleListParams), ApiError> {
    let pairs = parse_query_pairs(raw_query)?;
    let mut params = ArticleListParams::default();
    params.journal_id = parse_i64_values(&pairs, "journal_id")?;
    params.area = query_values(&pairs, "area");
    params.issue_id = parse_optional_i64(&pairs, "issue_id")?;
    params.year = parse_optional_i64(&pairs, "year")?;
    params.in_press = parse_optional_bool(&pairs, "in_press")?;
    params.open_access = parse_optional_bool(&pairs, "open_access")?;
    params.date_from = query_value(&pairs, "date_from");
    params.date_to = query_value(&pairs, "date_to");
    params.doi = query_value(&pairs, "doi");
    params.pmid = query_value(&pairs, "pmid");
    params.q = query_value(&pairs, "q");
    params.sort = query_value(&pairs, "sort").or(params.sort);
    params.limit = parse_optional_i64(&pairs, "limit")?.unwrap_or(params.limit);
    params.offset = parse_optional_i64(&pairs, "offset")?.unwrap_or(params.offset);
    params.cursor = query_value(&pairs, "cursor");
    params.include_total =
        parse_optional_bool(&pairs, "include_total")?.unwrap_or(params.include_total);
    Ok((query_value(&pairs, "db"), params))
}

fn parse_query_pairs(raw_query: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
    let Some(raw_query) = raw_query else {
        return Ok(Vec::new());
    };
    raw_query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Ok((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn percent_decode(value: &str) -> Result<String, ApiError> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1])?;
                let low = hex_value(bytes[index + 2])?;
                output.push(high * 16 + low);
                index += 3;
            }
            b'%' => return Err(ApiError::bad_request("Invalid query encoding")),
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).map_err(|_| ApiError::bad_request("Invalid query encoding"))
}

fn hex_value(value: u8) -> Result<u8, ApiError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(ApiError::bad_request("Invalid query encoding")),
    }
}

fn header_value(value: &str) -> Result<HeaderValue, ApiError> {
    HeaderValue::from_str(value).map_err(|_| ApiError::internal_server_error())
}

fn percent_encode_filename(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn query_values(pairs: &[(String, String)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .collect()
}

fn query_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .rev()
        .find_map(|(name, value)| (name == key).then(|| value.clone()))
        .and_then(nonempty_owned)
}

fn parse_i64_values(pairs: &[(String, String)], key: &str) -> Result<Vec<i64>, ApiError> {
    query_values(pairs, key)
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| parse_i64(key, &value))
        .collect()
}

fn parse_optional_i64(pairs: &[(String, String)], key: &str) -> Result<Option<i64>, ApiError> {
    query_value(pairs, key)
        .map(|value| parse_i64(key, &value))
        .transpose()
}

fn parse_i64(key: &str, value: &str) -> Result<i64, ApiError> {
    value
        .trim()
        .parse::<i64>()
        .map_err(|_| ApiError::bad_request(format!("Invalid integer for {key}")))
}

fn parse_optional_bool(pairs: &[(String, String)], key: &str) -> Result<Option<bool>, ApiError> {
    query_value(pairs, key)
        .map(|value| parse_bool(key, &value))
        .transpose()
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(ApiError::bad_request(format!("Invalid boolean for {key}"))),
    }
}

fn nonempty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn map_index_error(error: IndexRepositoryError) -> ApiError {
    match error {
        IndexRepositoryError::DatabaseResolution(DatabaseResolutionError::DatabaseNotFound)
        | IndexRepositoryError::DatabaseResolution(
            DatabaseResolutionError::NoSqliteDatabasesFound,
        )
        | IndexRepositoryError::NotFound(_) => ApiError::not_found(error.to_string()),
        IndexRepositoryError::DatabaseResolution(
            DatabaseResolutionError::MultipleDatabasesFound,
        )
        | IndexRepositoryError::UnsupportedSortField(_)
        | IndexRepositoryError::UnsupportedArticleSort
        | IndexRepositoryError::InvalidCursor
        | IndexRepositoryError::InvalidPagination(_) => ApiError::bad_request(error.to_string()),
        IndexRepositoryError::DatabaseResolution(DatabaseResolutionError::Io(_))
        | IndexRepositoryError::Sqlite(_)
        | IndexRepositoryError::Io(_)
        | IndexRepositoryError::Json(_) => ApiError::internal_server_error(),
    }
}

async fn run_index<Output, Work>(state: &ApiState, work: Work) -> Result<Output, ApiError>
where
    Work: FnOnce(StorageConfig) -> Result<Output, IndexRepositoryError> + Send + 'static,
    Output: Send + 'static,
{
    let storage = state.storage_config().clone();
    state
        .run_blocking(move || work(storage))
        .await?
        .map_err(map_index_error)
}

#[cfg(test)]
mod tests {
    use std::io;

    use axum::http::StatusCode;
    use rusqlite::Error as SqliteError;

    use super::*;

    #[test]
    fn parses_article_query_repeated_values_and_last_scalar_values() {
        let (db, params) = parse_article_query(Some(
            "db=first.sqlite&db=fixture.sqlite&journal_id=1&journal_id=2&journal_id=&area=Medicine&area=Data+Science&issue_id=10&year=2026&in_press=yes&open_access=1&date_from=2026-01-01&date_to=2026-02-01&doi=10.1000%2Fabc&pmid=PMID%2B42&q=genome+search&sort=date%3Aasc&limit=25&offset=5&cursor=2026-01-05%7C1001&include_total=false",
        ))
        .expect("query should parse");

        assert_eq!(db.as_deref(), Some("fixture.sqlite"));
        assert_eq!(params.journal_id, [1, 2]);
        assert_eq!(params.area, ["Medicine", "Data Science"]);
        assert_eq!(params.issue_id, Some(10));
        assert_eq!(params.year, Some(2026));
        assert_eq!(params.in_press, Some(true));
        assert_eq!(params.open_access, Some(true));
        assert_eq!(params.date_from.as_deref(), Some("2026-01-01"));
        assert_eq!(params.date_to.as_deref(), Some("2026-02-01"));
        assert_eq!(params.doi.as_deref(), Some("10.1000/abc"));
        assert_eq!(params.pmid.as_deref(), Some("PMID+42"));
        assert_eq!(params.q.as_deref(), Some("genome search"));
        assert_eq!(params.sort.as_deref(), Some("date:asc"));
        assert_eq!(params.limit, 25);
        assert_eq!(params.offset, 5);
        assert_eq!(params.cursor.as_deref(), Some("2026-01-05|1001"));
        assert!(!params.include_total);
    }

    #[test]
    fn query_decoding_rejects_invalid_percent_and_utf8_sequences() {
        assert_bad_request_detail(
            parse_query_pairs(Some("q=%ZZ")).expect_err("invalid hex should fail"),
            "Invalid query encoding",
        );
        assert_bad_request_detail(
            parse_query_pairs(Some("q=%E4%ZZ")).expect_err("partial invalid utf8 should fail"),
            "Invalid query encoding",
        );
        assert_bad_request_detail(
            parse_query_pairs(Some("q=%E4")).expect_err("truncated percent should fail"),
            "Invalid query encoding",
        );
        assert_bad_request_detail(
            parse_query_pairs(Some("q=%FF")).expect_err("invalid UTF-8 should fail"),
            "Invalid query encoding",
        );
    }

    #[test]
    fn query_scalar_parsers_report_field_specific_errors() {
        let pairs = vec![
            ("limit".to_string(), "abc".to_string()),
            ("open_access".to_string(), "maybe".to_string()),
        ];

        assert_bad_request_detail(
            parse_optional_i64(&pairs, "limit").expect_err("invalid integer should fail"),
            "Invalid integer for limit",
        );
        assert_bad_request_detail(
            parse_optional_bool(&pairs, "open_access").expect_err("invalid bool should fail"),
            "Invalid boolean for open_access",
        );

        for value in ["true", "TRUE", "on", "yes", "1"] {
            assert!(parse_bool("flag", value).expect("truthy value should parse"));
        }
        for value in ["false", "FALSE", "off", "no", "0"] {
            assert!(!parse_bool("flag", value).expect("falsey value should parse"));
        }
    }

    #[test]
    fn filename_encoding_preserves_safe_ascii_and_encodes_other_bytes() {
        assert_eq!(
            percent_encode_filename("Alpha Journal_2026.pdf"),
            "Alpha%20Journal_2026.pdf"
        );
        assert_eq!(
            percent_encode_filename("中文 fulltext.pdf"),
            "%E4%B8%AD%E6%96%87%20fulltext.pdf"
        );
        assert_eq!(
            header_value("attachment; filename*=UTF-8''Alpha%20Journal.pdf")
                .expect("header value should parse")
                .to_str()
                .expect("header value should be visible"),
            "attachment; filename*=UTF-8''Alpha%20Journal.pdf"
        );
        assert_api_error(
            header_value("bad\nheader").expect_err("invalid header should fail"),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        );
    }

    #[test]
    fn index_error_mapping_keeps_route_status_contract() {
        assert_api_error(
            map_index_error(IndexRepositoryError::DatabaseResolution(
                DatabaseResolutionError::DatabaseNotFound,
            )),
            StatusCode::NOT_FOUND,
            "Database not found",
        );
        assert_api_error(
            map_index_error(IndexRepositoryError::DatabaseResolution(
                DatabaseResolutionError::MultipleDatabasesFound,
            )),
            StatusCode::BAD_REQUEST,
            "Multiple databases found, specify ?db=<name>",
        );
        assert_api_error(
            map_index_error(IndexRepositoryError::UnsupportedArticleSort),
            StatusCode::BAD_REQUEST,
            "Articles only support sort=date:desc or date:asc",
        );
        assert_api_error(
            map_index_error(IndexRepositoryError::Sqlite(SqliteError::InvalidQuery)),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        );
        assert_api_error(
            map_index_error(IndexRepositoryError::DatabaseResolution(
                DatabaseResolutionError::Io(io::Error::other("disk")),
            )),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        );
    }

    fn assert_bad_request_detail(error: ApiError, detail: &str) {
        assert_api_error(error, StatusCode::BAD_REQUEST, detail);
    }

    fn assert_api_error(error: ApiError, status: StatusCode, detail: &str) {
        match error {
            ApiError::Http {
                status: actual_status,
                detail: actual_detail,
            } => {
                assert_eq!(actual_status, status);
                assert_eq!(actual_detail, detail);
            }
            ApiError::Unexpected { .. } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(detail, "Internal Server Error");
            }
            ApiError::JsonDetail { .. } | ApiError::TooManyRequests { .. } => {
                panic!("expected HTTP error")
            }
        }
    }
}
