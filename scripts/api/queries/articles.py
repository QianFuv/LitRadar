"""Article query handlers."""

from __future__ import annotations

from typing import Annotated, Any

import aiosqlite
from fastapi import Depends, HTTPException, Query
from starlette.responses import RedirectResponse

from scripts.api.dependencies import (
    fetch_all,
    fetch_one,
    get_db_dependency,
    is_simple_search_enabled,
    should_use_simple_query,
)
from scripts.api.models import ArticlePage, ArticleRecord
from scripts.api.pagination import (
    ARTICLE_SORT_FIELDS,
    build_article_cursor,
    build_page_meta,
    parse_article_cursor,
    parse_sort,
)
from scripts.shared.constants import MAX_LIMIT


async def is_article_listing_ready(db: aiosqlite.Connection) -> bool:
    """
    Check whether the article listing table is available and populated.

    Args:
        db: Database connection.

    Returns:
        True when the listing table can be used safely.
    """
    try:
        state_row = await fetch_one(
            db, "SELECT status FROM listing_state WHERE id = 1", []
        )
    except aiosqlite.Error:
        return False
    if not state_row or state_row.get("status") != "ready":
        return False
    try:
        await fetch_one(db, "SELECT 1 FROM article_listing LIMIT 1", [])
    except aiosqlite.Error:
        return False
    return True


