"""Issue query handlers."""

from __future__ import annotations

from typing import Annotated, Any

import aiosqlite
from fastapi import Depends, HTTPException, Query

from paper_scanner.api.dependencies import fetch_all, fetch_one, get_db_dependency
from paper_scanner.api.models import IssuePage, IssueRecord
from paper_scanner.api.pagination import (
    ISSUE_SORT_FIELDS,
    apply_sort,
    build_page_meta,
    parse_sort,
)
from paper_scanner.shared.constants import MAX_LIMIT


async def list_issues(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
    journal_id: int | None = Query(default=None, ge=0),
    year: int | None = Query(default=None, ge=0),
    is_valid_issue: bool | None = Query(default=None),
    suppressed: bool | None = Query(default=None),
    embargoed: bool | None = Query(default=None),
    within_subscription: bool | None = Query(default=None),
    sort: str | None = Query(default="publication_year:desc"),
    limit: int = Query(default=50, ge=1, le=MAX_LIMIT),
    offset: int = Query(default=0, ge=0),
) -> IssuePage:
    """
    List issues with filtering and sorting.

    Args:
        journal_id: Filter by journal ID.
        year: Filter by publication year.
        is_valid_issue: Filter by valid issue flag.
        suppressed: Filter by suppressed flag.
        embargoed: Filter by embargoed flag.
        within_subscription: Filter by within_subscription flag.
        sort: Multi-column sort string.
        limit: Page size.
        offset: Page offset.
        db: Database connection.

    Returns:
        Paginated issue list.
    """
    where_clauses: list[str] = []
    params: list[Any] = []

    if journal_id is not None:
        where_clauses.append("i.journal_id = ?")
        params.append(journal_id)
    if year is not None:
        where_clauses.append("i.publication_year = ?")
        params.append(year)
    if is_valid_issue is not None:
        where_clauses.append("i.is_valid_issue = ?")
        params.append(1 if is_valid_issue else 0)
    if suppressed is not None:
        where_clauses.append("i.suppressed = ?")
        params.append(1 if suppressed else 0)
    if embargoed is not None:
        where_clauses.append("i.embargoed = ?")
        params.append(1 if embargoed else 0)
    if within_subscription is not None:
        where_clauses.append("i.within_subscription = ?")
        params.append(1 if within_subscription else 0)

    where_sql = f"WHERE {' AND '.join(where_clauses)}" if where_clauses else ""
    order_sql = apply_sort(parse_sort(sort, ISSUE_SORT_FIELDS))

    count_row = await fetch_one(
        db,
        f"SELECT COUNT(*) AS total FROM issues i {where_sql}",
        params,
    )
    total = int(count_row["total"]) if count_row else 0

    rows = await fetch_all(
        db,
        f"""
        SELECT
            i.issue_id,
            i.journal_id,
            i.publication_year,
            i.title,
            i.volume,
            i.number,
            i.date,
            i.is_valid_issue,
            i.suppressed,
            i.embargoed,
            i.within_subscription
        FROM issues i
        {where_sql}
        {order_sql}
        LIMIT ? OFFSET ?
        """,
        params + [limit, offset],
    )

    return IssuePage(
        items=[IssueRecord(**row) for row in rows],
        page=build_page_meta(total, limit, offset),
    )


async def get_issue(
    issue_id: int,
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> IssueRecord:
    """
    Fetch a single issue record.

    Args:
        issue_id: Issue identifier.
        db: Database connection.

    Returns:
        Issue record.
    """
    row = await fetch_one(
        db,
        """
        SELECT
            issue_id,
            journal_id,
            publication_year,
            title,
            volume,
            number,
            date,
            is_valid_issue,
            suppressed,
            embargoed,
            within_subscription
        FROM issues
        WHERE issue_id = ?
        """,
        [issue_id],
    )
    if not row:
        raise HTTPException(status_code=404, detail="Issue not found")
    return IssueRecord(**row)
