"""Pagination and sorting helpers."""

from __future__ import annotations

from dataclasses import dataclass

from fastapi import HTTPException

from paper_scanner.api.models import PageMeta


@dataclass(frozen=True)
class SortSpec:
    """
    Sort specification entry.
    """

    column: str
    direction: str


def parse_sort(sort: str | None, allowed: dict[str, str]) -> list[SortSpec]:
    """
    Parse a multi-column sort string into SQL-safe specs.

    Args:
        sort: Comma-separated sort string.
        allowed: Mapping of public fields to SQL columns.

    Returns:
        List of sort specifications.
    """
    if not sort:
        return []
    specs: list[SortSpec] = []
    for raw_part in sort.split(","):
        part = raw_part.strip()
        if not part:
            continue
        direction = "ASC"
        field = part
        if part.startswith("-"):
            field = part[1:]
            direction = "DESC"
        elif ":" in part:
            field, raw_dir = part.split(":", 1)
            direction = "DESC" if raw_dir.strip().lower() == "desc" else "ASC"
        field = field.strip()
        column = allowed.get(field)
        if not column:
            raise HTTPException(
                status_code=400, detail=f"Unsupported sort field: {field}"
            )
        specs.append(SortSpec(column=column, direction=direction))
    return specs


def apply_sort(specs: list[SortSpec]) -> str:
    """
    Convert sort specs into an ORDER BY clause.

    Args:
        specs: List of sort specifications.

    Returns:
        ORDER BY clause or empty string.
    """
    if not specs:
        return ""
    parts = [f"{spec.column} {spec.direction}" for spec in specs]
    return f" ORDER BY {', '.join(parts)}"


def build_page_meta(
    total: int | None,
    limit: int,
    offset: int,
    next_cursor: str | None = None,
    has_more: bool | None = None,
) -> PageMeta:
    """
    Build pagination metadata.

    Args:
        total: Total rows.
        limit: Page size.
        offset: Page offset.
        next_cursor: Cursor for keyset pagination.
        has_more: Whether more rows are available.

    Returns:
        Page metadata.
    """
    return PageMeta(
        total=total,
        limit=limit,
        offset=offset,
        next_cursor=next_cursor,
        has_more=has_more,
    )


def parse_article_cursor(cursor: str) -> tuple[str, int]:
    """
    Parse a cursor string for keyset pagination.

    Args:
        cursor: Cursor string in "{date}|{article_id}" format.

    Returns:
        Tuple of date string and article id.
    """
    parts = cursor.split("|", 1)
    if len(parts) != 2 or not parts[0]:
        raise HTTPException(status_code=400, detail="Invalid cursor")
    try:
        article_id = int(parts[1])
    except ValueError as exc:
        raise HTTPException(status_code=400, detail="Invalid cursor") from exc
    return parts[0], article_id


def build_article_cursor(date_value: str | None, article_id: int) -> str | None:
    """
    Build a cursor string from an article row.

    Args:
        date_value: Article date value.
        article_id: Article identifier.

    Returns:
        Cursor string or None when date is missing.
    """
    if not date_value:
        return None
    return f"{date_value}|{article_id}"


JOURNAL_SORT_FIELDS = {
    "journal_id": "j.journal_id",
    "title": "j.title",
    "issn": "j.issn",
    "eissn": "j.eissn",
    "scimago_rank": "j.scimago_rank",
    "available": "j.available",
    "has_articles": "j.has_articles",
}

ISSUE_SORT_FIELDS = {
    "issue_id": "i.issue_id",
    "publication_year": "i.publication_year",
    "title": "i.title",
    "date": "i.date",
    "volume": "i.volume",
    "number": "i.number",
}

ARTICLE_SORT_FIELDS = {
    "date": "l.date",
}
