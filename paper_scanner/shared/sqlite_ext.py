"""Shared SQLite extension and FTS helpers."""

from __future__ import annotations

import os
import sqlite3
import sys

import aiosqlite

from paper_scanner.shared.constants import PROJECT_ROOT, SIMPLE_TOKENIZER_ENV


def resolve_simple_tokenizer_path() -> str | None:
    """
    Resolve path of the optional simple tokenizer extension.

    Returns:
        Existing extension path, or None when unavailable.
    """
    value = os.getenv(SIMPLE_TOKENIZER_ENV)
    if value:
        path = value.strip()
        return path or None
    libs_dir = PROJECT_ROOT / "libs"
    if sys.platform.startswith("win"):
        candidate = libs_dir / "simple-windows" / "libsimple-windows-x64" / "simple.dll"
    elif sys.platform.startswith("linux"):
        candidate = (
            libs_dir / "simple-linux" / "libsimple-linux-ubuntu-latest" / "libsimple.so"
        )
    else:
        return None
    return str(candidate) if candidate.exists() else None


async def load_simple_tokenizer(db: aiosqlite.Connection) -> bool:
    """
    Load simple tokenizer extension for a database connection.

    Args:
        db: Open aiosqlite connection.

    Returns:
        True when extension is loaded successfully.
    """
    path = resolve_simple_tokenizer_path()
    if not path:
        return False
    try:
        await db.enable_load_extension(True)
        await db.load_extension(path)
        await db.enable_load_extension(False)
    except (sqlite3.OperationalError, OSError):
        return False
    return True


def article_search_uses_simple(sql: str | None) -> bool:
    """
    Determine whether article_search table uses simple tokenizer.

    Args:
        sql: SQLite DDL text for article_search table.

    Returns:
        True when tokenizer clause contains simple.
    """
    if not sql:
        return False
    normalized = sql.lower()
    return "tokenize" in normalized and "simple" in normalized


async def fetch_article_search_sql(db: aiosqlite.Connection) -> str | None:
    """
    Read article_search schema SQL from sqlite_master.

    Args:
        db: Open aiosqlite connection.

    Returns:
        Table SQL or None when table is missing.
    """
    cursor = await db.execute(
        "SELECT sql FROM sqlite_master WHERE name = 'article_search'"
    )
    row = await cursor.fetchone()
    await cursor.close()
    if row and row[0]:
        return str(row[0])
    return None
