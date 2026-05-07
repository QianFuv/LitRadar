"""Change detection and notification trigger helpers."""

from __future__ import annotations

import json
import sqlite3
import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

from paper_scanner.shared.constants import NOTIFY_STATE_DIR
from paper_scanner.shared.converters import to_int


def normalize_issue_key(journal_id: int, issue_id: int) -> str:
    """
    Build a stable issue key string.

    Args:
        journal_id: Journal identifier.
        issue_id: Issue identifier.

    Returns:
        Normalized issue key.
    """
    return f"{journal_id}:{issue_id}"


def collect_article_snapshot(
    db_path: Path,
) -> tuple[dict[str, set[int]], dict[int, set[int]]]:
    """
    Collect article snapshot grouped by issue and in-press journal.

    Args:
        db_path: SQLite database path.

    Returns:
        Tuple of issue map and in-press map.
    """
    issue_map: dict[str, set[int]] = {}
    inpress_map: dict[int, set[int]] = {}
    with sqlite3.connect(db_path) as db:
        cursor = db.execute(
            """
            SELECT
                article_id,
                journal_id,
                issue_id,
                COALESCE(in_press, 0)
            FROM articles
            """
        )
        rows = cursor.fetchall()

    for article_id_raw, journal_id_raw, issue_id_raw, in_press_raw in rows:
        article_id = to_int(article_id_raw)
        journal_id = to_int(journal_id_raw)
        issue_id = to_int(issue_id_raw)
        in_press_flag = bool(to_int(in_press_raw) or 0)
        if article_id is None or journal_id is None:
            continue
        if issue_id is not None:
            issue_key = normalize_issue_key(journal_id, issue_id)
            issue_set = issue_map.setdefault(issue_key, set())
            issue_set.add(article_id)
            continue
        if not in_press_flag:
            continue
        inpress_set = inpress_map.setdefault(journal_id, set())
        inpress_set.add(article_id)

    return issue_map, inpress_map


def compute_changed_group_keys(
    before_issue_map: dict[str, set[int]],
    after_issue_map: dict[str, set[int]],
    before_inpress_map: dict[int, set[int]],
    after_inpress_map: dict[int, set[int]],
) -> tuple[list[str], list[int], dict[str, Any]]:
    """
    Compute changed issue and in-press groups from snapshots.

    Args:
        before_issue_map: Snapshot before update.
        after_issue_map: Snapshot after update.
        before_inpress_map: In-press snapshot before update.
        after_inpress_map: In-press snapshot after update.

    Returns:
        Changed issue keys, changed in-press journal ids, and summary.
    """
    issue_keys: set[str] = set(before_issue_map) | set(after_issue_map)
    changed_issue_keys = sorted(
        [
            key
            for key in issue_keys
            if before_issue_map.get(key, set()) != after_issue_map.get(key, set())
        ],
        key=lambda item: tuple(int(part) for part in item.split(":", maxsplit=1)),
    )

    inpress_keys: set[int] = set(before_inpress_map) | set(after_inpress_map)
    changed_inpress_ids = sorted(
        [
            journal_id
            for journal_id in inpress_keys
            if before_inpress_map.get(journal_id, set())
            != after_inpress_map.get(journal_id, set())
        ]
    )

    added_article_ids: set[int] = set()
    removed_article_ids: set[int] = set()
    changed_issue_details: list[dict[str, Any]] = []
    for issue_key in changed_issue_keys:
        before_set = before_issue_map.get(issue_key, set())
        after_set = after_issue_map.get(issue_key, set())
        added = sorted(after_set - before_set)
        removed = sorted(before_set - after_set)
        added_article_ids.update(added)
        removed_article_ids.update(removed)
        changed_issue_details.append(
            {
                "issue_key": issue_key,
                "before_count": len(before_set),
                "after_count": len(after_set),
                "added_article_ids": added,
                "removed_article_ids": removed,
            }
        )

    changed_inpress_details: list[dict[str, Any]] = []
    for journal_id in changed_inpress_ids:
        before_set = before_inpress_map.get(journal_id, set())
        after_set = after_inpress_map.get(journal_id, set())
        added = sorted(after_set - before_set)
        removed = sorted(before_set - after_set)
        added_article_ids.update(added)
        removed_article_ids.update(removed)
        changed_inpress_details.append(
            {
                "journal_id": journal_id,
                "before_count": len(before_set),
                "after_count": len(after_set),
                "added_article_ids": added,
                "removed_article_ids": removed,
            }
        )

    summary = {
        "changed_issue_count": len(changed_issue_keys),
        "changed_inpress_count": len(changed_inpress_ids),
        "added_article_count": len(added_article_ids),
        "removed_article_count": len(removed_article_ids),
        "added_article_ids": sorted(added_article_ids),
        "removed_article_ids": sorted(removed_article_ids),
        "issues": changed_issue_details,
        "inpress": changed_inpress_details,
    }
    return changed_issue_keys, changed_inpress_ids, summary


