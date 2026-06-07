"""Database read/write operations used by index workflows."""

from __future__ import annotations

from datetime import datetime
from typing import Any

import aiosqlite

from paper_scanner.index.db.client import DatabaseClient
from paper_scanner.index.db.retry import execute_with_retry
from paper_scanner.index.db.schema import (
    ARTICLE_COLUMNS,
    ARTICLE_LISTING_BATCH_SIZE,
    ARTICLE_LISTING_COLUMNS,
    ARTICLE_UPSERT,
    ISSUE_COLUMNS,
    ISSUE_UPSERT,
    JOURNAL_COLUMNS,
    JOURNAL_UPSERT,
    META_COLUMNS,
    META_UPSERT,
)
from paper_scanner.shared.converters import chunked


async def upsert_journal(db: DatabaseClient, record: dict[str, Any]) -> None:
    """
    Insert or update a journal record.

    Args:
        db: Database client.
        record: Journal record data.

    Returns:
        None.
    """
    await db.execute(JOURNAL_UPSERT, tuple(record[col] for col in JOURNAL_COLUMNS))


async def upsert_meta(db: DatabaseClient, record: dict[str, Any]) -> None:
    """
    Insert or update a journal meta record.

    Args:
        db: Database client.
        record: Journal meta record data.

    Returns:
        None.
    """
    await db.execute(META_UPSERT, tuple(record[col] for col in META_COLUMNS))


async def upsert_issues(db: DatabaseClient, records: list[dict[str, Any]]) -> None:
    """
    Insert or update issue records.

    Args:
        db: Database client.
        records: List of issue record data.

    Returns:
        None.
    """
    if not records:
        return
    rows = [tuple(record[col] for col in ISSUE_COLUMNS) for record in records]
    await db.executemany(ISSUE_UPSERT, rows)


async def upsert_articles(db: DatabaseClient, records: list[dict[str, Any]]) -> None:
    """
    Insert or update article records.

    Args:
        db: Database client.
        records: List of article record data.

    Returns:
        None.
    """
    if not records:
        return
    rows = [tuple(record[col] for col in ARTICLE_COLUMNS) for record in records]
    await db.executemany(ARTICLE_UPSERT, rows)


async def delete_articles(db: DatabaseClient, article_ids: list[int]) -> None:
    """
    Delete article rows and derived index rows.

    Args:
        db: Database client.
        article_ids: Article IDs to delete.

    Returns:
        None.
    """
    if not article_ids:
        return
    unique_ids = list(dict.fromkeys(article_ids))
    for batch in chunked(unique_ids, ARTICLE_LISTING_BATCH_SIZE):
        placeholders = ", ".join(["?"] * len(batch))
        params = tuple(batch)
        await db.execute(
            f"DELETE FROM article_search WHERE rowid IN ({placeholders})", params
        )
        await db.execute(
            f"DELETE FROM article_listing WHERE article_id IN ({placeholders})",
            params,
        )
        await db.execute(
            f"DELETE FROM articles WHERE article_id IN ({placeholders})", params
        )


async def upsert_article_search(
    db: DatabaseClient,
    records: list[dict[str, Any]],
    journal_title: str | None,
) -> None:
    """
    Update FTS index rows for articles.

    Args:
        db: Database client.
        records: Article record data.
        journal_title: Journal title for the article.

    Returns:
        None.
    """
    if not records:
        return
    title_value = journal_title or ""
    insert_rows = [
        (
            record["article_id"],
            record["article_id"],
            record.get("title") or "",
            record.get("abstract") or "",
            record.get("doi") or "",
            record.get("authors") or "",
            title_value,
        )
        for record in records
    ]
    await db.executemany(
        """
        INSERT OR REPLACE INTO article_search (
            rowid,
            article_id,
            title,
            abstract,
            doi,
            authors,
            journal_title
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        """,
        insert_rows,
    )


def build_article_listing_upsert(where_sql: str) -> str:
    """
    Build the upsert SQL for article listing rows.

    Args:
        where_sql: WHERE clause string starting with WHERE.

    Returns:
        SQL statement for inserting listing rows.
    """
    return f"""
    INSERT INTO article_listing ({", ".join(ARTICLE_LISTING_COLUMNS)})
    SELECT
        a.article_id,
        a.journal_id,
        a.issue_id,
        i.publication_year,
        a.date,
        a.open_access,
        a.in_press,
        a.suppressed,
        a.within_library_holdings,
        a.doi,
        a.pmid,
        m.area
    FROM articles a
    LEFT JOIN issues i ON i.issue_id = a.issue_id
    LEFT JOIN journal_meta m ON m.journal_id = a.journal_id
    {where_sql}
    ON CONFLICT(article_id) DO UPDATE SET
    {", ".join(f"{col}=excluded.{col}" for col in ARTICLE_LISTING_COLUMNS[1:])}
    """


async def refresh_article_listing_for_articles(
    db: DatabaseClient, article_ids: list[int]
) -> None:
    """
    Refresh listing rows for the provided article ids.

    Args:
        db: Database client.
        article_ids: Article id list to refresh.

    Returns:
        None.
    """
    if not article_ids:
        return
    for batch in chunked(article_ids, ARTICLE_LISTING_BATCH_SIZE):
        placeholders = ", ".join(["?"] * len(batch))
        sql = build_article_listing_upsert(f"WHERE a.article_id IN ({placeholders})")
        await db.execute(sql, tuple(batch))


