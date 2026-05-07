"""Database dependencies and query utilities."""

from __future__ import annotations

import re
import sqlite3
from collections.abc import AsyncGenerator
from typing import Annotated, Any

import aiosqlite
from fastapi import Depends, HTTPException, Query

from paper_scanner.index.db.schema import ensure_journal_platform_id_column
from paper_scanner.shared.db_path import resolve_db_path
from paper_scanner.shared.sqlite_ext import (
    article_search_uses_simple,
    fetch_article_search_sql,
    load_simple_tokenizer,
)


async def get_db(
    db: str | None = Query(
        default=None, description="Database name or filename under data/index"
    ),
) -> aiosqlite.Connection:
    """
    Provide a SQLite connection for a request.

    Args:
        db: Database name or filename under data/index.

    Returns:
        Open aiosqlite connection.
    """
    try:
        db_path = resolve_db_path(db)
    except ValueError as exc:
        message = str(exc)
        if message == "Database not found":
            raise HTTPException(status_code=404, detail=message) from exc
        if message == "No SQLite databases found":
            raise HTTPException(status_code=404, detail=message) from exc
        raise HTTPException(status_code=400, detail=message) from exc
    connection = await aiosqlite.connect(db_path)
    connection.row_factory = sqlite3.Row
    await load_simple_tokenizer(connection)
    await ensure_journal_platform_id_column(connection)
    await connection.commit()
    return connection


async def get_db_dependency(
    connection: Annotated[aiosqlite.Connection, Depends(get_db)],
) -> AsyncGenerator[aiosqlite.Connection]:
    """
    FastAPI dependency that ensures connection cleanup.

    Args:
        connection: Open database connection.

    Returns:
        Open database connection.
    """
    try:
        yield connection
    finally:
        await connection.close()


async def fetch_all(
    db: aiosqlite.Connection, query: str, params: list[Any]
) -> list[dict[str, Any]]:
    """
    Fetch all rows and return dictionaries.

    Args:
        db: Database connection.
        query: SQL query.
        params: Query parameters.

    Returns:
        List of row dictionaries.
    """
    cursor = await db.execute(query, params)
    rows = await cursor.fetchall()
    await cursor.close()
    return [dict(row) for row in rows]


async def fetch_one(
    db: aiosqlite.Connection, query: str, params: list[Any]
) -> dict[str, Any] | None:
    """
    Fetch a single row and return a dictionary.

    Args:
        db: Database connection.
        query: SQL query.
        params: Query parameters.

    Returns:
        Row dictionary or None.
    """
    cursor = await db.execute(query, params)
    row = await cursor.fetchone()
    await cursor.close()
    return dict(row) if row else None


async def is_simple_search_enabled(db: aiosqlite.Connection) -> bool:
    """
    Check whether article_search was built with the simple tokenizer.

    Args:
        db: Database connection.

    Returns:
        True when simple tokenizer is enabled for article_search.
    """
    sql = await fetch_article_search_sql(db)
    return article_search_uses_simple(sql)


def contains_cjk(value: str) -> bool:
    """
    Check whether a string contains CJK characters.

    Args:
        value: Input string.

    Returns:
        True when the string contains CJK characters.
    """
    return bool(re.search(r"[\u4e00-\u9fff]", value))


def should_use_simple_query(q: str | None, simple_enabled: bool) -> bool:
    """
    Decide whether to wrap the search query with simple_query().

    Args:
        q: Raw search query.
        simple_enabled: Whether the simple tokenizer is enabled.

    Returns:
        True when simple_query() should be used.
    """
    if not simple_enabled or not q:
        return False
    return not contains_cjk(q)
