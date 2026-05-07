"""SQLite retry helpers for indexer database access."""

from __future__ import annotations

import asyncio
import sqlite3
from typing import Any

import aiosqlite

from paper_scanner.shared.constants import DB_RETRY_ATTEMPTS, DB_RETRY_BASE_DELAY


async def execute_with_retry(
    db: aiosqlite.Connection, sql: str, params: tuple[Any, ...] | None = None
) -> None:
    """
    Execute a SQL statement with retries on database lock errors.

    Args:
        db: Open aiosqlite connection.
        sql: SQL statement to execute.
        params: SQL parameters.

    Returns:
        None.
    """
    for attempt in range(DB_RETRY_ATTEMPTS):
        try:
            if params is None:
                await db.execute(sql)
            else:
                await db.execute(sql, params)
            return
        except sqlite3.OperationalError as exc:
            if "database is locked" not in str(exc).lower():
                raise
            if attempt >= DB_RETRY_ATTEMPTS - 1:
                raise
            await asyncio.sleep(DB_RETRY_BASE_DELAY * (attempt + 1))


async def executemany_with_retry(
    db: aiosqlite.Connection, sql: str, rows: list[tuple[Any, ...]]
) -> None:
    """
    Execute many SQL statements with retries on database lock errors.

    Args:
        db: Open aiosqlite connection.
        sql: SQL statement to execute.
        rows: SQL parameter rows.

    Returns:
        None.
    """
    for attempt in range(DB_RETRY_ATTEMPTS):
        try:
            await db.executemany(sql, rows)
            return
        except sqlite3.OperationalError as exc:
            if "database is locked" not in str(exc).lower():
                raise
            if attempt >= DB_RETRY_ATTEMPTS - 1:
                raise
            await asyncio.sleep(DB_RETRY_BASE_DELAY * (attempt + 1))


async def commit_with_retry(db: aiosqlite.Connection) -> None:
    """
    Commit a transaction with retries on database lock errors.

    Args:
        db: Open aiosqlite connection.

    Returns:
        None.
    """
    for attempt in range(DB_RETRY_ATTEMPTS):
        try:
            await db.commit()
            return
        except sqlite3.OperationalError as exc:
            if "database is locked" not in str(exc).lower():
                raise
            if attempt >= DB_RETRY_ATTEMPTS - 1:
                raise
            await asyncio.sleep(DB_RETRY_BASE_DELAY * (attempt + 1))
