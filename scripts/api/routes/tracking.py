"""Literature tracking routes – push weekly articles, notification settings."""

from __future__ import annotations

import json
import logging
import sqlite3
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException

from scripts.api.auth_db import (
    bulk_add_favorites,
    get_notification_settings,
    get_tracking_folder,
    list_folders,
    upsert_notification_settings,
)
from scripts.api.auth_deps import get_current_user
from scripts.api.models import (
    NotificationSettingsResponse,
    NotificationSettingsUpdate,
)
from scripts.shared.constants import API_PREFIX, PROJECT_ROOT

logger = logging.getLogger(__name__)

router = APIRouter(prefix=f"{API_PREFIX}/tracking", tags=["tracking"])

CurrentUser = Annotated[dict, Depends(get_current_user)]

ALLOWED_DELIVERY_METHODS = {"folder", "pushplus"}


def _load_latest_weekly_articles() -> list[dict]:
    """
    Load the latest weekly articles from change manifests.

    Returns:
        List of dicts with article_id and db_name.
    """
    push_state_dir = PROJECT_ROOT / "data" / "push_state"
    if not push_state_dir.exists():
        return []

    articles: list[dict] = []
    seen_pairs: set[tuple[str, int]] = set()

    for state_file in sorted(push_state_dir.glob("*.changes.json")):
        try:
            with open(state_file, encoding="utf-8") as f:
                state = json.load(f)
        except (json.JSONDecodeError, OSError):
            continue

        if not isinstance(state, dict):
            continue

        db_name = str(state.get("db_name") or "").strip()
        if not db_name:
            continue

        delivered_articles = state.get("notifiable_article_ids")
        if not isinstance(delivered_articles, list):
            continue

        for aid in delivered_articles:
            if isinstance(aid, int):
                pair = (db_name, aid)
                if pair in seen_pairs:
                    continue
                seen_pairs.add(pair)
                articles.append(
                    {
                        "article_id": aid,
                        "db_name": db_name,
                    }
                )

    return articles


def _load_candidate_articles(
    article_items: list[dict],
) -> list[dict]:
    """
    Load full article data from index databases for given article items.

    Args:
        article_items: List of dicts with article_id and db_name.

    Returns:
        List of article dicts with all fields needed for AI selection.
    """
    from scripts.shared.db_path import resolve_db_path

    by_db: dict[str, list[int]] = {}
    for item in article_items:
        db_name = item["db_name"]
        by_db.setdefault(db_name, []).append(item["article_id"])

    results: list[dict] = []
    for db_name, article_ids in by_db.items():
        try:
            db_path = resolve_db_path(db_name)
        except ValueError:
            continue

        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        try:
            placeholders = ", ".join(["?"] * len(article_ids))
            rows = conn.execute(
                f"""
                SELECT
                    a.article_id, a.journal_id, a.issue_id,
                    a.title, a.abstract, a.date,
                    a.open_access, a.in_press,
                    a.within_library_holdings,
                    a.doi, a.full_text_file, a.permalink,
                    j.title AS journal_title
                FROM articles a
                JOIN journals j ON j.journal_id = a.journal_id
                WHERE a.article_id IN ({placeholders})
                """,
                article_ids,
            ).fetchall()
            for row in rows:
                d = dict(row)
                d["db_name"] = db_name
                results.append(d)
        finally:
            conn.close()

    return results


