//! Index database read route handlers.

use axum::extract::{Path, Query, RawQuery, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ps_storage::{
    ArticleListParams, DatabaseResolutionError, IndexRepositoryError, IssueListParams,
    JournalListParams,
};
use serde::Deserialize;

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

/// Query parameters that only select an index database.
#[derive(Debug, Deserialize)]
pub(crate) struct DbQuery {
    /// Database name or filename under `data/index`.
    db: Option<String>,
}

/// Journal list query parameters.
#[derive(Debug, Deserialize)]
pub(crate) struct JournalQuery {
    /// Database name or filename under `data/index`.
    db: Option<String>,
    /// Area filter.
    area: Option<String>,
    /// Library identifier filter.
    library_id: Option<String>,
    /// Available filter.
    available: Option<bool>,
    /// Has-articles filter.
    has_articles: Option<bool>,
    /// Publication year filter.
    year: Option<i64>,
    /// Minimum Scimago rank.
    scimago_min: Option<f64>,
    /// Maximum Scimago rank.
    scimago_max: Option<f64>,
    /// Sort expression.
    sort: Option<String>,
    /// Page size.
    limit: Option<i64>,
    /// Offset row count.
    offset: Option<i64>,
}

/// Issue list query parameters.
#[derive(Debug, Deserialize)]
pub(crate) struct IssueQuery {
    /// Database name or filename under `data/index`.
    db: Option<String>,
    /// Journal identifier filter.
    journal_id: Option<i64>,
    /// Publication year filter.
    year: Option<i64>,
    /// Valid issue filter.
    is_valid_issue: Option<bool>,
    /// Suppressed filter.
    suppressed: Option<bool>,
    /// Embargoed filter.
    embargoed: Option<bool>,
    /// Subscription filter.
    within_subscription: Option<bool>,
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
pub(crate) async fn list_databases(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, ApiError> {
    require_current_user(&state, &headers)?;
    let databases =
        ps_storage::list_index_database_names(state.storage_config()).map_err(map_index_error)?;
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
pub(crate) async fn list_areas(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<ps_domain::ValueCount>>, ApiError> {
    require_current_user(&state, &headers)?;
    let rows = ps_storage::list_areas(state.storage_config(), db_name(&query.db))
        .map_err(map_index_error)?;
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
pub(crate) async fn list_journal_options(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<ps_domain::JournalOption>>, ApiError> {
    require_current_user(&state, &headers)?;
    let rows = ps_storage::list_journal_options(state.storage_config(), db_name(&query.db))
        .map_err(map_index_error)?;
    Ok(Json(rows))
}

/// List metadata source counts.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `headers` - Request headers.
/// * `query` - Database selector.
///
/// # Returns
///
/// Source counts.
pub(crate) async fn list_sources(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<ps_domain::ValueCount>>, ApiError> {
    require_current_user(&state, &headers)?;
    let rows = ps_storage::list_sources(state.storage_config(), db_name(&query.db))
        .map_err(map_index_error)?;
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
pub(crate) async fn list_years(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<DbQuery>,
) -> Result<Json<Vec<ps_domain::YearSummary>>, ApiError> {
    require_current_user(&state, &headers)?;
    let rows = ps_storage::list_years(state.storage_config(), db_name(&query.db))
        .map_err(map_index_error)?;
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
pub(crate) async fn list_journals(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<JournalQuery>,
) -> Result<Json<ps_domain::JournalPage>, ApiError> {
    require_current_user(&state, &headers)?;
    let params = JournalListParams {
        area: query.area,
        library_id: query.library_id,
        available: query.available,
        has_articles: query.has_articles,
        year: query.year,
        scimago_min: query.scimago_min,
        scimago_max: query.scimago_max,
        sort: query.sort,
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
    };
    let page = ps_storage::list_journals(state.storage_config(), db_name(&query.db), &params)
        .map_err(map_index_error)?;
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
pub(crate) async fn get_journal(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(journal_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<ps_domain::JournalRecord>, ApiError> {
    require_current_user(&state, &headers)?;
    let row = ps_storage::get_journal(state.storage_config(), db_name(&query.db), journal_id)
        .map_err(map_index_error)?;
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
pub(crate) async fn list_issues(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<IssueQuery>,
) -> Result<Json<ps_domain::IssuePage>, ApiError> {
    require_current_user(&state, &headers)?;
    let params = IssueListParams {
        journal_id: query.journal_id,
        year: query.year,
        is_valid_issue: query.is_valid_issue,
        suppressed: query.suppressed,
        embargoed: query.embargoed,
        within_subscription: query.within_subscription,
        sort: query.sort,
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
    };
    let page = ps_storage::list_issues(state.storage_config(), db_name(&query.db), &params)
        .map_err(map_index_error)?;
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
pub(crate) async fn get_issue(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(issue_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<ps_domain::IssueRecord>, ApiError> {
    require_current_user(&state, &headers)?;
    let row = ps_storage::get_issue(state.storage_config(), db_name(&query.db), issue_id)
        .map_err(map_index_error)?;
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
pub(crate) async fn get_weekly_updates(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<ps_domain::WeeklyUpdatesResponse>, ApiError> {
    require_current_user(&state, &headers)?;
    let payload =
        ps_storage::get_weekly_updates(state.storage_config()).map_err(map_index_error)?;
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
pub(crate) async fn list_articles(
    State(state): State<ApiState>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<ps_domain::ArticlePage>, ApiError> {
    require_current_user(&state, &headers)?;
    let (db, params) = parse_article_query(raw_query.as_deref())?;
    let page = ps_storage::list_articles(state.storage_config(), db.as_deref(), &params)
        .map_err(map_index_error)?;
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
pub(crate) async fn get_article(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<ps_domain::ArticleRecord>, ApiError> {
    require_current_user(&state, &headers)?;
    let row = ps_storage::get_article(state.storage_config(), db_name(&query.db), article_id)
        .map_err(map_index_error)?;
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
pub(crate) async fn get_article_access(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Json<ps_domain::ArticleAccessResponse>, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let payload = ps_storage::get_article_access(
        state.storage_config(),
        db_name(&query.db),
        article_id,
        user.id,
    )
    .map_err(map_index_error)?;
    Ok(Json(payload))
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
pub(crate) async fn redirect_article_fulltext(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(article_id): Path<i64>,
    Query(query): Query<DbQuery>,
) -> Result<Response, ApiError> {
    let (user, _) = require_current_user(&state, &headers)?;
    let url = ps_storage::article_fulltext_redirect_url(
        state.storage_config(),
        db_name(&query.db),
        article_id,
        user.id,
    )
    .map_err(map_index_error)?;
    let location = HeaderValue::from_str(&url).map_err(|_| ApiError::internal_server_error())?;
    Ok((StatusCode::TEMPORARY_REDIRECT, [(LOCATION, location)]).into_response())
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
    params.suppressed = parse_optional_bool(&pairs, "suppressed")?;
    params.within_library_holdings = parse_optional_bool(&pairs, "within_library_holdings")?;
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

fn db_name(value: &Option<String>) -> Option<&str> {
    value.as_deref().and_then(nonempty)
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
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
