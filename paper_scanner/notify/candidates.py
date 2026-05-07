"""Candidate loading and normalization."""

from __future__ import annotations

import sqlite3
from typing import Any

from paper_scanner.notify.models import ArticleCandidate
from paper_scanner.notify.state import parse_issue_key
from paper_scanner.shared.converters import to_int


def fetch_candidates_for_issue_keys(
    connection: sqlite3.Connection,
    issue_keys: list[str],
) -> list[ArticleCandidate]:
    """
    Fetch candidate articles for pending issue keys.

    Args:
        connection: SQLite connection.
        issue_keys: Pending issue keys.

    Returns:
        Candidate list.
    """
    if not issue_keys:
        return []

    issue_ids = sorted({parse_issue_key(key)[1] for key in issue_keys})
    placeholders = ", ".join(["?"] * len(issue_ids))

    rows = connection.execute(
        f"""
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            a.title,
            a.abstract,
            a.date,
            a.open_access,
            a.in_press,
            a.within_library_holdings,
            a.doi,
            a.full_text_file,
            a.permalink,
            j.title AS journal_title
        FROM articles a
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.issue_id IN ({placeholders})
          AND COALESCE(a.suppressed, 0) = 0
        ORDER BY a.date DESC, a.article_id DESC
        """,
        issue_ids,
    ).fetchall()

    return [row_to_candidate(row) for row in rows]


def fetch_candidates_for_inpress_keys(
    connection: sqlite3.Connection,
    inpress_keys: list[str],
) -> list[ArticleCandidate]:
    """
    Fetch candidate in-press articles for pending journals.

    Args:
        connection: SQLite connection.
        inpress_keys: Pending journal keys.

    Returns:
        Candidate list.
    """
    if not inpress_keys:
        return []

    journal_ids = sorted({int(key) for key in inpress_keys})
    placeholders = ", ".join(["?"] * len(journal_ids))

    rows = connection.execute(
        f"""
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            a.title,
            a.abstract,
            a.date,
            a.open_access,
            a.in_press,
            a.within_library_holdings,
            a.doi,
            a.full_text_file,
            a.permalink,
            j.title AS journal_title
        FROM articles a
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.issue_id IS NULL
          AND COALESCE(a.in_press, 0) = 1
          AND a.journal_id IN ({placeholders})
          AND COALESCE(a.suppressed, 0) = 0
        ORDER BY a.date DESC, a.article_id DESC
        """,
        journal_ids,
    ).fetchall()

    return [row_to_candidate(row) for row in rows]


def row_to_candidate(row: sqlite3.Row | tuple[Any, ...]) -> ArticleCandidate:
    """
    Convert SQL row to candidate object.

    Args:
        row: SQLite row.

    Returns:
        Candidate instance.
    """
    row_data = dict(row)
    title = str(row_data.get("title") or "Untitled article").strip()
    abstract = str(row_data.get("abstract") or "").strip()
    journal_title = str(row_data.get("journal_title") or "Unknown journal").strip()

    return ArticleCandidate(
        article_id=int(row_data["article_id"]),
        journal_id=int(row_data["journal_id"]),
        issue_id=to_int(row_data.get("issue_id")),
        title=title,
        abstract=abstract,
        date=str(row_data.get("date") or "").strip() or None,
        journal_title=journal_title,
        doi=str(row_data.get("doi") or "").strip() or None,
        full_text_file=str(row_data.get("full_text_file") or "").strip() or None,
        permalink=str(row_data.get("permalink") or "").strip() or None,
        open_access=bool(to_int(row_data.get("open_access")) or 0),
        in_press=bool(to_int(row_data.get("in_press")) or 0),
        within_library_holdings=bool(
            to_int(row_data.get("within_library_holdings")) or 0
        ),
    )


def deduplicate_candidates(
    candidates: list[ArticleCandidate],
) -> list[ArticleCandidate]:
    """
    Deduplicate candidates by article id while preserving order.

    Args:
        candidates: Candidate list.

    Returns:
        Deduplicated list.
    """
    deduped: list[ArticleCandidate] = []
    seen_ids: set[int] = set()
    for item in candidates:
        if item.article_id in seen_ids:
            continue
        seen_ids.add(item.article_id)
        deduped.append(item)
    return deduped