def _run_ai_selection(
    settings: dict,
    candidate_articles: list[dict],
) -> dict:
    """
    Run AI article selection for one user using their notification settings.

    Args:
        settings: User notification settings dict.
        candidate_articles: Full article dicts from index DBs.

    Returns:
        Dict with 'selected' (list of article dicts with score),
        'summary' (str), and 'total_candidates' (int).
    """
    from scripts.notify.ai_selector import OpenAICompatibleSelector
    from scripts.notify.models import (
        ArticleCandidate,
        Subscriber,
    )
    from scripts.notify.selection import (
        select_articles_for_subscriber,
    )
    from scripts.notify.subscriptions import (
        load_notification_config,
    )

    global_config, defaults = load_notification_config()
    subscriber = Subscriber(
        subscriber_id=str(settings["user_id"]),
        name=settings.get("username", str(settings["user_id"])),
        pushplus_token=settings.get("pushplus_token", ""),
        channel=settings.get("pushplus_channel") or None,
        keywords=settings.get("keywords", []),
        directions=settings.get("directions", []),
        topic=settings.get("pushplus_topic") or None,
        template=settings.get("pushplus_template") or None,
        ai_base_url=(str(settings.get("ai_base_url") or "").strip() or None),
        ai_api_key=(str(settings.get("ai_api_key") or "").strip() or None),
        ai_model=(str(settings.get("ai_model") or "").strip() or None),
        ai_system_prompt=(str(settings.get("ai_system_prompt") or "").strip() or None),
        ai_backup_base_url=(
            str(settings.get("ai_backup_base_url") or "").strip() or None
        ),
        ai_backup_api_key=(
            str(settings.get("ai_backup_api_key") or "").strip() or None
        ),
        ai_backup_model=(str(settings.get("ai_backup_model") or "").strip() or None),
        ai_backup_system_prompt=(
            str(settings.get("ai_backup_system_prompt") or "").strip() or None
        ),
        ai_retry_attempts=max(1, int(settings.get("ai_retry_attempts") or 3)),
    )

    from scripts.shared.converters import to_int as _to_int

    candidates = []
    original_articles_by_candidate_id: dict[int, dict] = {}
    for synthetic_article_id, candidate_article in enumerate(
        candidate_articles, start=1
    ):
        original_articles_by_candidate_id[synthetic_article_id] = candidate_article
        candidates.append(
            ArticleCandidate(
                article_id=synthetic_article_id,
                journal_id=int(candidate_article["journal_id"]),
                issue_id=_to_int(candidate_article.get("issue_id")),
                title=str(candidate_article.get("title") or "Untitled"),
                abstract=str(candidate_article.get("abstract") or ""),
                date=str(candidate_article.get("date") or "") or None,
                journal_title=str(candidate_article.get("journal_title") or "Unknown"),
                doi=str(candidate_article.get("doi") or "") or None,
                full_text_file=(
                    str(candidate_article.get("full_text_file") or "") or None
                ),
                permalink=str(candidate_article.get("permalink") or "") or None,
                open_access=bool(_to_int(candidate_article.get("open_access")) or 0),
                in_press=bool(_to_int(candidate_article.get("in_press")) or 0),
                within_library_holdings=bool(
                    _to_int(candidate_article.get("within_library_holdings")) or 0
                ),
            )
        )

    if not candidates:
        return {"selected": [], "summary": "", "total_candidates": 0}

    candidates_by_id = {c.article_id: c for c in candidates}
    max_candidates = min(defaults.max_candidates, len(candidates))
    candidates_for_model = candidates[:max_candidates]
    selector_cache: dict[tuple[str, str, str, str, int], OpenAICompatibleSelector] = {}
    try:
        accepted, summary, skip_reason = select_articles_for_subscriber(
            subscriber=subscriber,
            global_config=global_config,
            defaults=defaults,
            candidates_for_model=candidates_for_model,
            candidates_by_id=candidates_by_id,
            delivery_dedupe={},
            selector_cache=selector_cache,
            timeout_seconds=120,
            retry_attempts=subscriber.ai_retry_attempts,
        )
        if skip_reason is not None:
            raise RuntimeError(skip_reason)

        selected = []
        for item in accepted:
            selected_article = original_articles_by_candidate_id.get(item.article_id)
            if selected_article is None:
                continue
            selected.append(
                {
                    **selected_article,
                    "score": item.score,
                }
            )

        return {
            "selected": selected,
            "summary": summary,
            "total_candidates": len(candidates),
        }
    finally:
        for selector in selector_cache.values():
            selector.close()