async def list_articles_from_listing(
    db: aiosqlite.Connection,
    journal_id: list[int] | None,
    issue_id: int | None,
    year: int | None,
    area: list[str] | None,
    in_press: bool | None,
    open_access: bool | None,
    suppressed: bool | None,
    within_library_holdings: bool | None,
    date_from: str | None,
    date_to: str | None,
    doi: str | None,
    pmid: str | None,
    q: str | None,
    use_simple_query: bool,
    sort: str | None,
    limit: int,
    offset: int,
    cursor: str | None,
    include_total: bool,
) -> ArticlePage:
    """
    List articles using the materialized listing table.

    Args:
        db: Database connection.
        journal_id: Filter by journal IDs.
        issue_id: Filter by issue ID.
        year: Filter by publication year from issues.
        area: Filter by journal area (multiple allowed).
        in_press: Filter by in_press flag.
        open_access: Filter by open_access flag.
        suppressed: Filter by suppressed flag.
        within_library_holdings: Filter by holdings flag.
        date_from: Minimum article date.
        date_to: Maximum article date.
        doi: Filter by DOI.
        pmid: Filter by PMID.
        q: Full-text search query for FTS5.
        use_simple_query: Whether to wrap queries with simple_query().
        sort: Sort string for articles.
        limit: Page size.
        offset: Page offset.
        cursor: Keyset cursor for pagination.
        include_total: Whether to compute total count.

    Returns:
        Paginated article list.
    """
    where_clauses: list[str] = []
    params: list[Any] = []

    if journal_id:
        placeholders = ", ".join(["?"] * len(journal_id))
        where_clauses.append(f"l.journal_id IN ({placeholders})")
        params.extend(journal_id)
    if issue_id is not None:
        where_clauses.append("l.issue_id = ?")
        params.append(issue_id)
    if area:
        placeholders = ", ".join(["?"] * len(area))
        where_clauses.append(f"l.area IN ({placeholders})")
        params.extend(area)
    if in_press is not None:
        where_clauses.append("l.in_press = ?")
        params.append(1 if in_press else 0)
    if open_access is not None:
        where_clauses.append("l.open_access = ?")
        params.append(1 if open_access else 0)
    if suppressed is not None:
        where_clauses.append("l.suppressed = ?")
        params.append(1 if suppressed else 0)
    if within_library_holdings is not None:
        where_clauses.append("l.within_library_holdings = ?")
        params.append(1 if within_library_holdings else 0)
    if date_from:
        where_clauses.append("l.date >= ?")
        params.append(date_from)
    if date_to:
        where_clauses.append("l.date <= ?")
        params.append(date_to)
    if doi:
        where_clauses.append("l.doi = ?")
        params.append(doi)
    if pmid:
        where_clauses.append("l.pmid = ?")
        params.append(pmid)
    if year is not None:
        where_clauses.append("l.publication_year = ?")
        params.append(year)
    if q and q.strip():
        matcher = "simple_query(?)" if use_simple_query else "?"
        fts_clause = (
            "l.article_id IN ("
            f"SELECT rowid FROM article_search WHERE article_search MATCH {matcher}"
            ")"
        )
        where_clauses.append(fts_clause)
        params.append(q.strip())

    sort_specs = parse_sort(sort, ARTICLE_SORT_FIELDS)
    supports_keyset = len(sort_specs) == 1 and sort_specs[0].column == "l.date"
    if not supports_keyset:
        raise HTTPException(
            status_code=400,
            detail="Articles only support sort=date:desc or date:asc",
        )
    direction = sort_specs[0].direction
    order_sql = f" ORDER BY l.date {direction}, l.article_id {direction}"

    if cursor:
        cursor_date, cursor_id = parse_article_cursor(cursor)
        operator = "<" if direction == "DESC" else ">"
        where_clauses.append(
            f"(l.date {operator} ? OR (l.date = ? AND l.article_id {operator} ?))"
        )
        params.extend([cursor_date, cursor_date, cursor_id])

    where_sql = f"WHERE {' AND '.join(where_clauses)}" if where_clauses else ""

    total = None
    if include_total:
        count_row = await fetch_one(
            db,
            f"""
            SELECT COUNT(*) AS total
            FROM article_listing l
            {where_sql}
            """,
            params,
        )
        total = int(count_row["total"]) if count_row else 0

    pagination_sql = "LIMIT ?"
    pagination_params: list[Any] = [limit]
    if cursor is None:
        pagination_sql = f"{pagination_sql} OFFSET ?"
        pagination_params.append(offset)

    id_rows = await fetch_all(
        db,
        f"""
        SELECT
            l.article_id,
            l.date
        FROM article_listing l INDEXED BY idx_article_listing_date_id
        {where_sql}
        {order_sql}
        {pagination_sql}
        """,
        params + pagination_params,
    )

    has_more = len(id_rows) == limit
    next_cursor = None
    if id_rows and has_more:
        last_row = id_rows[-1]
        next_cursor = build_article_cursor(
            last_row.get("date"),
            int(last_row["article_id"]),
        )
        if next_cursor is None:
            has_more = False

    article_ids = [int(row["article_id"]) for row in id_rows]
    if not article_ids:
        return ArticlePage(
            items=[],
            page=build_page_meta(
                total=total,
                limit=limit,
                offset=offset,
                next_cursor=next_cursor,
                has_more=has_more,
            ),
        )

    placeholders = ", ".join(["?"] * len(article_ids))
    rows = await fetch_all(
        db,
        f"""
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            a.sync_id,
            a.title,
            a.date,
            a.authors,
            a.start_page,
            a.end_page,
            a.abstract,
            a.doi,
            a.pmid,
            a.ill_url,
            a.link_resolver_openurl_link,
            a.email_article_request_link,
            a.permalink,
            a.suppressed,
            a.in_press,
            a.open_access,
            a.platform_id,
            a.retraction_doi,
            a.retraction_date,
            a.retraction_related_urls,
            a.unpaywall_data_suppressed,
            a.expression_of_concern_doi,
            a.within_library_holdings,
            a.noodletools_export_link,
            a.avoid_unpaywall_publisher_links,
            a.content_location,
            a.full_text_file,
            a.nomad_fallback_url,
            j.title AS journal_title,
            i.volume,
            i.number
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
        """,
        article_ids,
    )

    row_map = {int(row["article_id"]): row for row in rows}
    ordered_rows = [
        row_map[article_id] for article_id in article_ids if article_id in row_map
    ]

    return ArticlePage(
        items=[ArticleRecord(**row) for row in ordered_rows],
        page=build_page_meta(
            total=total,
            limit=limit,
            offset=offset,
            next_cursor=next_cursor,
            has_more=has_more,
        ),
    )


