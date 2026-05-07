"""Literature tracking routes – push weekly articles, notification settings."""

from __future__ import annotations

import json
import logging
import sqlite3
import threading
import time
import uuid
from typing import Annotated, cast

from fastapi import APIRouter, Depends, HTTPException

from paper_scanner.api.auth_db import (
    bulk_add_favorites,
    get_notification_settings,
    get_tracking_folder,
    list_folders,
    upsert_notification_settings,
)
from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.models import (
    NotificationSettingsResponse,
    NotificationSettingsUpdate,
)
from paper_scanner.shared.constants import API_PREFIX, PROJECT_ROOT
from paper_scanner.shared.converters import to_int
from paper_scanner.shared.db_path import (
    is_database_selected,
    list_database_files,
    normalize_database_names,
)

logger = logging.getLogger(__name__)

router = APIRouter(prefix=f"{API_PREFIX}/tracking", tags=["tracking"])

CurrentUser = Annotated[dict, Depends(get_current_user)]

ALLOWED_DELIVERY_METHODS = {"folder", "pushplus"}

_manual_push_jobs_lock = threading.Lock()
_manual_push_jobs: dict[int, dict[str, object | None]] = {}


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

        for key in ("notifiable_article_ids", "backfill_article_ids"):
            id_list = state.get(key)
            if not isinstance(id_list, list):
                continue
            for aid in id_list:
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


def _filter_weekly_articles_by_database(
    weekly_articles: list[dict],
    selected_databases: list[str] | tuple[str, ...] | set[str] | None,
) -> list[dict]:
    """
    Filter weekly article pairs by selected databases.

    Args:
        weekly_articles: Weekly article items with `db_name`.
        selected_databases: Selected database names. Empty means all.

    Returns:
        Filtered weekly article items.
    """
    return [
        article
        for article in weekly_articles
        if is_database_selected(selected_databases, str(article.get("db_name") or ""))
    ]


def _group_articles_by_database(article_items: list[dict]) -> dict[str, list[dict]]:
    """
    Group article payloads by database name.

    Args:
        article_items: Article payloads that include `db_name`.

    Returns:
        Mapping of database name to grouped article payloads.
    """
    grouped: dict[str, list[dict]] = {}
    for item in article_items:
        db_name = str(item.get("db_name") or "").strip()
        if not db_name:
            continue
        grouped.setdefault(db_name, []).append(item)
    return grouped


