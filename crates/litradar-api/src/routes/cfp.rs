//! Authenticated CFP catalog and revision-consistent original-notice queries.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use litradar_domain::cfp::{
    CfpCatalogResponse, CfpCatalogSummary, CfpCoverage, CfpJournalSummary, CfpNoticePage,
    CfpNoticeView, CfpRefreshStatus,
};
use litradar_domain::{JournalCatalogEntry, PageMeta};
use litradar_index::transforms::read_catalog_csv;
use litradar_sources::cfp::cfp_source_registry;
use litradar_storage::business::cfp::{load_cfp_journals, CfpJournalSnapshot};
use litradar_storage::{SecretCodec, StorageConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::IntoParams;

use crate::response::ApiError;
use crate::routes::auth::require_current_user;
use crate::state::ApiState;

const CURSOR_CONTEXT: &str = "litradar.cfp.cursor.v1";
const CURSOR_RELOAD_DETAIL: &str =
    "CFP page changed or cursor is invalid; reload from the first page";

/// Maintained database catalog and optional journal search.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in=Query)]
pub(crate) struct CatalogQuery {
    /// Database filename, such as english_journals.sqlite.
    db: String,
    /// Case-insensitive journal title, ID or subject search, limited to 256 characters.
    q: Option<String>,
}

/// One journal's closed filter and bounded cursor page.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in=Query)]
pub(crate) struct NoticeQuery {
    /// Database filename containing the maintained journal.
    db: String,
    /// Include closed and historical records; defaults to false.
    #[serde(default)]
    include_closed: bool,
    /// Page size, default 50 and maximum 200.
    limit: Option<usize>,
    /// Opaque cursor returned by this journal/filter's previous page.
    cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeCursor {
    database: String,
    catalog_id: String,
    include_closed: bool,
    revision: String,
    evaluated_at: i64,
    position: usize,
    order: String,
}

/// List every maintained journal, including those with no indexed articles or CFP adapter.
#[utoipa::path(get,path="/api/cfp/journals",operation_id="list_cfp_journals",tag="cfp",params(CatalogQuery),
    responses((status=200,description="Lightweight journal catalog and source coverage.",body=CfpCatalogResponse),(status=401,description="Authentication required."),(status=404,description="Database catalog not found.")),
    security(("bearer_auth"=[]),("session_cookie"=[])))]
pub(crate) async fn list_journals(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<CfpCatalogResponse>, ApiError> {
    require_current_user(&state, &headers).await?;
    if query
        .q
        .as_ref()
        .is_some_and(|value| value.chars().count() > 256)
    {
        return Err(ApiError::bad_request(
            "CFP journal search exceeds 256 characters",
        ));
    }
    let storage = state.storage_config().clone();
    let result = state
        .run_blocking(move || catalog_response(&storage, &query))
        .await??;
    Ok(Json(result))
}

/// Read original notices and backend-calculated availability at a cursor's fixed instant.
#[utoipa::path(get,path="/api/cfp/journals/{catalog_id}/notices",operation_id="list_cfp_notices",tag="cfp",
    params(("catalog_id"=String,Path,description="Canonical maintained catalog ID or its alias."),NoticeQuery),
    responses((status=200,description="Original notices, coverage and pagination.",body=CfpNoticePage),(status=401,description="Authentication required."),(status=404,description="Database or journal not found."),(status=409,description="Stale or incompatible cursor; reload the first page.")),
    security(("bearer_auth"=[]),("session_cookie"=[])))]
pub(crate) async fn list_notices(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(catalog_id): Path<String>,
    Query(query): Query<NoticeQuery>,
) -> Result<Json<CfpNoticePage>, ApiError> {
    require_current_user(&state, &headers).await?;
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(ApiError::bad_request(
            "CFP page limit must be between 1 and 200",
        ));
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 4096)
    {
        return Err(ApiError::conflict(CURSOR_RELOAD_DETAIL));
    }
    let storage = state.storage_config().clone();
    let codec = state.secret_codec().clone();
    let result = state
        .run_blocking(move || notice_response(&storage, &codec, &catalog_id, &query, limit))
        .await??;
    Ok(Json(result))
}

