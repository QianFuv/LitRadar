"""Data transformation helpers for scholarly and CNKI payloads."""

from __future__ import annotations

import html
import re
from pathlib import Path
from typing import Any

from paper_scanner.shared.converters import (
    to_bool_int,
    to_float,
    to_int,
    to_int_stable,
    to_text,
)


def source_from_row(csv_row: dict[str, str]) -> str:
    """
    Read a normalized source value from a CSV row.

    Args:
        csv_row: Source CSV row.

    Returns:
        Source identifier.
    """
    return (csv_row.get("source") or "scholarly").strip().lower()


def build_journal_id(csv_row: dict[str, str]) -> int | None:
    """
    Build a stable internal journal identifier.

    Args:
        csv_row: Source CSV row.

    Returns:
        Stable SQLite integer identifier.
    """
    source = source_from_row(csv_row)
    source_id = csv_row.get("id") or csv_row.get("issn") or csv_row.get("title")
    return to_int_stable(source_id, f"{source}:journal")


def build_meta_record(
    journal_id: int, csv_path: Path, csv_row: dict[str, str]
) -> dict[str, Any]:
    """
    Build CSV metadata for the journal.

    Args:
        journal_id: Internal journal ID.
        csv_path: Path to the source CSV.
        csv_row: Source CSV row.

    Returns:
        Dictionary of CSV metadata fields.
    """
    return {
        "journal_id": journal_id,
        "source_csv": csv_path.name,
        "area": csv_row.get("area"),
        "csv_title": csv_row.get("title"),
        "csv_issn": csv_row.get("issn"),
        "csv_library": source_from_row(csv_row),
    }


def build_scholarly_journal_record(
    journal_id: int,
    csv_row: dict[str, str],
    crossref_works: list[dict[str, Any]],
) -> dict[str, Any]:
    """
    Build a journal record for the scholarly source.

    Args:
        journal_id: Internal journal ID.
        csv_row: Source CSV row.
        crossref_works: Crossref works fetched for the journal.

    Returns:
        Dictionary of journal fields.
    """
    issn = csv_row.get("issn")
    eissn = None
    for work in crossref_works:
        issns = work.get("ISSN")
        if isinstance(issns, list):
            for candidate in issns:
                text = str(candidate).strip()
                if text and text != issn:
                    eissn = text
                    break
        if eissn:
            break
    return {
        "journal_id": journal_id,
        "library_id": source_from_row(csv_row),
        "platform_journal_id": csv_row.get("id") or issn,
        "title": csv_row.get("title"),
        "issn": issn,
        "eissn": eissn,
        "scimago_rank": None,
        "cover_url": None,
        "available": 1,
        "toc_data_approved_and_live": None,
        "has_articles": 1 if crossref_works else 0,
    }


def build_cnki_journal_record(
    journal_id: int,
    csv_row: dict[str, str],
    details: dict[str, Any] | None,
) -> dict[str, Any]:
    """
    Build a journal record for the CNKI source.

    Args:
        journal_id: Internal journal ID.
        csv_row: Source CSV row.
        details: CNKI journal detail payload.

    Returns:
        Dictionary of journal fields.
    """
    return {
        "journal_id": journal_id,
        "library_id": source_from_row(csv_row),
        "platform_journal_id": (details or {}).get("pykm") or csv_row.get("id"),
        "title": (details or {}).get("title") or csv_row.get("title"),
        "issn": (details or {}).get("issn") or csv_row.get("issn"),
        "eissn": None,
        "scimago_rank": to_float((details or {}).get("impact_factor")),
        "cover_url": (details or {}).get("cover_url"),
        "available": 1 if details else 0,
        "toc_data_approved_and_live": None,
        "has_articles": 1 if details else 0,
    }