def _combine_database_summaries(summaries_by_db: dict[str, str]) -> str:
    """
    Combine per-database summaries into one status text.

    Args:
        summaries_by_db: Per-database summary text.

    Returns:
        Combined summary string.
    """
    parts: list[str] = []
    for db_name in sorted(summaries_by_db):
        summary = str(summaries_by_db[db_name] or "").strip()
        if not summary:
            continue
        parts.append(f"[{db_name}]\n{summary}")
    return "\n\n".join(parts)


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
    from paper_scanner.shared.db_path import resolve_db_path

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
    from paper_scanner.notify.ai_selector import OpenAICompatibleSelector
    from paper_scanner.notify.models import (
        ArticleCandidate,
        Subscriber,
    )
    from paper_scanner.notify.selection import (
        select_articles_for_subscriber,
    )
    from paper_scanner.notify.subscriptions import (
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
        selected_databases=settings.get("selected_databases", []),
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

    from paper_scanner.shared.converters import to_int as _to_int

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


def _send_pushplus_for_selected_articles(
    user: dict,
    settings: dict,
    selected_articles_by_db: dict[str, list[dict]],
    summaries_by_db: dict[str, str],
) -> int:
    """
    Send manually selected weekly articles through PushPlus.

    Args:
        user: Authenticated user payload.
        settings: User notification settings.
        selected_articles_by_db: Selected article payloads grouped by database.
        summaries_by_db: AI-generated summary text grouped by database.

    Returns:
        Number of PushPlus messages sent.
    """
    from paper_scanner.notify.message import build_markdown_content, build_message_title
    from paper_scanner.notify.models import (
        ArticleCandidate,
        RankedSelection,
        Subscriber,
    )
    from paper_scanner.notify.pushplus import PushPlusClient
    from paper_scanner.notify.state import utc_now_iso
    from paper_scanner.notify.subscriptions import load_notification_config
    from paper_scanner.shared.converters import to_float
    from paper_scanner.shared.converters import to_int as _to_int

    token = str(settings.get("pushplus_token") or "").strip()
    if not token:
        raise RuntimeError("PushPlus token is missing")

    global_config, _ = load_notification_config()
    subscriber = Subscriber(
        subscriber_id=str(user["id"]),
        name=str(user.get("username") or user["id"]),
        pushplus_token=token,
        channel=(str(settings.get("pushplus_channel") or "").strip() or None),
        keywords=settings.get("keywords", []),
        directions=settings.get("directions", []),
        selected_databases=settings.get("selected_databases", []),
        topic=(str(settings.get("pushplus_topic") or "").strip() or None),
        template=(str(settings.get("pushplus_template") or "").strip() or None),
        delivery_method="pushplus",
        sync_to_tracking_folder=bool(settings.get("sync_to_tracking_folder")),
    )

    run_id = utc_now_iso()
    push_client = PushPlusClient(timeout_seconds=60, retries=1)
    try:
        sent_count = 0
        for db_name in sorted(selected_articles_by_db):
            db_articles = selected_articles_by_db[db_name]
            selections: list[RankedSelection] = []
            candidates_by_id: dict[int, ArticleCandidate] = {}

            for candidate_id, article in enumerate(db_articles, start=1):
                candidates_by_id[candidate_id] = ArticleCandidate(
                    article_id=candidate_id,
                    journal_id=int(article["journal_id"]),
                    issue_id=_to_int(article.get("issue_id")),
                    title=str(article.get("title") or "Untitled"),
                    abstract=str(article.get("abstract") or ""),
                    date=str(article.get("date") or "") or None,
                    journal_title=str(article.get("journal_title") or "Unknown"),
                    doi=str(article.get("doi") or "") or None,
                    full_text_file=(str(article.get("full_text_file") or "") or None),
                    permalink=str(article.get("permalink") or "") or None,
                    open_access=bool(_to_int(article.get("open_access")) or 0),
                    in_press=bool(_to_int(article.get("in_press")) or 0),
                    within_library_holdings=bool(
                        _to_int(article.get("within_library_holdings")) or 0
                    ),
                )
                selections.append(
                    RankedSelection(
                        article_id=candidate_id,
                        score=float(to_float(article.get("score")) or 0.0),
                    )
                )

            selections.sort(key=lambda item: item.score, reverse=True)
            content = build_markdown_content(
                db_name=db_name,
                run_id=run_id,
                subscriber=subscriber,
                summary=str(summaries_by_db.get(db_name) or ""),
                selections=selections,
                candidates_by_id=candidates_by_id,
            )
            push_client.send(
                token=token,
                title=build_message_title(db_name, run_id),
                content=content,
                channel=subscriber.channel or global_config.pushplus_channel,
                template=subscriber.template or global_config.pushplus_template,
                topic=subscriber.topic or global_config.pushplus_topic,
                option=global_config.pushplus_option,
                to=None,
            )
            sent_count += 1
        return sent_count
    finally:
        push_client.close()


def _build_manual_push_status(
    *,
    job_id: str | None,
    status: str,
    message: str,
    started_at: float | None = None,
    finished_at: float | None = None,
    pushed: int = 0,
    selected: int = 0,
    total_candidates: int | None = None,
    summary: str = "",
    folder_id: int | None = None,
    folder_name: str | None = None,
) -> dict[str, object | None]:
    """
    Build one manual weekly-push status payload.

    Args:
        job_id: Background job identifier.
        status: Job status string.
        message: Human-readable status message.
        started_at: Job start timestamp.
        finished_at: Job finish timestamp.
        pushed: Number of pushed or synced articles.
        selected: Number of selected articles.
        total_candidates: Number of AI candidates.
        summary: AI-generated summary text.
        folder_id: Tracking folder identifier.
        folder_name: Tracking folder name.

    Returns:
        Manual push status payload.
    """
    return {
        "job_id": job_id,
        "status": status,
        "message": message,
        "started_at": started_at,
        "finished_at": finished_at,
        "pushed": pushed,
        "selected": selected,
        "total_candidates": total_candidates,
        "summary": summary,
        "folder_id": folder_id,
        "folder_name": folder_name,
    }


def _get_manual_push_status(user_id: int) -> dict[str, object | None]:
    """
    Read the current manual weekly-push status for one user.

    Args:
        user_id: User identifier.

    Returns:
        Status payload for the current or last job.
    """
    with _manual_push_jobs_lock:
        current = _manual_push_jobs.get(user_id)
        if current is None:
            return _build_manual_push_status(
                job_id=None,
                status="idle",
                message="No manual push task is running",
            )
        return dict(current)


def _set_manual_push_status(
    user_id: int,
    status_payload: dict[str, object | None],
) -> None:
    """
    Persist manual weekly-push status for one user.

    Args:
        user_id: User identifier.
        status_payload: Status payload to store.

    Returns:
        None.
    """
    with _manual_push_jobs_lock:
        _manual_push_jobs[user_id] = dict(status_payload)


def _execute_weekly_push(user: dict) -> tuple[str, dict[str, object | None]]:
    """
    Push weekly articles to the user's tracking folder.

    AI selection is required for delivery. When no usable recommendation
    configuration is available, the push is skipped.
    """
    settings = get_notification_settings(user["id"])
    if not settings or not settings.get("enabled", True):
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "Recommendation settings are not enabled; skipped push",
        }

    delivery_method = (
        str(settings.get("delivery_method") or "folder").strip() or "folder"
    )
    selected_databases = normalize_database_names(settings.get("selected_databases"))
    sync_to_tracking_folder = bool(settings.get("sync_to_tracking_folder"))
    folder = get_tracking_folder(user["id"])
    requires_tracking_folder = delivery_method == "folder" or sync_to_tracking_folder
    if requires_tracking_folder and not folder:
        raise HTTPException(
            status_code=400,
            detail=(
                "No tracking folder configured."
                " Create a folder and set it as tracking first."
            ),
        )

    weekly_articles = _filter_weekly_articles_by_database(
        _load_latest_weekly_articles(),
        selected_databases,
    )
    if not weekly_articles:
        message = "No new weekly articles available"
        if selected_databases:
            message = "No new weekly articles available in selected databases"
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": message,
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }

    if not (settings.get("keywords") or settings.get("directions")):
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "No keywords or directions configured; skipped push",
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }

    candidate_articles = _load_candidate_articles(weekly_articles)
    if not candidate_articles:
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "No article data found for weekly articles",
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }

    candidate_articles_by_db = _group_articles_by_database(candidate_articles)
    if not candidate_articles_by_db:
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "No article data found for weekly articles",
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }

    selection_results_by_db: dict[str, dict[str, object]] = {}
    total_candidates = 0
    try:
        for db_name in sorted(candidate_articles_by_db):
            db_candidate_articles = candidate_articles_by_db[db_name]
            db_ai_result = _run_ai_selection(settings, db_candidate_articles)
            total_candidates += int(db_ai_result["total_candidates"])
            if db_ai_result["selected"]:
                selection_results_by_db[db_name] = db_ai_result
    except RuntimeError as error:
        logger.warning(
            "AI selection is unavailable for user %s: %s",
            user["id"],
            error,
        )
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": f"{error}; skipped push",
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }
    except Exception:
        logger.exception("AI selection failed for user %s", user["id"])
        return "completed", {
            "pushed": 0,
            "selected": 0,
            "summary": "",
            "message": "AI selection failed across configured endpoints; skipped push",
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }

    if selection_results_by_db:
        selected_articles_by_db = {
            db_name: cast(list[dict], result["selected"])
            for db_name, result in selection_results_by_db.items()
        }
        selected_articles = [
            article
            for db_name in sorted(selected_articles_by_db)
            for article in selected_articles_by_db[db_name]
        ]
        summaries_by_db = {
            db_name: str(result.get("summary") or "")
            for db_name, result in selection_results_by_db.items()
        }
        combined_summary = _combine_database_summaries(summaries_by_db)
        articles_to_push = [
            {
                "article_id": art["article_id"],
                "db_name": art["db_name"],
            }
            for art in selected_articles
        ]
        synced_count = 0
        if folder is not None and (
            delivery_method == "folder" or sync_to_tracking_folder
        ):
            synced_count = bulk_add_favorites(
                user["id"], folder["id"], articles_to_push
            )

        if delivery_method == "pushplus":
            try:
                pushplus_count = _send_pushplus_for_selected_articles(
                    user=user,
                    settings=settings,
                    selected_articles_by_db=selected_articles_by_db,
                    summaries_by_db=summaries_by_db,
                )
            except Exception as error:
                logger.exception("PushPlus delivery failed for user %s", user["id"])
                return "failed", {
                    "pushed": synced_count,
                    "selected": len(selected_articles),
                    "total_candidates": total_candidates,
                    "summary": combined_summary,
                    "message": f"PushPlus delivery failed: {error}",
                    "folder_id": folder["id"] if folder else None,
                    "folder_name": folder["name"] if folder else None,
                }

            success_message = (
                f"PushPlus sent successfully ({pushplus_count} message"
                f"{'' if pushplus_count == 1 else 's'})"
            )
            success_message += (
                f"; selected {len(selected_articles)} article"
                f"{'' if len(selected_articles) == 1 else 's'} across "
                f"{len(selected_articles_by_db)} database"
                f"{'' if len(selected_articles_by_db) == 1 else 's'}"
            )
            if synced_count > 0:
                success_message += (
                    f"; synced {synced_count} article"
                    f"{'' if synced_count == 1 else 's'} to the tracking folder"
                )
            return "completed", {
                "pushed": synced_count,
                "selected": len(selected_articles),
                "total_candidates": total_candidates,
                "summary": combined_summary,
                "message": success_message,
                "folder_id": folder["id"] if folder else None,
                "folder_name": folder["name"] if folder else None,
            }

        return "completed", {
            "pushed": synced_count,
            "selected": len(selected_articles),
            "total_candidates": total_candidates,
            "summary": combined_summary,
            "folder_id": folder["id"] if folder else None,
            "folder_name": folder["name"] if folder else None,
        }

    return "completed", {
        "pushed": 0,
        "selected": 0,
        "summary": "",
        "total_candidates": total_candidates,
        "message": "AI selection found no matching articles",
        "folder_id": folder["id"] if folder else None,
        "folder_name": folder["name"] if folder else None,
    }


