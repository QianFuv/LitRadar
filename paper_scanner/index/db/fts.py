"""FTS table helpers for article search."""

from __future__ import annotations

import aiosqlite

from paper_scanner.index.db.retry import execute_with_retry
from paper_scanner.shared.sqlite_ext import (
    article_search_uses_simple,
    fetch_article_search_sql,
)


def build_article_search_sql(use_simple: bool) -> str:
    """
    Build the CREATE VIRTUAL TABLE SQL for article_search.

    Args:
        use_simple: Whether to enable the simple tokenizer.

    Returns:
        SQL statement for creating the FTS table.
    """
    tokenizer_clause = ", tokenize = 'simple'" if use_simple else ""
    return f"""
        CREATE VIRTUAL TABLE IF NOT EXISTS article_search
        USING fts5(
            article_id UNINDEXED,
            title,
            abstract,
            doi,
            authors,
            journal_title
            {tokenizer_clause}
        );
        """


async def rebuild_article_search(db: aiosqlite.Connection) -> None:
    """
    Rebuild the article_search FTS rows from stored articles.

    Args:
        db: Open aiosqlite connection.

    Returns:
        None.
    """
    await execute_with_retry(
        db,
        """
        INSERT OR REPLACE INTO article_search (
            rowid,
            article_id,
            title,
            abstract,
            doi,
            authors,
            journal_title
        )
        SELECT
            a.article_id,
            a.article_id,
            COALESCE(a.title, ''),
            COALESCE(a.abstract, ''),
            COALESCE(a.doi, ''),
            COALESCE(a.authors, ''),
            COALESCE(j.title, '')
        FROM articles a
        LEFT JOIN journals j ON j.journal_id = a.journal_id
        """,
    )


async def ensure_article_search(db: aiosqlite.Connection, use_simple: bool) -> None:
    """
    Ensure the article_search FTS table exists and matches tokenizer settings.

    Args:
        db: Open aiosqlite connection.
        use_simple: Whether the simple tokenizer is enabled.

    Returns:
        None.
    """
    existing_sql = await fetch_article_search_sql(db)
    if existing_sql and article_search_uses_simple(existing_sql) and not use_simple:
        raise RuntimeError(
            "Simple tokenizer required for article_search. "
            "Set SIMPLE_TOKENIZER_PATH to the simple extension."
        )
    if existing_sql and not use_simple:
        return
    if not existing_sql:
        await execute_with_retry(db, build_article_search_sql(use_simple))
        return
    if use_simple and not article_search_uses_simple(existing_sql):
        await execute_with_retry(db, "DROP TABLE IF EXISTS article_search")
        await execute_with_retry(db, build_article_search_sql(use_simple))
        await rebuild_article_search(db)