async def refresh_article_listing_for_issues(
    db: DatabaseClient, issue_ids: list[int]
) -> None:
    """
    Refresh listing rows for the provided issue ids.

    Args:
        db: Database client.
        issue_ids: Issue id list to refresh.

    Returns:
        None.
    """
    if not issue_ids:
        return
    for batch in chunked(issue_ids, ARTICLE_LISTING_BATCH_SIZE):
        placeholders = ", ".join(["?"] * len(batch))
        sql = build_article_listing_upsert(f"WHERE a.issue_id IN ({placeholders})")
        await db.execute(sql, tuple(batch))


async def get_journal_issue_ids_with_articles(
    db: DatabaseClient, journal_id: int
) -> set[int]:
    """
    Fetch issue IDs that already have articles for a journal.

    Args:
        db: Database client.
        journal_id: Internal journal ID.

    Returns:
        Set of issue IDs with existing articles.
    """
    rows = await db.fetchall(
        """
        SELECT DISTINCT a.issue_id
        FROM articles a
        JOIN issues i ON i.issue_id = a.issue_id
        WHERE i.journal_id = ?
        """,
        (journal_id,),
    )
    return {row[0] for row in rows if row[0] is not None}


async def get_latest_issue_with_articles(
    db: DatabaseClient, journal_id: int
) -> tuple[int, int | None, str | None] | None:
    """
    Fetch the newest issue that already has articles for a journal.

    Args:
        db: Database client.
        journal_id: Internal journal ID.

    Returns:
        Issue ID, publication year, and issue date, or None when absent.
    """
    row = await db.fetchone(
        """
        SELECT i.issue_id, i.publication_year, i.date
        FROM issues i
        WHERE i.journal_id = ?
            AND EXISTS (
                SELECT 1
                FROM articles a
                WHERE a.issue_id = i.issue_id
            )
        ORDER BY
            COALESCE(i.publication_year, 0) DESC,
            COALESCE(i.date, '') DESC,
            i.issue_id DESC
        LIMIT 1
        """,
        (journal_id,),
    )
    if row is None:
        return None
    return row[0], row[1], row[2]


async def get_issue_ids_with_articles(
    db: DatabaseClient, journal_id: int, year: int
) -> set[int]:
    """
    Fetch issue IDs that already have articles for a journal year.

    Args:
        db: Database client.
        journal_id: Internal journal ID.
        year: Publication year.

    Returns:
        Set of issue IDs with existing articles.
    """
    rows = await db.fetchall(
        """
        SELECT DISTINCT a.issue_id
        FROM articles a
        JOIN issues i ON i.issue_id = a.issue_id
        WHERE i.journal_id = ? AND i.publication_year = ?
        """,
        (journal_id, year),
    )
    return {row[0] for row in rows if row[0] is not None}


async def get_completed_years(db: DatabaseClient, journal_id: int) -> set[int]:
    """
    Fetch completed years for a journal.

    Args:
        db: Database client.
        journal_id: Internal journal ID.

    Returns:
        Set of completed years.
    """
    rows = await db.fetchall(
        "SELECT year FROM journal_year_state WHERE journal_id = ? AND status = 'done'",
        (journal_id,),
    )
    return {row[0] for row in rows}


async def is_journal_complete(db: DatabaseClient, journal_id: int) -> bool:
    """
    Check whether a journal is marked as completed.

    Args:
        db: Database client.
        journal_id: Internal journal ID.

    Returns:
        True when the journal is completed.
    """
    row = await db.fetchone(
        "SELECT status FROM journal_state WHERE journal_id = ?",
        (journal_id,),
    )
    return row is not None and row[0] == "done"


async def mark_year_done(db: DatabaseClient, journal_id: int, year: int) -> None:
    """
    Mark a year as completed for a journal.

    Args:
        db: Database client.
        journal_id: Internal journal ID.
        year: Publication year.

    Returns:
        None.
    """
    timestamp = datetime.utcnow().isoformat()
    await db.execute(
        """
        INSERT INTO journal_year_state (journal_id, year, status, updated_at)
        VALUES (?, ?, 'done', ?)
        ON CONFLICT(journal_id, year) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        """,
        (journal_id, year, timestamp),
    )


async def mark_journal_done(db: DatabaseClient, journal_id: int) -> None:
    """
    Mark a journal as completed.

    Args:
        db: Database client.
        journal_id: Internal journal ID.

    Returns:
        None.
    """
    timestamp = datetime.utcnow().isoformat()
    await db.execute(
        """
        INSERT INTO journal_state (journal_id, status, updated_at)
        VALUES (?, 'done', ?)
        ON CONFLICT(journal_id) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        """,
        (journal_id, timestamp),
    )


async def mark_listing_ready(db: aiosqlite.Connection) -> None:
    """
    Mark the article listing as ready for query use.

    Args:
        db: Database client.

    Returns:
        None.
    """
    timestamp = datetime.utcnow().isoformat()
    await execute_with_retry(
        db,
        """
        INSERT INTO listing_state (id, status, updated_at)
        VALUES (1, 'ready', ?)
        ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        """,
        (timestamp,),
    )
