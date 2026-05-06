"""Metadata query handlers."""

from __future__ import annotations

from typing import Annotated

import aiosqlite
from fastapi import Depends

from scripts.api.dependencies import fetch_all, get_db_dependency
from scripts.api.models import JournalOption, ValueCount, YearSummary
from scripts.shared.db_path import list_database_files


async def list_databases() -> list[str]:
    """
    List available SQLite databases.

    Returns:
        List of database filenames.
    """
    return [f.name for f in list_database_files()]


async def list_areas(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> list[ValueCount]:
    """
    List distinct journal areas.

    Args:
        db: Database connection.

    Returns:
        Area values with counts.
    """
    rows = await fetch_all(
        db,
        """
        SELECT area AS value, COUNT(*) AS count
        FROM journal_meta
        WHERE area IS NOT NULL AND area != ''
        GROUP BY area
        ORDER BY value ASC
        """,
        [],
    )
    return [ValueCount(**row) for row in rows]


async def list_journal_options(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> list[JournalOption]:
    """
    List journal identifiers and titles.

    Args:
        db: Database connection.

    Returns:
        Journal option list.
    """
    rows = await fetch_all(
        db,
        """
        SELECT journal_id, title
        FROM journals
        ORDER BY title ASC
        """,
        [],
    )
    return [JournalOption(**row) for row in rows]


async def list_sources(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> list[ValueCount]:
    """
    List distinct metadata source values.

    Args:
        db: Database connection.

    Returns:
        Source values with counts.
    """
    rows = await fetch_all(
        db,
        """
        SELECT csv_library AS value, COUNT(*) AS count
        FROM journal_meta
        WHERE csv_library IS NOT NULL AND csv_library != ''
        GROUP BY csv_library
        ORDER BY count DESC, value ASC
        """,
        [],
    )
    return [ValueCount(**row) for row in rows]


async def list_years(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> list[YearSummary]:
    """
    List publication years with issue and journal counts.

    Args:
        db: Database connection.

    Returns:
        Year summary rows.
    """
    rows = await fetch_all(
        db,
        """
        SELECT
            CAST(strftime('%Y', date) AS INTEGER) AS year,
            COUNT(DISTINCT issue_id) AS issue_count,
            COUNT(DISTINCT journal_id) AS journal_count
        FROM issues
        WHERE date IS NOT NULL
        GROUP BY year
        ORDER BY year DESC
        """,
        [],
    )
    return [YearSummary(**row) for row in rows]
