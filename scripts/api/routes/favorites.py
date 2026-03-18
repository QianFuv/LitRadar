"""Favorites and folder management routes."""

from __future__ import annotations

import sqlite3
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Query

from scripts.api.auth_db import (
    add_favorite,
    batch_is_favorited,
    bulk_add_favorites,
    count_favorites,
    create_folder,
    delete_folder,
    get_tracking_folder,
    is_favorited,
    list_favorites,
    list_folders,
    remove_favorite,
    rename_folder,
    set_tracking_folder,
)
from scripts.api.auth_deps import get_current_user
from scripts.api.models import (
    FavoriteAdd,
    FavoriteArticleResponse,
    FavoriteBatchCheckRequest,
    FavoriteBatchCheckResponse,
    FavoriteBulkAdd,
    FavoriteCheckResponse,
    FavoriteResponse,
    FolderCreate,
    FolderRename,
    FolderResponse,
    TrackingSetRequest,
)
from scripts.shared.constants import API_PREFIX
from scripts.shared.db_path import resolve_db_path

router = APIRouter(prefix=f"{API_PREFIX}/favorites", tags=["favorites"])

CurrentUser = Annotated[dict, Depends(get_current_user)]


def _load_article_details_by_db(
    db_name: str,
    article_ids: list[int],
) -> dict[int, dict]:
    """
    Load article details for one favorites database.

    Args:
        db_name: Database name stored with the favorite row.
        article_ids: Article identifiers from that database.

    Returns:
        Mapping of article_id to article metadata.
    """
    if not article_ids:
        return {}

    unique_ids = list(dict.fromkeys(article_ids))
    try:
        db_path = resolve_db_path(db_name or None)
    except ValueError:
        return {}

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    try:
        placeholders = ", ".join("?" for _ in unique_ids)
        rows = conn.execute(
            f"""
            SELECT
                a.article_id,
                a.journal_id,
                a.issue_id,
                a.title,
                a.date,
                a.authors,
                a.abstract,
                a.doi,
                a.platform_id,
                a.open_access,
                a.in_press,
                a.full_text_file,
                j.title AS journal_title,
                i.volume,
                i.number
            FROM articles a
            LEFT JOIN issues i ON i.issue_id = a.issue_id
            JOIN journals j ON j.journal_id = a.journal_id
            WHERE a.article_id IN ({placeholders})
            """,
            unique_ids,
        ).fetchall()
    except sqlite3.Error:
        return {}
    finally:
        conn.close()

    return {int(row["article_id"]): dict(row) for row in rows}


def _build_favorite_article_responses(
    rows: list[dict],
) -> list[FavoriteArticleResponse]:
    """
    Merge favorite rows with article metadata in batch.

    Args:
        rows: Favorite rows from auth.sqlite.

    Returns:
        Enriched favorites responses in original order.
    """
    article_ids_by_db: dict[str, list[int]] = {}
    for row in rows:
        db_name = str(row.get("db_name") or "")
        article_ids_by_db.setdefault(db_name, []).append(int(row["article_id"]))

    article_details_by_db = {
        db_name: _load_article_details_by_db(db_name, article_ids)
        for db_name, article_ids in article_ids_by_db.items()
    }

    responses: list[FavoriteArticleResponse] = []
    for row in rows:
        db_name = str(row.get("db_name") or "")
        article_details = article_details_by_db.get(db_name, {}).get(
            int(row["article_id"]),
            {},
        )
        payload = dict(row)
        payload.update(article_details)
        responses.append(FavoriteArticleResponse(**payload))
    return responses


@router.get("/folders", response_model=list[FolderResponse])
async def api_list_folders(user: CurrentUser):
    """List all folders for the authenticated user."""
    rows = list_folders(user["id"])
    return [
        FolderResponse(
            id=r["id"],
            name=r["name"],
            is_tracking=bool(r["is_tracking"]),
            article_count=r["article_count"],
            created_at=r["created_at"],
        )
        for r in rows
    ]


@router.post("/folders", response_model=FolderResponse)
async def api_create_folder(body: FolderCreate, user: CurrentUser):
    """Create a new folder."""
    name = body.name.strip()
    if not name or len(name) > 100:
        raise HTTPException(
            status_code=400,
            detail="Folder name must be 1-100 characters",
        )
    try:
        r = create_folder(user["id"], name, body.is_tracking)
    except sqlite3.IntegrityError:
        raise HTTPException(
            status_code=409, detail="Folder name already exists"
        ) from None
    return FolderResponse(
        id=r["id"],
        name=r["name"],
        is_tracking=r["is_tracking"],
        article_count=0,
        created_at=r["created_at"],
    )


