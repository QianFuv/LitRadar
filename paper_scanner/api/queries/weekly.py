"""Weekly updates query handlers."""

from __future__ import annotations

import json
import sqlite3
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import aiosqlite

from paper_scanner.api.dependencies import fetch_all
from paper_scanner.api.models import (
    WeeklyArticleRecord,
    WeeklyDatabaseUpdate,
    WeeklyJournalUpdate,
    WeeklyManifestSummary,
    WeeklyUpdatesResponse,
)
from paper_scanner.shared.constants import INDEX_DIR, PUSH_STATE_DIR


def parse_iso_datetime(value: str) -> datetime | None:
    """
    Parse an ISO datetime string into a timezone-aware UTC datetime.

    Args:
        value: ISO datetime string.

    Returns:
        Parsed datetime in UTC or None when invalid.
    """
    text = value.strip()
    if not text:
        return None
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)


def parse_manifest_generated_at(payload: dict[str, Any]) -> datetime:
    """
    Parse generated timestamp from a changes manifest payload.

    Args:
        payload: Manifest JSON payload.

    Returns:
        Parsed UTC datetime.
    """
    for key in ("generated_at", "run_id"):
        raw_value = payload.get(key)
        if isinstance(raw_value, str):
            parsed = parse_iso_datetime(raw_value)
            if parsed:
                return parsed
    return datetime.now(UTC)


def extract_added_article_ids(payload: dict[str, Any]) -> list[int]:
    """
    Extract all added article IDs (notifiable + backfill) from a changes manifest.

    Args:
        payload: Manifest JSON payload.

    Returns:
        Unique article IDs preserving first appearance order.
    """
    unique_ids: list[int] = []
    seen: set[int] = set()
    for key in ("notifiable_article_ids", "backfill_article_ids"):
        raw_ids = payload.get(key)
        if not isinstance(raw_ids, list):
            continue
        for item in raw_ids:
            if not isinstance(item, int):
                continue
            if item in seen:
                continue
            seen.add(item)
            unique_ids.append(item)
    return unique_ids


def parse_weekly_manifest(
    payload: dict[str, Any],
) -> WeeklyManifestSummary | None:
    """
    Parse one raw manifest into a validated weekly summary object.

    Args:
        payload: Manifest JSON payload.

    Returns:
        Weekly summary object or None when invalid.
    """
    generated_at = parse_manifest_generated_at(payload)

    db_name = parse_db_name_from_manifest(payload)
    if not db_name:
        return None

    article_ids = extract_added_article_ids(payload)
    if not article_ids:
        return None

    run_id_value = payload.get("run_id")
    run_id = run_id_value if isinstance(run_id_value, str) else None
    return WeeklyManifestSummary(
        db_name=db_name,
        run_id=run_id,
        generated_at=generated_at,
        article_ids=article_ids,
    )


def load_weekly_manifest_payloads() -> list[WeeklyManifestSummary]:
    """
    Load all changes manifest payloads from push_state.

    Returns:
        Sorted weekly manifest summaries.
    """
    if not PUSH_STATE_DIR.exists():
        return []

    manifest_entries: list[WeeklyManifestSummary] = []
    for path in sorted(PUSH_STATE_DIR.glob("*.changes.json")):
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        parsed = parse_weekly_manifest(payload)
        if parsed is None:
            continue
        manifest_entries.append(parsed)

    manifest_entries.sort(
        key=lambda item: (
            item.generated_at,
            item.db_name,
        ),
        reverse=True,
    )
    return manifest_entries


def parse_db_name_from_manifest(payload: dict[str, Any]) -> str | None:
    """
    Resolve database filename from a changes manifest payload.

    Args:
        payload: Manifest JSON payload.

    Returns:
        Database filename with .sqlite suffix or None.
    """
    raw_name = payload.get("db_name")
    if isinstance(raw_name, str):
        candidate = Path(raw_name).name.strip()
        if candidate:
            if candidate.endswith(".sqlite"):
                return candidate
            return f"{candidate}.sqlite"

    raw_path = payload.get("db_path")
    if isinstance(raw_path, str):
        candidate = Path(raw_path).name.strip()
        if candidate:
            if candidate.endswith(".sqlite"):
                return candidate
            return f"{candidate}.sqlite"
    return None