def _run_manual_push_job(user: dict, job_id: str, started_at: float) -> None:
    """
    Execute one manual weekly-push job in the background.

    Args:
        user: Authenticated user payload.
        job_id: Background job identifier.
        started_at: Job start timestamp.

    Returns:
        None.
    """
    try:
        final_status, result = _execute_weekly_push(user)
    except Exception as error:
        logger.exception("Manual weekly push crashed for user %s", user["id"])
        status_payload = _build_manual_push_status(
            job_id=job_id,
            status="failed",
            message=f"Manual push failed: {error}",
            started_at=started_at,
            finished_at=time.time(),
        )
    else:
        status_payload = _build_manual_push_status(
            job_id=job_id,
            status=final_status,
            message=str(result.get("message") or ""),
            started_at=started_at,
            finished_at=time.time(),
            pushed=to_int(result.get("pushed")) or 0,
            selected=to_int(result.get("selected")) or 0,
            total_candidates=to_int(result.get("total_candidates")),
            summary=str(result.get("summary") or ""),
            folder_id=to_int(result.get("folder_id")),
            folder_name=(
                str(result.get("folder_name") or "")
                if result.get("folder_name") is not None
                else None
            ),
        )

    with _manual_push_jobs_lock:
        current = _manual_push_jobs.get(int(user["id"]))
        if current is None or current.get("job_id") != job_id:
            return
        _manual_push_jobs[int(user["id"])] = status_payload


