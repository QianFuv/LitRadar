"""Journal query handlers."""

from __future__ import annotations

from typing import Annotated, Any

import aiosqlite
from fastapi import Depends, HTTPException, Query

from paper_scanner.api.dependencies import fetch_all, fetch_one, get_db_dependency
from paper_scanner.api.models import JournalPage, JournalRecord
from paper_scanner.api.pagination import (
    JOURNAL_SORT_FIELDS,
    apply_sort,
    build_page_meta,
    parse_sort,
)
from paper_scanner.shared.constants import MAX_LIMIT


async def list_journals(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
    area: str | None = Query(default=None),
    library_id: str | None = Query(default=None),
    available: bool | None = Query(default=None),
    has_articles: bool | None = Query(default=None),
    year: int | None = Query(default=None, ge=0),
    scimago_min: float | None = Query(default=None),
    scimago_max: float | None = Query(default=None),
    sort: str | None = Query(default="scimago_rank:desc"),
    limit: int = Query(default=50, ge=1, le=MAX_LIMIT),
    offset: int = Query(default=0, ge=0),
) -> JournalPage:
    """
    List journals with filtering and sorting.

    Args:
        area: Filter by CSV area.
        library_id: Filter by library_id from journals.
        available: Filter by availability flag.
        has_articles: Filter by has_articles flag.
        year: Filter journals with issues in a year.
        scimago_min: Minimum Scimago rank value.
        scimago_max: Maximum Scimago rank value.
        sort: Multi-column sort string.
        limit: Page size.
        offset: Page offset.
        db: Database connection.

    Returns:
        Paginated journal list.
    """
    where_clauses: list[str] = []
    params: list[Any] = []

    if area:
        where_clauses.append("m.area = ?")
        params.append(area)
    if library_id:
        where_clauses.append("j.library_id = ?")
        params.append(library_id)
    if available is not None:
        where_clauses.append("j.available = ?")
        params.append(1 if available else 0)
    if has_articles is not None:
        where_clauses.append("j.has_articles = ?")
        params.append(1 if has_articles else 0)
    if scimago_min is not None:
        where_clauses.append("j.scimago_rank >= ?")
        params.append(scimago_min)
    if scimago_max is not None:
        where_clauses.append("j.scimago_rank <= ?")
        params.append(scimago_max)
    if year is not None:
        where_clauses.append(
            "EXISTS (SELECT 1 FROM issues i WHERE i.journal_id = j.journal_id AND "
            "i.publication_year = ?)"
        )
        params.append(year)
    where_sql = f"WHERE {' AND '.join(where_clauses)}" if where_clauses else ""
    order_sql = apply_sort(parse_sort(sort, JOURNAL_SORT_FIELDS))

    count_row = await fetch_one(
        db,
        f"""
        SELECT COUNT(*) AS total
        FROM journals j
        LEFT JOIN journal_meta m ON j.journal_id = m.journal_id
        {where_sql}
        """,
        params,
    )
    total = int(count_row["total"]) if count_row else 0

    rows = await fetch_all(
        db,
        f"""
        SELECT
            j.journal_id,
            j.library_id,
            j.platform_journal_id,
            j.title,
            j.issn,
            j.eissn,
            j.scimago_rank,
            j.cover_url,
            j.available,
            j.toc_data_approved_and_live,
            j.has_articles,
            m.source_csv,
            m.area,
            m.csv_title,
            m.csv_issn,
            m.csv_library
        FROM journals j
        LEFT JOIN journal_meta m ON j.journal_id = m.journal_id
        {where_sql}
        {order_sql}
        LIMIT ? OFFSET ?
        """,
        params + [limit, offset],
    )

    return JournalPage(
        items=[JournalRecord(**row) for row in rows],
        page=build_page_meta(total, limit, offset),
    )


async def get_journal(
    journal_id: int,
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> JournalRecord:
    """
    Fetch a single journal record.

    Args:
        journal_id: Journal identifier.
        db: Database connection.

    Returns:
        Journal record.
    """
    row = await fetch_one(
        db,
        """
        SELECT
            j.journal_id,
            j.library_id,
            j.platform_journal_id,
            j.title,
            j.issn,
            j.eissn,
            j.scimago_rank,
            j.cover_url,
            j.available,
            j.toc_data_approved_and_live,
            j.has_articles,
            m.source_csv,
            m.area,
            m.csv_title,
            m.csv_issn,
            m.csv_library
        FROM journals j
        LEFT JOIN journal_meta m ON j.journal_id = m.journal_id
        WHERE j.journal_id = ?
        """,
        [journal_id],
    )
    if not row:
        raise HTTPException(status_code=404, detail="Journal not found")
    return JournalRecord(**row)
