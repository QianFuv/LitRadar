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
from scripts.shared.db_path import list_database_files

logger = logging.getLogger(__name__)

router = APIRouter(prefix=f"{API_PREFIX}/tracking", tags=["tracking"])

CurrentUser = Annotated[dict, Depends(get_current_user)]

ALLOWED_DELIVERY_METHODS = {"folder", "pushplus"}


def _load_latest_weekly_articles() -> list[dict]:
    """
    Load the latest weekly articles from push state manifests.

    Returns:
        List of dicts with article_id and db_name.
    """
    push_state_dir = PROJECT_ROOT / "data" / "push_state"
    if not push_state_dir.exists():
        return []

    articles: list[dict] = []
    db_files = list_database_files()

    for db_path in db_files:
        state_file = push_state_dir / f"{db_path.stem}.json"
        if not state_file.exists():
            continue

        try:
            with open(state_file) as f:
                state = json.load(f)
        except (json.JSONDecodeError, OSError):
            continue

        run = state.get("run")
        if not isinstance(run, dict):
            continue

        delivered_articles = run.get("delivered_article_ids")
        if not isinstance(delivered_articles, list):
            continue

        for aid in delivered_articles:
            if isinstance(aid, int):
                articles.append(
                    {
                        "article_id": aid,
                        "db_name": db_path.name,
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
    from scripts.notify.ai_selector import SiliconFlowSelector
    from scripts.notify.models import (
        ArticleCandidate,
        Subscriber,
    )
    from scripts.notify.selection import (
        apply_selection_rules,
        select_articles_with_retries,
    )
    from scripts.notify.subscriptions import load_notification_config

    global_config, defaults = load_notification_config()
    if not global_config.siliconflow_api_key:
        raise RuntimeError("SiliconFlow AI selection is not configured")

    subscriber = Subscriber(
        subscriber_id=str(settings["user_id"]),
        name=settings.get("username", str(settings["user_id"])),
        pushplus_token=settings.get("pushplus_token", ""),
        to=settings.get("pushplus_to") or None,
        keywords=settings.get("keywords", []),
        directions=settings.get("directions", []),
        topic=settings.get("pushplus_topic") or None,
        template=settings.get("pushplus_template") or None,
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

    selector = SiliconFlowSelector(
        api_key=global_config.siliconflow_api_key,
        model=defaults.siliconflow_model,
        timeout_seconds=120,
        retries=2,
        temperature=defaults.temperature,
    )

    try:
        selection_result = select_articles_with_retries(
            selector,
            subscriber,
            defaults,
            candidates_for_model,
            candidates_by_id,
            {},
            5,
        )
        accepted = apply_selection_rules(
            selection_result,
            subscriber,
            candidates_by_id,
            {},
        )

        selected_candidates = [
            candidates_by_id[item.article_id]
            for item in accepted
            if item.article_id in candidates_by_id
        ]

        summary = selection_result.summary
        if selected_candidates:
            try:
                better = selector.summarize_selected_articles(
                    subscriber, selected_candidates
                )
                if better:
                    summary = better
            except Exception:
                pass

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
        selector.close()


@router.post("/push-weekly")
async def push_weekly_to_tracking(user: CurrentUser):
    """
    Push weekly articles to the user's tracking folder.

    If the user has notification settings with keywords/directions,
    AI selection is applied to filter and rank articles first.
    Otherwise all weekly articles are pushed directly.
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
    has_preferences = (
        settings
        and settings.get("enabled", True)
        and (settings.get("keywords") or settings.get("directions"))
    )

    if has_preferences and settings:
        candidate_articles = _load_candidate_articles(weekly_articles)
        if not candidate_articles:
            return {
                "pushed": 0,
                "selected": 0,
                "summary": "",
                "message": "No article data found for weekly articles",
            }

        try:
            ai_result = _run_ai_selection(settings, candidate_articles)
        except Exception:
            logger.exception("AI selection failed, falling back to all")
            ai_result = None

        if ai_result is None:
            count = bulk_add_favorites(user["id"], folder["id"], weekly_articles)
            return {
                "pushed": count,
                "selected": len(weekly_articles),
                "summary": "",
                "message": "AI selection failed; pushed all weekly articles",
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

    count = bulk_add_favorites(user["id"], folder["id"], weekly_articles)
    return {
        "pushed": count,
        "selected": len(weekly_articles),
        "summary": "",
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
    return upsert_notification_settings(
        user_id=user["id"],
        keywords=[k.strip() for k in body.keywords if k.strip()],
        directions=[d.strip() for d in body.directions if d.strip()],
        delivery_method=body.delivery_method,
        pushplus_token=body.pushplus_token.strip(),
        pushplus_template=body.pushplus_template.strip() or "markdown",
        pushplus_topic=body.pushplus_topic.strip(),
        pushplus_to=body.pushplus_to.strip(),
        enabled=body.enabled,
    )