@router.post("/push-weekly")
def push_weekly_to_tracking(user: CurrentUser):
    """
    Start one manual weekly-push job in the background.

    Args:
        user: Authenticated user payload.

    Returns:
        Current background job status payload.
    """
    existing_status = _get_manual_push_status(int(user["id"]))
    if existing_status["status"] == "running":
        return existing_status

    started_at = time.time()
    job_id = uuid.uuid4().hex
    status_payload = _build_manual_push_status(
        job_id=job_id,
        status="running",
        message="Manual push started and is running in the background",
        started_at=started_at,
    )
    _set_manual_push_status(int(user["id"]), status_payload)

    worker = threading.Thread(
        target=_run_manual_push_job,
        args=(dict(user), job_id, started_at),
        daemon=True,
        name=f"manual-push-{user['id']}",
    )
    worker.start()
    return status_payload


@router.get("/push-weekly/status")
def get_push_weekly_status(user: CurrentUser):
    """
    Fetch the current manual weekly-push status for the user.

    Args:
        user: Authenticated user payload.

    Returns:
        Current background job status payload.
    """
    return _get_manual_push_status(int(user["id"]))


@router.get("/status")
async def tracking_status(user: CurrentUser):
    """Get tracking status for the user."""
    folder = get_tracking_folder(user["id"])
    folders = list_folders(user["id"])
    settings = get_notification_settings(user["id"])
    selected_databases = normalize_database_names(
        settings.get("selected_databases") if settings else []
    )
    weekly_articles = _filter_weekly_articles_by_database(
        _load_latest_weekly_articles(),
        selected_databases,
    )
    return {
        "tracking_folder": (
            {"id": folder["id"], "name": folder["name"]} if folder else None
        ),
        "total_folders": len(folders),
        "weekly_articles_available": len(weekly_articles),
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
    available_databases = [path.name for path in list_database_files()]
    selected_databases = normalize_database_names(body.selected_databases)
    invalid_databases = [
        db_name for db_name in selected_databases if db_name not in available_databases
    ]
    if invalid_databases:
        raise HTTPException(
            status_code=400,
            detail=("Unknown databases: " + ", ".join(invalid_databases)),
        )
    if selected_databases and set(selected_databases) == set(available_databases):
        selected_databases = []
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
        selected_databases=selected_databases,
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