async def list_articles_from_articles(
    db: aiosqlite.Connection,
    journal_id: list[int] | None,
    issue_id: int | None,
    year: int | None,
    area: list[str] | None,
    in_press: bool | None,
    open_access: bool | None,
    suppressed: bool | None,
    within_library_holdings: bool | None,
    date_from: str | None,
    date_to: str | None,
    doi: str | None,
    pmid: str | None,
    q: str | None,
    use_simple_query: bool,
    sort: str | None,
    limit: int,
    offset: int,
    cursor: str | None,
    include_total: bool,
) -> ArticlePage:
    """
    List articles using direct table joins when listing data is unavailable.

    Args:
        db: Database connection.
        journal_id: Filter by journal IDs.
        issue_id: Filter by issue ID.
        year: Filter by publication year from issues.
        area: Filter by journal area (multiple allowed).
        in_press: Filter by in_press flag.
        open_access: Filter by open_access flag.
        suppressed: Filter by suppressed flag.
        within_library_holdings: Filter by holdings flag.
        date_from: Minimum article date.
        date_to: Maximum article date.
        doi: Filter by DOI.
        pmid: Filter by PMID.
        q: Full-text search query for FTS5.
        use_simple_query: Whether to wrap queries with simple_query().
        sort: Sort string for articles.
        limit: Page size.
        offset: Page offset.
        cursor: Keyset cursor for pagination.
        include_total: Whether to compute total count.

    Returns:
        Paginated article list.
    """
    where_clauses: list[str] = []
    params: list[Any] = []
    join_meta = area is not None and len(area) > 0
    join_search = q is not None and q.strip() != ""
    join_issues = year is not None

    if journal_id:
        placeholders = ", ".join(["?"] * len(journal_id))
        where_clauses.append(f"a.journal_id IN ({placeholders})")
        params.extend(journal_id)
    if issue_id is not None:
        where_clauses.append("a.issue_id = ?")
        params.append(issue_id)
    if area:
        placeholders = ", ".join(["?"] * len(area))
        where_clauses.append(f"m.area IN ({placeholders})")
        params.extend(area)
    if in_press is not None:
        where_clauses.append("a.in_press = ?")
        params.append(1 if in_press else 0)
    if open_access is not None:
        where_clauses.append("a.open_access = ?")
        params.append(1 if open_access else 0)
    if suppressed is not None:
        where_clauses.append("a.suppressed = ?")
        params.append(1 if suppressed else 0)
    if within_library_holdings is not None:
        where_clauses.append("a.within_library_holdings = ?")
        params.append(1 if within_library_holdings else 0)
    if date_from:
        where_clauses.append("a.date >= ?")
        params.append(date_from)
    if date_to:
        where_clauses.append("a.date <= ?")
        params.append(date_to)
    if doi:
        where_clauses.append("a.doi = ?")
        params.append(doi)
    if pmid:
        where_clauses.append("a.pmid = ?")
        params.append(pmid)
    if year is not None:
        where_clauses.append("i.publication_year = ?")
        params.append(year)
    if q and q.strip():
        matcher = "simple_query(?)" if use_simple_query else "?"
        where_clauses.append(f"article_search MATCH {matcher}")
        params.append(q.strip())

    join_sql = []
    if join_issues:
        join_sql.append("JOIN issues i ON i.issue_id = a.issue_id")
    if join_search:
        join_sql.append(
            "JOIN article_search ON article_search.article_id = a.article_id"
        )
    if join_meta:
        join_sql.append("JOIN journal_meta m ON m.journal_id = a.journal_id")

    filter_joins = " ".join(join_sql)

    sort_specs = parse_sort(sort, {"date": "a.date"})
    supports_keyset = len(sort_specs) == 1 and sort_specs[0].column == "a.date"
    if not supports_keyset:
        raise HTTPException(
            status_code=400,
            detail="Articles only support sort=date:desc or date:asc",
        )
    direction = sort_specs[0].direction
    order_sql = f" ORDER BY a.date {direction}, a.article_id {direction}"

    if cursor:
        cursor_date, cursor_id = parse_article_cursor(cursor)
        operator = "<" if direction == "DESC" else ">"
        where_clauses.append(
            f"(a.date {operator} ? OR (a.date = ? AND a.article_id {operator} ?))"
        )
        params.extend([cursor_date, cursor_date, cursor_id])

    where_sql = f"WHERE {' AND '.join(where_clauses)}" if where_clauses else ""

    total = None
    if include_total:
        count_row = await fetch_one(
            db,
            f"""
            SELECT COUNT(*) AS total
            FROM articles a
            {filter_joins}
            {where_sql}
            """,
            params,
        )
        total = int(count_row["total"]) if count_row else 0

    pagination_sql = "LIMIT ?"
    pagination_params: list[Any] = [limit]
    if cursor is None:
        pagination_sql = f"{pagination_sql} OFFSET ?"
        pagination_params.append(offset)

    id_rows = await fetch_all(
        db,
        f"""
        SELECT
            a.article_id,
            a.date
        FROM articles a
        {filter_joins}
        {where_sql}
        {order_sql}
        {pagination_sql}
        """,
        params + pagination_params,
    )

    has_more = len(id_rows) == limit
    next_cursor = None
    if id_rows and has_more:
        last_row = id_rows[-1]
        next_cursor = build_article_cursor(
            last_row.get("date"),
            int(last_row["article_id"]),
        )
        if next_cursor is None:
            has_more = False

    article_ids = [int(row["article_id"]) for row in id_rows]
    if not article_ids:
        return ArticlePage(
            items=[],
            page=build_page_meta(
                total=total,
                limit=limit,
                offset=offset,
                next_cursor=next_cursor,
                has_more=has_more,
            ),
        )

    placeholders = ", ".join(["?"] * len(article_ids))
    rows = await fetch_all(
        db,
        f"""
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            a.sync_id,
            a.title,
            a.date,
            a.authors,
            a.start_page,
            a.end_page,
            a.abstract,
            a.doi,
            a.pmid,
            a.ill_url,
            a.link_resolver_openurl_link,
            a.email_article_request_link,
            a.permalink,
            a.suppressed,
            a.in_press,
            a.open_access,
            a.platform_id,
            a.retraction_doi,
            a.retraction_date,
            a.retraction_related_urls,
            a.unpaywall_data_suppressed,
            a.expression_of_concern_doi,
            a.within_library_holdings,
            a.noodletools_export_link,
            a.avoid_unpaywall_publisher_links,
            a.content_location,
            a.full_text_file,
            a.nomad_fallback_url,
            j.title AS journal_title,
            i.volume,
            i.number
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
        """,
        article_ids,
    )

    row_map = {int(row["article_id"]): row for row in rows}
    ordered_rows = [
        row_map[article_id] for article_id in article_ids if article_id in row_map
    ]

    return ArticlePage(
        items=[ArticleRecord(**row) for row in ordered_rows],
        page=build_page_meta(
            total=total,
            limit=limit,
            offset=offset,
            next_cursor=next_cursor,
            has_more=has_more,
        ),
    )