def build_scholarly_issue_record(
    journal_id: int, work: dict[str, Any]
) -> dict[str, Any] | None:
    """
    Build an issue record from Crossref metadata.

    Args:
        journal_id: Internal journal ID.
        work: Crossref work payload.

    Returns:
        Issue record or None when no publication year exists.
    """
    date = crossref_publication_date(work)
    year = _year_from_date(date)
    if year is None:
        return None
    volume = _clean_text(work.get("volume"))
    number = _clean_text(work.get("issue")) or "in-press"
    issue_id = to_int_stable(
        f"{journal_id}:{year}:{volume or ''}:{number}", "scholarly:issue"
    )
    if not issue_id:
        return None
    return {
        "issue_id": issue_id,
        "journal_id": journal_id,
        "publication_year": year,
        "title": f"{year} {volume or ''} {number}".strip(),
        "volume": volume,
        "number": None if number == "in-press" else number,
        "date": date,
        "is_valid_issue": 1,
        "suppressed": None,
        "embargoed": None,
        "within_subscription": None,
    }


def build_cnki_issue_record(
    journal_id: int,
    journal_code: str,
    issue: dict[str, Any],
) -> dict[str, Any] | None:
    """
    Build an issue record from CNKI issue data.

    Args:
        journal_id: Internal journal ID.
        journal_code: CNKI journal code.
        issue: CNKI issue payload.

    Returns:
        Issue record or None.
    """
    year = to_int(issue.get("year"))
    number = _clean_text(issue.get("number"))
    if year is None or not number:
        return None
    issue_id = to_int_stable(f"{journal_code}:{year}:{number}", "cnki:issue")
    if not issue_id:
        return None
    return {
        "issue_id": issue_id,
        "journal_id": journal_id,
        "publication_year": year,
        "title": issue.get("title") or f"{year}年第{number}期",
        "volume": None,
        "number": number,
        "date": f"{year}-01-01",
        "is_valid_issue": 1,
        "suppressed": None,
        "embargoed": None,
        "within_subscription": None,
    }


def build_scholarly_article_record(
    work: dict[str, Any],
    openalex_work: dict[str, Any] | None,
    unpaywall_record: dict[str, Any] | None,
    journal_id: int,
    issue_id: int | None,
) -> dict[str, Any] | None:
    """
    Build an article record from Crossref and enrichment payloads.

    Args:
        work: Crossref work payload.
        openalex_work: OpenAlex work payload.
        unpaywall_record: Unpaywall record.
        journal_id: Internal journal ID.
        issue_id: Internal issue ID.

    Returns:
        Article record or None.
    """
    doi = normalize_doi(work.get("DOI"))
    platform_id = doi or _clean_text(work.get("URL"))
    article_id = to_int_stable(platform_id, "scholarly:article")
    if not article_id:
        return None

    page = _clean_text(work.get("page")) or _clean_text(work.get("article-number"))
    start_page, end_page = split_page_range(page)
    openalex_abstract = restore_openalex_abstract(
        (openalex_work or {}).get("abstract_inverted_index")
    )
    unpaywall_location = (unpaywall_record or {}).get("best_oa_location") or {}
    openalex_location = (openalex_work or {}).get("best_oa_location") or {}
    full_text_url = _first_text(
        [
            unpaywall_location.get("url_for_pdf"),
            openalex_location.get("pdf_url"),
            unpaywall_location.get("url_for_landing_page"),
            openalex_location.get("landing_page_url"),
        ]
    )
    landing_page_url = _first_text(
        [
            unpaywall_location.get("url_for_landing_page"),
            openalex_location.get("landing_page_url"),
            work.get("URL"),
        ]
    )
    is_open_access = (
        to_bool_int((unpaywall_record or {}).get("is_oa"))
        or to_bool_int(((openalex_work or {}).get("open_access") or {}).get("is_oa"))
        or 0
    )

    return _blank_article_record(
        article_id=article_id,
        journal_id=journal_id,
        issue_id=issue_id,
        title=_first_text(work.get("title")) or (openalex_work or {}).get("title"),
        date=crossref_publication_date(work)
        or (openalex_work or {}).get("publication_date"),
        authors=format_crossref_authors(work.get("author"))
        or format_openalex_authors((openalex_work or {}).get("authorships")),
        start_page=start_page,
        end_page=end_page,
        abstract=strip_markup(_clean_text(work.get("abstract"))) or openalex_abstract,
        doi=doi,
        pmid=((openalex_work or {}).get("ids") or {}).get("pmid"),
        permalink=f"https://doi.org/{doi}" if doi else landing_page_url,
        open_access=is_open_access,
        platform_id=platform_id,
        retraction_doi=_relation_doi(work.get("relation")),
        full_text_file=full_text_url,
        content_location=landing_page_url,
        in_press=1 if issue_id is None else None,
    )