@router.post("/push-weekly")
async def push_weekly_to_tracking(user: CurrentUser):
    """
    Push weekly articles to the user's tracking folder.

    AI selection is required for delivery. When no usable recommendation
    configuration is available, the push is skipped.
    """
    folder = get_tracking_folder(user["id"])
    if not folder:
        raise HTTPException(
            status_code=400,
            detail=(
                "No tracking folder configured."
                " Create a folder and set it as tracking first."
            ),
        )

    weekly_articles = _load_latest_weekly_articles()
    if not weekly_articles:
        return {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "No new weekly articles available",
        }

    settings = get_notification_settings(user["id"])
    if not settings or not settings.get("enabled", True):
        return {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "Recommendation settings are not enabled; skipped push",
            "folder_id": folder["id"],
            "folder_name": folder["name"],
        }

    if not (settings.get("keywords") or settings.get("directions")):
        return {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "No keywords or directions configured; skipped push",
            "folder_id": folder["id"],
            "folder_name": folder["name"],
        }

    candidate_articles = _load_candidate_articles(weekly_articles)
    if not candidate_articles:
        return {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "No article data found for weekly articles",
            "folder_id": folder["id"],
            "folder_name": folder["name"],
        }

    try:
        ai_result = _run_ai_selection(settings, candidate_articles)
    except RuntimeError as error:
        logger.warning(
            "AI selection is unavailable for user %s: %s",
            user["id"],
            error,
        )
        return {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": f"{error}; skipped push",
            "folder_id": folder["id"],
            "folder_name": folder["name"],
        }
    except Exception:
        logger.exception("AI selection failed for user %s", user["id"])
        return {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "AI selection failed across configured endpoints; skipped push",
            "folder_id": folder["id"],
            "folder_name": folder["name"],
        }

    if ai_result["selected"]:
        articles_to_push = [
            {
                "article_id": art["article_id"],
                "db_name": art["db_name"],
            }
            for art in ai_result["selected"]
        ]
        count = bulk_add_favorites(user["id"], folder["id"], articles_to_push)
        return {
            "pushed": count,
            "selected": len(ai_result["selected"]),
            "total_candidates": ai_result["total_candidates"],
            "summary": ai_result["summary"],
            "folder_id": folder["id"],
            "folder_name": folder["name"],
        }

    return {
        "pushed": 0,
        "selected": 0,
        "summary": ai_result["summary"],
        "total_candidates": ai_result["total_candidates"],
        "message": "AI selection found no matching articles",
        "folder_id": folder["id"],
        "folder_name": folder["name"],
    }


@router.get("/status")
async def tracking_status(user: CurrentUser):
    """Get tracking status for the user."""
    folder = get_tracking_folder(user["id"])
    folders = list_folders(user["id"])
    settings = get_notification_settings(user["id"])
    return {
        "tracking_folder": (
            {"id": folder["id"], "name": folder["name"]} if folder else None
        ),
        "total_folders": len(folders),
        "weekly_articles_available": len(_load_latest_weekly_articles()),
        "notification_configured": settings is not None,
    }


@router.get(
    "/notification-settings",
    response_model=NotificationSettingsResponse | None,
)
async def get_settings(user: CurrentUser):
    """Get the user's notification settings."""
    return get_notification_settings(user["id"])


@router.put(
    "/notification-settings",
    response_model=NotificationSettingsResponse,
)
async def update_settings(
    body: NotificationSettingsUpdate,
    user: CurrentUser,
):
    """Create or update the user's notification settings."""
    if body.delivery_method not in ALLOWED_DELIVERY_METHODS:
        raise HTTPException(
            status_code=400,
            detail=(
                f"delivery_method must be one of: "
                f"{', '.join(sorted(ALLOWED_DELIVERY_METHODS))}"
            ),
        )
    if body.delivery_method == "pushplus" and not body.pushplus_token.strip():
        raise HTTPException(
            status_code=400,
            detail="pushplus_token is required when delivery_method is 'pushplus'",
        )
    if (
        body.delivery_method == "pushplus"
        and body.sync_to_tracking_folder
        and get_tracking_folder(user["id"]) is None
    ):
        raise HTTPException(
            status_code=400,
            detail=(
                "A tracking folder is required before enabling "
                "PushPlus sync to tracking"
            ),
        )
    return upsert_notification_settings(
        user_id=user["id"],
        keywords=[k.strip() for k in body.keywords if k.strip()],
        directions=[d.strip() for d in body.directions if d.strip()],
        delivery_method=body.delivery_method,
        pushplus_token=body.pushplus_token.strip(),
        pushplus_template=body.pushplus_template.strip() or "markdown",
        pushplus_topic=body.pushplus_topic.strip(),
        pushplus_channel=body.pushplus_channel.strip(),
        sync_to_tracking_folder=body.sync_to_tracking_folder,
        ai_base_url=body.ai_base_url.strip(),
        ai_api_key=body.ai_api_key.strip(),
        ai_model=body.ai_model.strip(),
        ai_system_prompt=body.ai_system_prompt.strip(),
        ai_backup_base_url=body.ai_backup_base_url.strip(),
        ai_backup_api_key=body.ai_backup_api_key.strip(),
        ai_backup_model=body.ai_backup_model.strip(),
        ai_backup_system_prompt=body.ai_backup_system_prompt.strip(),
        ai_retry_attempts=body.ai_retry_attempts,
        enabled=body.enabled,
    )