def group_articles_by_journal(
    articles: list[WeeklyArticleRecord],
) -> list[WeeklyJournalUpdate]:
    """
    Group weekly article rows by journal.

    Args:
        articles: Weekly article rows.

    Returns:
        Sorted journal update summaries.
    """
    journal_map: dict[int, list[WeeklyArticleRecord]] = {}
    for article in articles:
        journal_map.setdefault(article.journal_id, []).append(article)

    journals: list[WeeklyJournalUpdate] = []
    for journal_id, journal_articles in journal_map.items():
        journal_title = None
        if journal_articles:
            journal_title = journal_articles[0].journal_title
        journals.append(
            WeeklyJournalUpdate(
                journal_id=journal_id,
                journal_title=journal_title,
                new_article_count=len(journal_articles),
                articles=journal_articles,
            )
        )

    journals.sort(
        key=lambda item: (
            -item.new_article_count,
            (item.journal_title or "").lower(),
            item.journal_id,
        )
    )
    return journals


async def fetch_articles_by_ids(
    db: aiosqlite.Connection,
    article_ids: list[int],
) -> list[WeeklyArticleRecord]:
    """
    Fetch article records by article IDs.

    Args:
        db: Database connection.
        article_ids: Article IDs.

    Returns:
        Weekly article records.
    """
    if not article_ids:
        return []
    placeholders = ", ".join(["?"] * len(article_ids))
    rows = await fetch_all(
        db,
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
            j.title AS journal_title,
            i.volume,
            i.number
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
        """,
        article_ids,
    )
    row_map = {int(row["article_id"]): row for row in rows}
    ordered_rows = [
        row_map[article_id] for article_id in article_ids if article_id in row_map
    ]
    return [WeeklyArticleRecord(**row) for row in ordered_rows]


async def get_weekly_updates() -> WeeklyUpdatesResponse:
    """
    List weekly new-article updates grouped by database and journal.

    Returns:
        Weekly updates response grouped by database and journal.
    """
    now = datetime.now(UTC)

    manifests = load_weekly_manifest_payloads()
    if not manifests:
        window_start = now - timedelta(days=7)
        return WeeklyUpdatesResponse(
            generated_at=now.isoformat().replace("+00:00", "Z"),
            window_start=window_start.isoformat().replace("+00:00", "Z"),
            window_end=now.isoformat().replace("+00:00", "Z"),
            databases=[],
        )

    aggregated_by_db: dict[str, dict[str, Any]] = {}
    for manifest in manifests:
        db_bucket = aggregated_by_db.get(manifest.db_name)
        if db_bucket is None:
            db_bucket = {
                "generated_at": manifest.generated_at,
                "run_id": manifest.run_id,
                "article_ids": [],
                "seen_ids": set(),
            }
            aggregated_by_db[manifest.db_name] = db_bucket

        seen_ids = db_bucket["seen_ids"]
        if isinstance(seen_ids, set):
            for article_id in manifest.article_ids:
                if article_id in seen_ids:
                    continue
                seen_ids.add(article_id)
                article_ids = db_bucket["article_ids"]
                if isinstance(article_ids, list):
                    article_ids.append(article_id)

    db_updates: list[WeeklyDatabaseUpdate] = []
    for db_name, bucket in aggregated_by_db.items():
        db_path = INDEX_DIR / db_name
        if not db_path.exists():
            continue

        article_ids = bucket.get("article_ids")
        if not isinstance(article_ids, list) or not article_ids:
            continue

        connection = await aiosqlite.connect(db_path)
        try:
            connection.row_factory = sqlite3.Row
            article_rows = await fetch_articles_by_ids(connection, article_ids)
        finally:
            await connection.close()

        if not article_rows:
            continue
        journals = group_articles_by_journal(article_rows)

        generated_at = bucket.get("generated_at")
        generated_text = now
        if isinstance(generated_at, datetime):
            generated_text = generated_at

        run_id = bucket.get("run_id")
        run_id_value = run_id if isinstance(run_id, str) else None

        db_updates.append(
            WeeklyDatabaseUpdate(
                db_name=db_name,
                run_id=run_id_value,
                generated_at=generated_text.isoformat().replace("+00:00", "Z"),
                new_article_count=len(article_rows),
                journals=journals,
            )
        )

    db_updates.sort(
        key=lambda item: (
            item.generated_at,
            item.db_name,
        ),
        reverse=True,
    )

    manifest_times = [m.generated_at for m in manifests]
    window_end = max(manifest_times)
    window_start = window_end - timedelta(days=7)

    return WeeklyUpdatesResponse(
        generated_at=now.isoformat().replace("+00:00", "Z"),
        window_start=window_start.isoformat().replace("+00:00", "Z"),
        window_end=window_end.isoformat().replace("+00:00", "Z"),
        databases=db_updates,
    )