fn catalog_members(
    storage: &StorageConfig,
    database: &str,
) -> Result<Vec<JournalCatalogEntry>, ApiError> {
    let catalog = storage
        .list_provider_catalogs()
        .map_err(|_| ApiError::internal_server_error())?
        .into_iter()
        .find(|catalog| format!("{}.sqlite", catalog.stem) == database)
        .ok_or_else(|| ApiError::not_found("CFP database catalog not found"))?;
    let filename = catalog
        .csv_filename
        .ok_or_else(|| ApiError::not_found("CFP database catalog not found"))?;
    read_catalog_csv(storage.meta_dir().join(filename))
        .map_err(|_| ApiError::internal_server_error())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn find_snapshot<'a>(
    entry: &JournalCatalogEntry,
    snapshots: &'a [CfpJournalSnapshot],
) -> Result<Option<&'a CfpJournalSnapshot>, ApiError> {
    let aliases: BTreeSet<_> = std::iter::once(entry.catalog_id.as_str())
        .chain(entry.catalog_aliases.iter().map(String::as_str))
        .collect();
    let matches: Vec<_> = snapshots
        .iter()
        .filter(|snapshot| {
            snapshot
                .catalog_ids
                .iter()
                .any(|alias| aliases.contains(alias.as_str()))
        })
        .collect();
    if matches.len() > 1 {
        return Err(ApiError::internal_server_error());
    }
    Ok(matches.first().copied())
}

fn journal_summary(
    entry: &JournalCatalogEntry,
    snapshot: Option<&CfpJournalSnapshot>,
    evaluated_at: i64,
) -> Result<CfpJournalSummary, ApiError> {
    let now = chrono::DateTime::from_timestamp(evaluated_at, 0)
        .ok_or_else(ApiError::internal_server_error)?;
    let mut counts = BTreeMap::new();
    let mut current_count = 0;
    if let Some(snapshot) = snapshot {
        for notice in &snapshot.notices {
            let state = notice.state(now);
            *counts.entry(state).or_insert(0) += 1;
            if !state.is_archived() {
                current_count += 1;
            }
        }
    }
    let registration = snapshot.and_then(|snapshot| {
        cfp_source_registry()
            .iter()
            .find(|source| source.catalog_ids.contains(&snapshot.journal_key))
    });
    let source = snapshot.and_then(|snapshot| {
        snapshot
            .sources
            .iter()
            .max_by_key(|source| source.last_attempt)
    });
    let has_expired_lease = source.is_some_and(|source| {
        source.status == "refreshing"
            && source
                .lease_expires_at
                .is_none_or(|expires| expires < unix_now())
    });
    let refresh_status = match source.map(|source| source.status.as_str()) {
        None => CfpRefreshStatus::Unadapted,
        Some("success") => CfpRefreshStatus::Success,
        Some("failed") => CfpRefreshStatus::Failed,
        Some("refreshing") if has_expired_lease => CfpRefreshStatus::Failed,
        Some("refreshing") => CfpRefreshStatus::Refreshing,
        Some("unsupported") => CfpRefreshStatus::Unsupported,
        _ => CfpRefreshStatus::Snapshot,
    };
    Ok(CfpJournalSummary {
        catalog_id: entry.catalog_id.clone(),
        catalog_aliases: entry.catalog_aliases.clone(),
        all_issns: entry.all_issns.clone(),
        title_aliases: entry.title_aliases.clone(),
        title: entry.title.clone(),
        area: entry.area.clone(),
        coverage: if snapshot.is_some() {
            CfpCoverage::Adapted
        } else {
            CfpCoverage::Unadapted
        },
        checked_on: snapshot.map(|snapshot| snapshot.checked_on.clone()),
        source_url: snapshot.and_then(|snapshot| snapshot.source_url.clone()),
        source_statement: snapshot.and_then(|snapshot| snapshot.source_statement.clone()),
        notice_count: snapshot.map_or(0, |snapshot| snapshot.notices.len()),
        current_count,
        state_counts: counts,
        can_refresh: registration.is_some_and(|source| source.can_refresh()),
        refresh_status,
        last_attempt: source.and_then(|source| source.last_attempt),
        last_success: source.and_then(|source| source.last_success),
        last_error: if has_expired_lease {
            Some("Previous source refresh ended before publication".into())
        } else {
            source.and_then(|source| source.last_error.clone())
        },
    })
}

fn catalog_response(
    storage: &StorageConfig,
    query: &CatalogQuery,
) -> Result<CfpCatalogResponse, ApiError> {
    let entries = catalog_members(storage, &query.db)?;
    let snapshots =
        load_cfp_journals(storage.auth_db_path()).map_err(|_| ApiError::internal_server_error())?;
    let evaluated_at = unix_now();
    let mut items = entries
        .iter()
        .map(|entry| journal_summary(entry, find_snapshot(entry, &snapshots)?, evaluated_at))
        .collect::<Result<Vec<_>, ApiError>>()?;
    let summary = CfpCatalogSummary {
        journals: items.len(),
        adapted_journals: items
            .iter()
            .filter(|journal| journal.coverage == CfpCoverage::Adapted)
            .count(),
        notices: items.iter().map(|journal| journal.notice_count).sum(),
        current_notices: items.iter().map(|journal| journal.current_count).sum(),
    };
    if let Some(query) = query
        .q
        .as_ref()
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty())
    {
        items.retain(|journal| {
            journal.title.to_lowercase().contains(&query)
                || journal.catalog_id.to_lowercase().contains(&query)
                || journal
                    .all_issns
                    .iter()
                    .any(|issn| issn.to_lowercase().contains(&query))
                || journal
                    .title_aliases
                    .iter()
                    .any(|alias| alias.to_lowercase().contains(&query))
                || journal
                    .area
                    .as_deref()
                    .is_some_and(|area| area.to_lowercase().contains(&query))
        });
    }
    items.sort_by(|first, second| {
        (second.coverage == CfpCoverage::Adapted)
            .cmp(&(first.coverage == CfpCoverage::Adapted))
            .then(first.title.to_lowercase().cmp(&second.title.to_lowercase()))
            .then(first.catalog_id.cmp(&second.catalog_id))
    });
    Ok(CfpCatalogResponse {
        database: query.db.clone(),
        evaluated_at,
        summary,
        items,
    })
}