def build_cnki_article_record(
    detail: dict[str, Any] | None,
    summary: dict[str, Any],
    journal_id: int,
    issue_id: int | None,
) -> dict[str, Any] | None:
    """
    Build an article record from CNKI detail and summary data.

    Args:
        detail: CNKI article detail payload.
        summary: CNKI article summary payload.
        journal_id: Internal journal ID.
        issue_id: Internal issue ID.

    Returns:
        Article record or None.
    """
    payload = detail or {}
    platform_id = payload.get("platform_id") or summary.get("platform_id")
    article_id = to_int_stable(
        platform_id or summary.get("article_url"), "cnki:article"
    )
    if not article_id:
        return None
    page = _clean_text(payload.get("pages") or summary.get("pages"))
    start_page, end_page = split_page_range(page)
    doi = normalize_doi(payload.get("doi"))
    permalink = payload.get("permalink") or summary.get("article_url")
    return _blank_article_record(
        article_id=article_id,
        journal_id=journal_id,
        issue_id=issue_id,
        title=payload.get("title") or summary.get("title"),
        date=payload.get("online_release_date") or summary.get("date"),
        authors=payload.get("authors") or summary.get("authors"),
        start_page=start_page,
        end_page=end_page,
        abstract=payload.get("abstract"),
        doi=doi,
        permalink=permalink,
        open_access=to_bool_int(summary.get("is_free")),
        platform_id=str(platform_id) if platform_id else None,
        full_text_file=payload.get("html_read_url"),
        content_location=payload.get("content_location") or permalink,
    )


def _blank_article_record(**values: Any) -> dict[str, Any]:
    """
    Build a complete article record with default None values.

    Args:
        values: Field overrides.

    Returns:
        Complete article record.
    """
    record = {
        "article_id": None,
        "journal_id": None,
        "issue_id": None,
        "sync_id": None,
        "title": None,
        "date": None,
        "authors": None,
        "start_page": None,
        "end_page": None,
        "abstract": None,
        "doi": None,
        "pmid": None,
        "ill_url": None,
        "link_resolver_openurl_link": None,
        "email_article_request_link": None,
        "permalink": None,
        "suppressed": None,
        "in_press": None,
        "open_access": None,
        "platform_id": None,
        "retraction_doi": None,
        "retraction_date": None,
        "retraction_related_urls": None,
        "unpaywall_data_suppressed": None,
        "expression_of_concern_doi": None,
        "within_library_holdings": None,
        "noodletools_export_link": None,
        "avoid_unpaywall_publisher_links": None,
        "content_location": None,
        "full_text_file": None,
        "nomad_fallback_url": None,
    }
    record.update(values)
    return record


def crossref_publication_date(work: dict[str, Any]) -> str | None:
    """
    Extract a normalized Crossref publication date.

    Args:
        work: Crossref work payload.

    Returns:
        ISO-like date string or None.
    """
    for key in ("published-print", "published-online", "published", "issued"):
        date = _crossref_date_parts(work.get(key))
        if date:
            return date
    return None


def _crossref_date_parts(value: Any) -> str | None:
    """
    Convert Crossref date-parts payload to a date string.

    Args:
        value: Crossref date payload.

    Returns:
        Date string or None.
    """
    if not isinstance(value, dict):
        return None
    parts = value.get("date-parts")
    if not isinstance(parts, list) or not parts:
        return None
    first = parts[0]
    if not isinstance(first, list) or not first:
        return None
    try:
        year = int(first[0])
        month = int(first[1]) if len(first) > 1 else 1
        day = int(first[2]) if len(first) > 2 else 1
    except (TypeError, ValueError):
        return None
    return f"{year:04d}-{month:02d}-{day:02d}"