@router.put("/folders/{folder_id}")
async def api_rename_folder(folder_id: int, body: FolderRename, user: CurrentUser):
    """Rename an existing folder."""
    name = body.name.strip()
    if not name or len(name) > 100:
        raise HTTPException(
            status_code=400,
            detail="Folder name must be 1-100 characters",
        )
    try:
        ok = rename_folder(user["id"], folder_id, name)
    except sqlite3.IntegrityError:
        raise HTTPException(
            status_code=409, detail="Folder name already exists"
        ) from None
    if not ok:
        raise HTTPException(status_code=404, detail="Folder not found")
    return {"ok": True}


@router.delete("/folders/{folder_id}")
async def api_delete_folder(folder_id: int, user: CurrentUser):
    """Delete a folder and all its favorites."""
    ok = delete_folder(user["id"], folder_id)
    if not ok:
        raise HTTPException(status_code=404, detail="Folder not found")
    return {"ok": True}


@router.get("/tracking")
async def api_get_tracking(user: CurrentUser):
    """Get the current tracking folder for the user."""
    folder = get_tracking_folder(user["id"])
    if not folder:
        return {"folder_id": None, "folder_name": None}
    return {"folder_id": folder["id"], "folder_name": folder["name"]}


@router.put("/tracking")
async def api_set_tracking(body: TrackingSetRequest, user: CurrentUser):
    """Set a folder as the tracking folder."""
    ok = set_tracking_folder(user["id"], body.folder_id)
    if not ok:
        raise HTTPException(status_code=404, detail="Folder not found")
    return {"ok": True}


@router.get(
    "/folders/{folder_id}/articles",
    response_model=list[FavoriteArticleResponse],
)
async def api_list_folder_articles(
    folder_id: int,
    user: CurrentUser,
    limit: int = Query(default=100, ge=1, le=500),
    offset: int = Query(default=0, ge=0),
):
    """List favorited articles in a folder."""
    rows = list_favorites(user["id"], folder_id, limit, offset)
    return _build_favorite_article_responses(rows)


@router.get("/folders/{folder_id}/count")
async def api_folder_count(folder_id: int, user: CurrentUser):
    """Get the article count for a folder."""
    return {"count": count_favorites(user["id"], folder_id)}


@router.post(
    "/folders/{folder_id}/articles",
    response_model=FavoriteResponse,
)
async def api_add_favorite(
    folder_id: int,
    body: FavoriteAdd,
    user: CurrentUser,
):
    """Add an article to a folder."""
    try:
        r = add_favorite(
            user["id"],
            folder_id,
            body.article_id,
            body.db_name,
            body.note,
        )
    except ValueError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return FavoriteResponse(**r)


@router.delete("/folders/{folder_id}/articles/{article_id}")
async def api_remove_favorite(
    folder_id: int,
    article_id: int,
    user: CurrentUser,
    db_name: str = Query(default=""),
):
    """Remove a favorited article from a folder."""
    ok = remove_favorite(user["id"], folder_id, article_id, db_name)
    if not ok:
        raise HTTPException(status_code=404, detail="Favorite not found")
    return {"ok": True}


@router.post("/folders/{folder_id}/articles/bulk")
async def api_bulk_add(
    folder_id: int,
    body: FavoriteBulkAdd,
    user: CurrentUser,
):
    """Bulk add articles to a folder."""
    try:
        count = bulk_add_favorites(
            user["id"],
            folder_id,
            [a.model_dump() for a in body.articles],
        )
    except ValueError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return {"added": count}


@router.get("/check", response_model=list[FavoriteCheckResponse])
async def api_check_favorite(
    user: CurrentUser,
    article_id: int = Query(...),
    db_name: str = Query(default=""),
):
    """Check which folders an article is favorited in."""
    rows = is_favorited(user["id"], article_id, db_name)
    return [FavoriteCheckResponse(**r) for r in rows]


@router.post("/check/batch", response_model=list[FavoriteBatchCheckResponse])
async def api_check_favorites_batch(
    body: FavoriteBatchCheckRequest,
    user: CurrentUser,
):
    """Check which folders multiple articles are favorited in."""
    article_ids = [
        article_id
        for article_id in dict.fromkeys(body.article_ids)
        if isinstance(article_id, int) and article_id > 0
    ]
    favorite_map = batch_is_favorited(user["id"], article_ids, body.db_name)
    return [
        FavoriteBatchCheckResponse(
            article_id=article_id,
            folders=[
                FavoriteCheckResponse(**row) for row in favorite_map.get(article_id, [])
            ],
        )
        for article_id in article_ids
    ]