fn content_revision(
    entry: &JournalCatalogEntry,
    snapshot: Option<&CfpJournalSnapshot>,
) -> Result<String, ApiError> {
    let revisions = snapshot.map(|snapshot| {
        snapshot
            .sources
            .iter()
            .map(|source| (&source.source_key, source.revision))
            .collect::<Vec<_>>()
    });
    let bytes =
        serde_json::to_vec(&(entry, revisions)).map_err(|_| ApiError::internal_server_error())?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn notice_response(
    storage: &StorageConfig,
    codec: &SecretCodec,
    catalog_id: &str,
    query: &NoticeQuery,
    limit: usize,
) -> Result<CfpNoticePage, ApiError> {
    let entries = catalog_members(storage, &query.db)?;
    let entry = entries
        .iter()
        .find(|entry| {
            entry.catalog_id == catalog_id
                || entry
                    .catalog_aliases
                    .iter()
                    .any(|alias| alias == catalog_id)
        })
        .ok_or_else(|| ApiError::not_found("CFP journal catalog member not found"))?;
    let snapshots =
        load_cfp_journals(storage.auth_db_path()).map_err(|_| ApiError::internal_server_error())?;
    let snapshot = find_snapshot(entry, &snapshots)?;
    let revision = content_revision(entry, snapshot)?;
    let (evaluated_at, position) = if let Some(cursor) = &query.cursor {
        let json = codec
            .decrypt(cursor, CURSOR_CONTEXT)
            .map_err(|_| ApiError::conflict(CURSOR_RELOAD_DETAIL))?;
        let cursor: NoticeCursor =
            serde_json::from_str(&json).map_err(|_| ApiError::conflict(CURSOR_RELOAD_DETAIL))?;
        if cursor.database != query.db
            || cursor.catalog_id != entry.catalog_id
            || cursor.include_closed != query.include_closed
            || cursor.revision != revision
            || cursor.order != "source-v1"
            || !(0..=900).contains(&unix_now().saturating_sub(cursor.evaluated_at))
        {
            return Err(ApiError::conflict(CURSOR_RELOAD_DETAIL));
        }
        (cursor.evaluated_at, cursor.position)
    } else {
        (unix_now(), 0)
    };
    let now = chrono::DateTime::from_timestamp(evaluated_at, 0)
        .ok_or_else(|| ApiError::conflict(CURSOR_RELOAD_DETAIL))?;
    let journal = journal_summary(entry, snapshot, evaluated_at)?;
    let notices: Vec<_> = snapshot
        .into_iter()
        .flat_map(|snapshot| snapshot.notices.iter())
        .filter_map(|notice| {
            let state = notice.state(now);
            (query.include_closed || !state.is_archived()).then(|| CfpNoticeView {
                notice: notice.clone(),
                state,
                entry_deadline: notice.entry_deadline().cloned(),
            })
        })
        .collect();
    if position > notices.len() {
        return Err(ApiError::conflict(CURSOR_RELOAD_DETAIL));
    }
    let total = notices.len();
    let next_position = position.saturating_add(limit).min(total);
    let has_more = next_position < total;
    let next_cursor = if has_more {
        let cursor = NoticeCursor {
            database: query.db.clone(),
            catalog_id: entry.catalog_id.clone(),
            include_closed: query.include_closed,
            revision,
            evaluated_at,
            position: next_position,
            order: "source-v1".into(),
        };
        let json = serde_json::to_string(&cursor).map_err(|_| ApiError::internal_server_error())?;
        Some(
            codec
                .encrypt(&json, CURSOR_CONTEXT)
                .map_err(|_| ApiError::internal_server_error())?,
        )
    } else {
        None
    };
    Ok(CfpNoticePage {
        journal,
        evaluated_at,
        items: notices.into_iter().skip(position).take(limit).collect(),
        page: PageMeta {
            total: Some(total as i64),
            limit: limit as i64,
            offset: position as i64,
            next_cursor,
            has_more: Some(has_more),
        },
    })
}

#[cfg(test)]
mod tests;