def normalize_doi(value: Any) -> str | None:
    """
    Normalize DOI-like values.

    Args:
        value: Raw DOI.

    Returns:
        Bare lowercase DOI or None.
    """
    text = _clean_text(value)
    if not text:
        return None
    lowered = text.lower()
    for prefix in ("https://doi.org/", "http://doi.org/", "doi:"):
        if lowered.startswith(prefix):
            lowered = lowered[len(prefix) :]
            break
    return lowered.strip() or None


def format_crossref_authors(value: Any) -> str | None:
    """
    Format Crossref author payloads.

    Args:
        value: Crossref author list.

    Returns:
        Formatted author string or None.
    """
    if not isinstance(value, list):
        return None
    names: list[str] = []
    for item in value:
        if not isinstance(item, dict):
            continue
        parts = [
            _clean_text(item.get("given")),
            _clean_text(item.get("family")),
        ]
        name = " ".join(part for part in parts if part)
        if name:
            names.append(name)
    return "; ".join(names) if names else None


def format_openalex_authors(value: Any) -> str | None:
    """
    Format OpenAlex authorships.

    Args:
        value: OpenAlex authorships list.

    Returns:
        Formatted author string or None.
    """
    if not isinstance(value, list):
        return None
    names: list[str] = []
    for item in value:
        if not isinstance(item, dict):
            continue
        author = item.get("author")
        if isinstance(author, dict):
            name = _clean_text(author.get("display_name"))
            if name:
                names.append(name)
    return "; ".join(names) if names else None


def restore_openalex_abstract(value: Any) -> str | None:
    """
    Restore OpenAlex inverted-index abstracts.

    Args:
        value: OpenAlex abstract_inverted_index payload.

    Returns:
        Abstract text or None.
    """
    if not isinstance(value, dict):
        return None
    positions: list[tuple[int, str]] = []
    for word, indexes in value.items():
        if not isinstance(indexes, list):
            continue
        for index in indexes:
            if isinstance(index, int):
                positions.append((index, str(word)))
    if not positions:
        return None
    return " ".join(word for _, word in sorted(positions))


def strip_markup(value: str | None) -> str | None:
    """
    Strip simple XML or HTML tags from text.

    Args:
        value: Raw text.

    Returns:
        Clean text or None.
    """
    if not value:
        return None
    text = re.sub(r"<[^>]+>", " ", value)
    text = html.unescape(text)
    return re.sub(r"\s+", " ", text).strip() or None


def split_page_range(value: str | None) -> tuple[str | None, str | None]:
    """
    Split a page range into start and end page values.

    Args:
        value: Page range text.

    Returns:
        Tuple of start and end page.
    """
    text = _clean_text(value)
    if not text:
        return None, None
    for separator in ("-", "–", "—"):
        if separator in text:
            start, end = text.split(separator, 1)
            return _clean_text(start), _clean_text(end)
    return text, None


def _relation_doi(value: Any) -> str | None:
    """
    Extract a related DOI from Crossref relation metadata.

    Args:
        value: Crossref relation payload.

    Returns:
        Related DOI or None.
    """
    if not isinstance(value, dict):
        return None
    for relation in value.values():
        if not isinstance(relation, list):
            continue
        for item in relation:
            if isinstance(item, dict):
                doi = normalize_doi(item.get("id"))
                if doi:
                    return doi
    return None


def _year_from_date(value: str | None) -> int | None:
    """
    Extract a year from a date string.

    Args:
        value: Date string.

    Returns:
        Year or None.
    """
    if not value or len(value) < 4:
        return None
    return to_int(value[:4])


def _first_text(value: Any) -> str | None:
    """
    Return the first non-empty text from a value or list.

    Args:
        value: Source value.

    Returns:
        Text or None.
    """
    if isinstance(value, list):
        for item in value:
            text = _clean_text(item)
            if text:
                return text
        return None
    return _clean_text(value)


def _clean_text(value: Any) -> str | None:
    """
    Convert a value to stripped text.

    Args:
        value: Raw value.

    Returns:
        Clean text or None.
    """
    text = to_text(value)
    if text is None:
        return None
    stripped = text.strip()
    return stripped or None