def parse_article_datetime(value: str | None) -> datetime | None:
    """
    Parse article date text into a timezone-aware UTC datetime.

    Args:
        value: Article date text.

    Returns:
        Parsed UTC datetime or None when parsing fails.
    """
    text = (value or "").strip()
    if not text:
        return None
    normalized = text.replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        try:
            parsed = datetime.fromisoformat(f"{text}T00:00:00+00:00")
        except ValueError:
            return None
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)


def split_notifiable_and_backfill_article_ids(
    db_path: Path,
    article_ids: list[int],
) -> tuple[set[int], set[int]]:
    """
    Split added article ids into notifiable and backfill groups.

    Args:
        db_path: Database path.
        article_ids: Added article identifiers.

    Returns:
        Tuple of notifiable article ids and backfill article ids.
    """
    if not article_ids:
        return set(), set()

    notifiable_ids: set[int] = set()
    backfill_ids: set[int] = set(article_ids)
    window_start = datetime.now(UTC) - timedelta(days=7)

    with sqlite3.connect(db_path) as db:
        for start in range(0, len(article_ids), 900):
            batch = article_ids[start : start + 900]
            placeholders = ", ".join(["?"] * len(batch))
            rows = db.execute(
                f"""
                SELECT article_id, date, COALESCE(in_press, 0)
                FROM articles
                WHERE article_id IN ({placeholders})
                """,
                batch,
            ).fetchall()
            for article_id_raw, date_raw, in_press_raw in rows:
                article_id = to_int(article_id_raw)
                if article_id is None:
                    continue
                in_press_flag = bool(to_int(in_press_raw) or 0)
                if in_press_flag:
                    notifiable_ids.add(article_id)
                    backfill_ids.discard(article_id)
                    continue
                article_date = parse_article_datetime(
                    str(date_raw) if date_raw is not None else None
                )
                if article_date and article_date >= window_start:
                    notifiable_ids.add(article_id)
                    backfill_ids.discard(article_id)

    return notifiable_ids, backfill_ids


def apply_manifest_filters(
    db_path: Path,
    changed_issue_keys: list[str],
    changed_inpress_ids: list[int],
    summary: dict[str, Any],
) -> tuple[list[str], list[int], list[str], list[int], dict[str, Any]]:
    """
    Filter manifest changes to notifiable additions and retain backfill metadata.

    Args:
        db_path: Database path.
        changed_issue_keys: Raw changed issue keys.
        changed_inpress_ids: Raw changed in-press journal ids.
        summary: Raw summary payload.

    Returns:
        Filtered issue keys, filtered in-press ids, backfill issue keys,
        backfill in-press ids, and updated summary payload.
    """
    raw_added_ids = summary.get("added_article_ids")
    added_article_ids: list[int] = []
    if isinstance(raw_added_ids, list):
        added_article_ids = [
            article_id for article_id in raw_added_ids if isinstance(article_id, int)
        ]
    notifiable_ids, backfill_ids = split_notifiable_and_backfill_article_ids(
        db_path,
        sorted(set(added_article_ids)),
    )

    issue_details = summary.get("issues")
    issue_notifiable_keys: set[str] = set()
    issue_backfill_keys: set[str] = set()
    if isinstance(issue_details, list):
        for item in issue_details:
            if not isinstance(item, dict):
                continue
            issue_key = str(item.get("issue_key") or "").strip()
            raw_issue_added = item.get("added_article_ids")
            issue_added: list[int] = []
            if isinstance(raw_issue_added, list):
                issue_added = [
                    article_id
                    for article_id in raw_issue_added
                    if isinstance(article_id, int)
                ]
            notifiable_added = sorted(
                article_id for article_id in issue_added if article_id in notifiable_ids
            )
            backfill_added = sorted(
                article_id for article_id in issue_added if article_id in backfill_ids
            )
            item["notifiable_added_article_ids"] = notifiable_added
            item["backfill_added_article_ids"] = backfill_added
            if issue_key and notifiable_added:
                issue_notifiable_keys.add(issue_key)
            if issue_key and backfill_added:
                issue_backfill_keys.add(issue_key)

    inpress_details = summary.get("inpress")
    inpress_notifiable_ids: set[int] = set()
    inpress_backfill_ids: set[int] = set()
    if isinstance(inpress_details, list):
        for item in inpress_details:
            if not isinstance(item, dict):
                continue
            journal_id = to_int(item.get("journal_id"))
            raw_inpress_added = item.get("added_article_ids")
            inpress_added: list[int] = []
            if isinstance(raw_inpress_added, list):
                inpress_added = [
                    article_id
                    for article_id in raw_inpress_added
                    if isinstance(article_id, int)
                ]
            notifiable_added = sorted(
                article_id
                for article_id in inpress_added
                if article_id in notifiable_ids
            )
            backfill_added = sorted(
                article_id for article_id in inpress_added if article_id in backfill_ids
            )
            item["notifiable_added_article_ids"] = notifiable_added
            item["backfill_added_article_ids"] = backfill_added
            if journal_id is not None and notifiable_added:
                inpress_notifiable_ids.add(journal_id)
            if journal_id is not None and backfill_added:
                inpress_backfill_ids.add(journal_id)

    filtered_issue_keys = [
        issue_key
        for issue_key in changed_issue_keys
        if issue_key in issue_notifiable_keys
    ]
    filtered_inpress_ids = [
        journal_id
        for journal_id in changed_inpress_ids
        if journal_id in inpress_notifiable_ids
    ]
    backfill_issue_keys = [
        issue_key
        for issue_key in changed_issue_keys
        if issue_key in issue_backfill_keys
    ]
    backfill_inpress_ids = [
        journal_id
        for journal_id in changed_inpress_ids
        if journal_id in inpress_backfill_ids
    ]

    summary["raw_changed_issue_count"] = len(changed_issue_keys)
    summary["raw_changed_inpress_count"] = len(changed_inpress_ids)
    summary["changed_issue_count"] = len(filtered_issue_keys)
    summary["changed_inpress_count"] = len(filtered_inpress_ids)
    summary["added_article_ids"] = sorted(notifiable_ids)
    summary["added_article_count"] = len(notifiable_ids)
    summary["backfill_article_ids"] = sorted(backfill_ids)
    summary["backfill_article_count"] = len(backfill_ids)
    summary["backfill_issue_keys"] = backfill_issue_keys
    summary["backfill_inpress_journal_ids"] = backfill_inpress_ids

    return (
        filtered_issue_keys,
        filtered_inpress_ids,
        backfill_issue_keys,
        backfill_inpress_ids,
        summary,
    )