async def list_articles(
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
    journal_id: Annotated[list[int] | None, Query()] = None,
    issue_id: int | None = Query(default=None, ge=0),
    year: int | None = Query(default=None, ge=0),
    area: Annotated[list[str] | None, Query()] = None,
    in_press: bool | None = Query(default=None),
    open_access: bool | None = Query(default=None),
    suppressed: bool | None = Query(default=None),
    within_library_holdings: bool | None = Query(default=None),
    date_from: str | None = Query(default=None),
    date_to: str | None = Query(default=None),
    doi: str | None = Query(default=None),
    pmid: str | None = Query(default=None),
    q: str | None = Query(default=None, description="FTS query for article_search"),
    sort: str | None = Query(default="date:desc"),
    limit: int = Query(default=50, ge=1, le=MAX_LIMIT),
    offset: int = Query(default=0, ge=0),
    cursor: str | None = Query(default=None),
    include_total: bool = Query(default=True),
) -> ArticlePage:
    """
    List articles with filtering, FTS, and sorting.

    Args:
        journal_id: Filter by journal IDs.
        issue_id: Filter by issue ID.
        year: Filter by publication year from issues.
        area: Filter by journal area (multiple allowed).
        in_press: Filter by in_press flag.
        open_access: Filter by open_access flag.
        suppressed: Filter by suppressed flag.
        within_library_holdings: Filter by holdings flag.
        date_from: Minimum article date.
        date_to: Maximum article date.
        doi: Filter by DOI.
        pmid: Filter by PMID.
        q: Full-text search query for FTS5.
        sort: Multi-column sort string.
        limit: Page size.
        offset: Page offset.
        cursor: Keyset cursor for pagination.
        include_total: Whether to compute total count.
        db: Database connection.

    Returns:
        Paginated article list.
    """
    use_simple_search = await is_simple_search_enabled(db)
    use_simple_query = should_use_simple_query(q, use_simple_search)
    if await is_article_listing_ready(db):
        return await list_articles_from_listing(
            db,
            journal_id,
            issue_id,
            year,
            area,
            in_press,
            open_access,
            suppressed,
            within_library_holdings,
            date_from,
            date_to,
            doi,
            pmid,
            q,
            use_simple_query,
            sort,
            limit,
            offset,
            cursor,
            include_total,
        )

    return await list_articles_from_articles(
        db,
        journal_id,
        issue_id,
        year,
        area,
        in_press,
        open_access,
        suppressed,
        within_library_holdings,
        date_from,
        date_to,
        doi,
        pmid,
        q,
        use_simple_query,
        sort,
        limit,
        offset,
        cursor,
        include_total,
    )


