"""Change detection helpers."""

from __future__ import annotations

import sqlite3

from paper_scanner.notify.state import build_issue_key, parse_issue_key


def collect_issue_article_counts(connection: sqlite3.Connection) -> dict[str, int]:
    """
    Collect article counts grouped by journal and issue.

    Args:
        connection: SQLite connection.

    Returns:
        Snapshot map keyed by issue key.
    """
    rows = connection.execute(
        """
        SELECT journal_id, issue_id, COUNT(*)
        FROM articles
        WHERE issue_id IS NOT NULL
        GROUP BY journal_id, issue_id
        """
    ).fetchall()
    snapshot: dict[str, int] = {}
    for journal_id, issue_id, count in rows:
        snapshot[build_issue_key(int(journal_id), int(issue_id))] = int(count)
    return snapshot


def collect_inpress_article_counts(connection: sqlite3.Connection) -> dict[str, int]:
    """
    Collect in-press article counts grouped by journal.

    Args:
        connection: SQLite connection.

    Returns:
        Snapshot map keyed by journal id.
    """
    rows = connection.execute(
        """
        SELECT journal_id, COUNT(*)
        FROM articles
        WHERE issue_id IS NULL AND COALESCE(in_press, 0) = 1
        GROUP BY journal_id
        """
    ).fetchall()
    return {str(int(journal_id)): int(count) for journal_id, count in rows}


def compute_changed_issue_keys(
    previous_counts: dict[str, int],
    current_counts: dict[str, int],
) -> list[str]:
    """
    Compute issue keys whose counts changed.

    Args:
        previous_counts: Previous snapshot map.
        current_counts: Current snapshot map.

    Returns:
        Sorted changed issue keys.
    """
    changed = [
        key
        for key, current_count in current_counts.items()
        if previous_counts.get(key) != current_count
    ]
    return sorted(changed, key=lambda item: parse_issue_key(item))


def compute_changed_inpress_keys(
    previous_counts: dict[str, int],
    current_counts: dict[str, int],
) -> list[str]:
    """
    Compute in-press keys whose counts changed.

    Args:
        previous_counts: Previous snapshot map.
        current_counts: Current snapshot map.

    Returns:
        Sorted changed in-press keys.
    """
    changed = [
        key
        for key, current_count in current_counts.items()
        if previous_counts.get(key) != current_count
    ]
    return sorted(changed, key=lambda item: int(item))
