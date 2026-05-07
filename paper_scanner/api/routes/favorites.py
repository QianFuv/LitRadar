"""Favorites and folder management routes."""

from __future__ import annotations

import re
import sqlite3
from typing import Annotated, Literal

from fastapi import APIRouter, Depends, Header, HTTPException, Query, Response

from paper_scanner.api.auth_db import (
    add_favorite,
    batch_is_favorited,
    bulk_add_favorites,
    bulk_move_favorites,
    bulk_remove_favorites,
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
    verify_access_token,
)
from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.citations import to_bibtex, to_endnote, to_ris
from paper_scanner.api.models import (
    FavoriteAdd,
    FavoriteArticleResponse,
    FavoriteBatchCheckRequest,
    FavoriteBatchCheckResponse,
    FavoriteBulkAdd,
    FavoriteBulkMove,
    FavoriteBulkRemove,
    FavoriteBulkResult,
    FavoriteCheckResponse,
    FavoriteResponse,
    FolderCreate,
    FolderRename,
    FolderResponse,
    TrackingSetRequest,
)
from paper_scanner.shared.constants import API_PREFIX
from paper_scanner.shared.db_path import resolve_db_path

router = APIRouter(prefix=f"{API_PREFIX}/favorites", tags=["favorites"])

CurrentUser = Annotated[dict, Depends(get_current_user)]


async def get_export_user(
    authorization: str | None = Header(default=None),
    access_token: str | None = Query(default=None),
) -> dict:
    """
    Resolve the export user from either bearer auth or a token query string.

    Args:
        authorization: Optional Authorization header.
        access_token: Optional raw access token query parameter.

    Returns:
        Authenticated user mapping.
    """
    if authorization:
        return await get_current_user(authorization)

    if access_token:
        user = verify_access_token(access_token)
        if user:
            return user

    raise HTTPException(status_code=401, detail="Authentication required")


ExportUser = Annotated[dict, Depends(get_export_user)]


def _build_export_filename(folder_name: str, format_name: str) -> str:
    """
    Build a safe download filename for a folder export.

    Args:
        folder_name: Folder display name.
        format_name: Export format name.

    Returns:
        Sanitized filename.
    """
    safe_name = re.sub(r"[^a-zA-Z0-9._-]+", "_", folder_name).strip("._")
    base_name = safe_name or "favorites"
    extension = {
        "bibtex": "bib",
        "ris": "ris",
        "endnote": "xml",
    }[format_name]
    return f"{base_name}.{extension}"


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
                j.issn,
                j.eissn,
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
            id=row["id"],
            name=row["name"],
            is_tracking=bool(row["is_tracking"]),
            article_count=row["article_count"],
            created_at=row["created_at"],
        )
        for row in rows
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
        row = create_folder(user["id"], name, body.is_tracking)
    except sqlite3.IntegrityError:
        raise HTTPException(
            status_code=409,
            detail="Folder name already exists",
        ) from None
    return FolderResponse(
        id=row["id"],
        name=row["name"],
        is_tracking=row["is_tracking"],
        article_count=0,
        created_at=row["created_at"],
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
            status_code=409,
            detail="Folder name already exists",
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


@router.get("/folders/{folder_id}/export")
async def api_export_folder(
    folder_id: int,
    user: ExportUser,
    format: Literal["bibtex", "ris", "endnote"] = Query(default="bibtex"),
):
    """
    Export one folder's favorites in a citation format.

    Args:
        folder_id: Folder identifier.
        user: Authenticated user.
        format: Export format name.

    Returns:
        Download response with formatted citation content.
    """
    folder = next(
        (item for item in list_folders(user["id"]) if int(item["id"]) == folder_id),
        None,
    )
    if folder is None:
        raise HTTPException(status_code=404, detail="Folder not found")

    rows = list_favorites(user["id"], folder_id, limit=100_000, offset=0)
    articles = _build_favorite_article_responses(rows)

    if format == "bibtex":
        content = to_bibtex(articles)
        media_type = "application/x-bibtex"
    elif format == "ris":
        content = to_ris(articles)
        media_type = "application/x-research-info-systems"
    else:
        content = to_endnote(articles)
        media_type = "application/xml"

    filename = _build_export_filename(str(folder["name"]), format)
    return Response(
        content=content,
        media_type=media_type,
        headers={
            "Content-Disposition": f'attachment; filename="{filename}"',
        },
    )


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
        row = add_favorite(
            user["id"],
            folder_id,
            body.article_id,
            body.db_name,
            body.note,
        )
    except ValueError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return FavoriteResponse(**row)


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
            [article.model_dump() for article in body.articles],
        )
    except ValueError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return {"added": count}


@router.post(
    "/folders/{folder_id}/articles/bulk-remove",
    response_model=FavoriteBulkResult,
)
async def api_bulk_remove(
    folder_id: int,
    body: FavoriteBulkRemove,
    user: CurrentUser,
):
    """Bulk remove favorited articles from a folder."""
    try:
        count = bulk_remove_favorites(
            user["id"],
            folder_id,
            [article.model_dump() for article in body.articles],
        )
    except ValueError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return FavoriteBulkResult(count=count)


@router.post(
    "/folders/{folder_id}/articles/bulk-move",
    response_model=FavoriteBulkResult,
)
async def api_bulk_move(
    folder_id: int,
    body: FavoriteBulkMove,
    user: CurrentUser,
):
    """Bulk move favorited articles to another folder."""
    try:
        count = bulk_move_favorites(
            user["id"],
            folder_id,
            body.target_folder_id,
            [article.model_dump() for article in body.articles],
        )
    except ValueError as exc:
        detail = str(exc)
        status_code = 400 if "different" in detail else 404
        raise HTTPException(status_code=status_code, detail=detail) from exc
    return FavoriteBulkResult(count=count)


@router.get("/check", response_model=list[FavoriteCheckResponse])
async def api_check_favorite(
    user: CurrentUser,
    article_id: int = Query(...),
    db_name: str = Query(default=""),
):
    """Check which folders an article is favorited in."""
    rows = is_favorited(user["id"], article_id, db_name)
    return [FavoriteCheckResponse(**row) for row in rows]


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