async def get_article(
    article_id: int,
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> ArticleRecord:
    """
    Fetch a single article record.

    Args:
        article_id: Article identifier.
        db: Database connection.

    Returns:
        Article record.
    """
    row = await fetch_one(
        db,
        """
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            a.sync_id,
            a.title,
            a.date,
            a.authors,
            a.start_page,
            a.end_page,
            a.abstract,
            a.doi,
            a.pmid,
            a.ill_url,
            a.link_resolver_openurl_link,
            a.email_article_request_link,
            a.permalink,
            a.suppressed,
            a.in_press,
            a.open_access,
            a.platform_id,
            a.retraction_doi,
            a.retraction_date,
            a.retraction_related_urls,
            a.unpaywall_data_suppressed,
            a.expression_of_concern_doi,
            a.within_library_holdings,
            a.noodletools_export_link,
            a.avoid_unpaywall_publisher_links,
            a.content_location,
            a.full_text_file,
            a.nomad_fallback_url,
            j.title AS journal_title,
            i.volume,
            i.number
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.article_id = ?
        """,
        [article_id],
    )
    if not row:
        raise HTTPException(status_code=404, detail="Article not found")
    return ArticleRecord(**row)


async def redirect_article_fulltext(
    article_id: int,
    db: Annotated[aiosqlite.Connection, Depends(get_db_dependency)],
) -> RedirectResponse:
    """
    Redirect to a DOI or signed full text URL for an article.

    Args:
        article_id: Article identifier.
        db: Database connection.

    Returns:
        RedirectResponse to the resolved full text URL.
    """
    row = await fetch_one(
        db,
        """
        SELECT
            a.article_id,
            a.title,
            a.doi,
            a.platform_id,
            a.full_text_file,
            a.permalink,
            i.publication_year,
            i.number,
            j.issn,
            j.title AS journal_title
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        JOIN journals j ON j.journal_id = a.journal_id
        WHERE a.article_id = ?
        """,
        [article_id],
    )
    if not row:
        raise HTTPException(status_code=404, detail="Article not found")
    full_text_file = row.get("full_text_file")
    if full_text_file:
        return RedirectResponse(str(full_text_file))
    permalink = row.get("permalink")
    if permalink:
        return RedirectResponse(str(permalink))
    doi = row.get("doi")
    if doi:
        doi_text = str(doi).strip()
        if doi_text:
            return RedirectResponse(f"https://doi.org/{doi_text}")
    raise HTTPException(status_code=404, detail="Full text not available")