def write_change_manifest(
    db_path: Path,
    changed_issue_keys: list[str],
    changed_inpress_ids: list[int],
    summary: dict[str, Any],
) -> Path:
    """
    Write change manifest used by notification task.

    Args:
        db_path: Database path.
        changed_issue_keys: Changed issue keys.
        changed_inpress_ids: Changed in-press journal ids.
        summary: Change summary details.

    Returns:
        Manifest file path.
    """
    (
        filtered_issue_keys,
        filtered_inpress_ids,
        backfill_issue_keys,
        backfill_inpress_ids,
        filtered_summary,
    ) = apply_manifest_filters(
        db_path,
        changed_issue_keys,
        changed_inpress_ids,
        summary,
    )
    changed_issue_keys[:] = filtered_issue_keys
    changed_inpress_ids[:] = filtered_inpress_ids

    now = datetime.utcnow().replace(microsecond=0).isoformat() + "Z"
    state_dir = db_path.parent.parent / "push_state"
    state_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = state_dir / f"{db_path.stem}.changes.json"
    payload = {
        "run_id": now,
        "generated_at": now,
        "db_name": db_path.name,
        "db_path": str(db_path),
        "changed_issue_keys": filtered_issue_keys,
        "changed_inpress_journal_ids": filtered_inpress_ids,
        "notifiable_article_ids": filtered_summary.get("added_article_ids", []),
        "backfill_issue_keys": backfill_issue_keys,
        "backfill_inpress_journal_ids": backfill_inpress_ids,
        "backfill_article_ids": filtered_summary.get("backfill_article_ids", []),
        "summary": filtered_summary,
    }
    tmp_path = manifest_path.with_name(f"{manifest_path.name}.tmp")
    with open(tmp_path, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
    tmp_path.replace(manifest_path)
    return manifest_path


def run_notify_for_manifest(
    db_path: Path,
    manifest_path: Path,
    dry_run: bool,
) -> int:
    """
    Invoke notify command with change manifest.

    Args:
        db_path: Database path.
        manifest_path: Change manifest path.
        dry_run: Whether to run notify in dry-run mode.

    Returns:
        Notify process return code.
    """
    command = [
        "uv",
        "run",
        "notify",
        "--db",
        db_path.name,
        "--changes-file",
        str(manifest_path),
        "--state-dir",
        NOTIFY_STATE_DIR,
    ]
    if dry_run:
        command.append("--dry-run")
    result = subprocess.run(command, check=False)
    return int(result.returncode)
