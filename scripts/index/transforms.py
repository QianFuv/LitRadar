"""Data transformation helpers for BrowZine and WeiPu payloads."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from scripts.shared.converters import (
    to_bool_int,
    to_float,
    to_int,
    to_int_stable,
    to_text,
)


def build_journal_record(
    journal_id: int,
    library_id: str,
    csv_row: dict[str, str],
    journal_info: dict[str, Any] | None,
) -> dict[str, Any]:
    """
    Build a journal record for database insertion.

    Args:
        journal_id: BrowZine journal ID.
        library_id: Library ID used for the journal.
        csv_row: Source CSV row.
        journal_info: BrowZine journal payload.

    Returns:
        Dictionary of journal fields.
    """
    attrs = journal_info.get("attributes", {}) if journal_info else {}

    def pick_attr(*keys: str) -> Any:
        """
        Choose the first available attribute key.

        Args:
            keys: Attribute keys to check.

        Returns:
            Attribute value or None.
        """
        for key in keys:
            if key in attrs:
                return attrs[key]
        return None

    return {
        "journal_id": journal_id,
        "library_id": library_id,
        "platform_journal_id": str(journal_id),
        "title": pick_attr("title") or csv_row.get("title"),
        "issn": pick_attr("issn") or csv_row.get("issn"),
        "eissn": pick_attr("eissn"),
        "scimago_rank": to_float(pick_attr("scimagoRank", "scimago_rank")),
        "cover_url": pick_attr("coverURL", "coverUrl"),
        "available": to_bool_int(pick_attr("available")),
        "toc_data_approved_and_live": to_bool_int(
            pick_attr("tocDataApprovedAndLive", "toc_data_approved_and_live")
        ),
        "has_articles": to_bool_int(pick_attr("hasArticles", "has_articles")),
    }


def build_meta_record(
    journal_id: int, csv_path: Path, csv_row: dict[str, str]
) -> dict[str, Any]:
    """
    Build CSV metadata for the journal.

    Args:
        journal_id: BrowZine journal ID.
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
        "csv_library": csv_row.get("library"),
    }


def format_weipu_authors(authors: Any) -> str | None:
    """
    Format WeiPu authors into a semicolon-delimited string.

    Args:
        authors: WeiPu author payload.

    Returns:
        Formatted author string or None.
    """
    if authors is None:
        return None
    if isinstance(authors, str):
        text = authors.strip()
        return text or None
    if isinstance(authors, list):
        names: list[str] = []
        for item in authors:
            if isinstance(item, dict):
                name = item.get("name") or item.get("authorName") or item.get("author")
                if name:
                    names.append(str(name))
            else:
                text = str(item).strip()
                if text:
                    names.append(text)
        return "; ".join(names) if names else None
    text = str(authors).strip()
    return text or None


def extract_weipu_page_range(
    pages: dict[str, Any] | None,
) -> tuple[str | None, str | None]:
    """
    Extract page range from WeiPu page data.

    Args:
        pages: WeiPu page dictionary.

    Returns:
        Tuple of (start_page, end_page).
    """
    if not pages:
        return None, None
    start_page = pages.get("begin")
    end_page = pages.get("end")
    start_text = str(start_page).strip() if start_page is not None else None
    end_text = str(end_page).strip() if end_page is not None else None
    return start_text or None, end_text or None


def is_numeric_page(value: str | None) -> bool:
    """
    Check whether a page value contains only digits.

    Args:
        value: Page value string.

    Returns:
        True when the value is empty or numeric-only.
    """
    if value is None:
        return True
    text = value.strip()
    if not text:
        return True
    return bool(re.fullmatch(r"\d+", text))


def build_weipu_journal_record(
    journal_id: int,
    library_id: str,
    platform_journal_id: str | None,
    csv_row: dict[str, str],
    journal_info: dict[str, Any] | None,
    has_articles: bool,
) -> dict[str, Any]:
    """
    Build a WeiPu journal record for database insertion.

    Args:
        journal_id: Internal journal ID.
        library_id: Library identifier.
        platform_journal_id: Resolved WeiPu journal identifier.
        csv_row: Source CSV row.
        journal_info: WeiPu journal payload.
        has_articles: Whether the journal has articles.

    Returns:
        Dictionary of journal fields.
    """
    title = None
    issn = None
    if journal_info:
        title = journal_info.get("journalName") or journal_info.get("name")
        issn = journal_info.get("issn")
    return {
        "journal_id": journal_id,
        "library_id": library_id,
        "platform_journal_id": platform_journal_id,
        "title": title or csv_row.get("title"),
        "issn": issn or csv_row.get("issn"),
        "eissn": None,
        "scimago_rank": None,
        "cover_url": None,
        "available": 1 if journal_info else 0,
        "toc_data_approved_and_live": None,
        "has_articles": 1 if has_articles else 0,
    }


def build_weipu_issue_record(
    issue: dict[str, Any], journal_id: int, year: int | None
) -> dict[str, Any] | None:
    """
    Build a WeiPu issue record for database insertion.

    Args:
        issue: WeiPu issue payload.
        journal_id: Internal journal ID.
        year: Publication year if available.

    Returns:
        Dictionary of issue fields or None when issue ID is missing.
    """
    issue_id = to_int_stable(issue.get("id"), f"weipu-issue:{journal_id}")
    if not issue_id:
        return None
    title = issue.get("name") or issue.get("title")
    number = issue.get("name") or issue.get("number")
    return {
        "issue_id": issue_id,
        "journal_id": journal_id,
        "publication_year": year,
        "title": title,
        "volume": None,
        "number": number,
        "date": None,
        "is_valid_issue": 1,
        "suppressed": None,
        "embargoed": None,
        "within_subscription": None,
    }


def build_weipu_article_record(
    article: dict[str, Any],
    journal_id: int,
    issue_id: int | None,
) -> dict[str, Any] | None:
    """
    Build a WeiPu article record for database insertion.

    Args:
        article: WeiPu article payload.
        journal_id: Internal journal ID.
        issue_id: Internal issue ID.

    Returns:
        Dictionary of article fields or None when article ID is missing.
    """
    article_id = to_int_stable(article.get("id"), f"weipu-article:{journal_id}")
    if not article_id:
        return None
    pages = article.get("pages") if isinstance(article.get("pages"), dict) else None
    start_page, end_page = extract_weipu_page_range(pages)
    if not is_numeric_page(start_page) or not is_numeric_page(end_page):
        return None
    publish_date = (
        article.get("publishDate") or article.get("pubDate") or article.get("date")
    )
    return {
        "article_id": article_id,
        "journal_id": journal_id,
        "issue_id": issue_id,
        "sync_id": None,
        "title": article.get("title"),
        "date": publish_date,
        "authors": format_weipu_authors(article.get("authors")),
        "start_page": start_page,
        "end_page": end_page,
        "abstract": article.get("abstract"),
        "doi": article.get("doi"),
        "pmid": None,
        "ill_url": None,
        "link_resolver_openurl_link": None,
        "email_article_request_link": None,
        "permalink": None,
        "suppressed": None,
        "in_press": None,
        "open_access": None,
        "platform_id": str(article.get("id")) if article.get("id") else None,
        "retraction_doi": None,
        "retraction_date": None,
        "retraction_related_urls": None,
        "unpaywall_data_suppressed": None,
        "expression_of_concern_doi": None,
        "within_library_holdings": None,
        "noodletools_export_link": None,
        "avoid_unpaywall_publisher_links": None,
        "browzine_web_in_context_link": None,
        "content_location": None,
        "libkey_content_location": None,
        "full_text_file": None,
        "libkey_full_text_file": None,
        "nomad_fallback_url": None,
    }


def build_issue_record(
    issue: dict[str, Any], journal_id: int, year: int
) -> dict[str, Any] | None:
    """
    Build an issue record for database insertion.

    Args:
        issue: Issue payload.
        journal_id: BrowZine journal ID fallback.
        year: Publication year.

    Returns:
        Dictionary of issue fields or None when issue ID is missing.
    """
    issue_id = to_int(issue.get("id"))
    if not issue_id:
        return None
    attrs = issue.get("attributes", {})
    return {
        "issue_id": issue_id,
        "journal_id": to_int(attrs.get("journal")) or journal_id,
        "publication_year": year,
        "title": attrs.get("title"),
        "volume": attrs.get("volume"),
        "number": attrs.get("number"),
        "date": attrs.get("date"),
        "is_valid_issue": to_bool_int(attrs.get("isValidIssue")),
        "suppressed": to_bool_int(attrs.get("suppressed")),
        "embargoed": to_bool_int(attrs.get("embargoed")),
        "within_subscription": to_bool_int(attrs.get("withinSubscription")),
    }


def build_article_record(
    article: dict[str, Any],
    fallback_journal_id: int,
    fallback_issue_id: int | None,
) -> dict[str, Any] | None:
    """
    Build an article record for database insertion.

    Args:
        article: Article payload.
        fallback_journal_id: Journal ID fallback when relationship is missing.
        fallback_issue_id: Issue ID fallback when relationship is missing.

    Returns:
        Dictionary of article fields or None when article ID is missing.
    """
    article_id = to_int(article.get("id"))
    if not article_id:
        return None
    attrs = article.get("attributes", {})
    relationships = article.get("relationships", {})
    journal_rel = relationships.get("journal", {}).get("data", {})
    issue_rel = relationships.get("issue", {}).get("data", {})

    journal_id = to_int(journal_rel.get("id")) or fallback_journal_id
    issue_id = to_int(issue_rel.get("id")) or fallback_issue_id

    return {
        "article_id": article_id,
        "journal_id": journal_id,
        "issue_id": issue_id,
        "sync_id": to_int(attrs.get("syncId")),
        "title": attrs.get("title"),
        "date": attrs.get("date"),
        "authors": attrs.get("authors"),
        "start_page": attrs.get("startPage"),
        "end_page": attrs.get("endPage"),
        "abstract": attrs.get("abstract"),
        "doi": attrs.get("doi"),
        "pmid": attrs.get("pmid"),
        "ill_url": attrs.get("ILLURL"),
        "link_resolver_openurl_link": attrs.get("linkResolverOpenurlLink"),
        "email_article_request_link": attrs.get("emailArticleRequestLink"),
        "permalink": attrs.get("permalink"),
        "suppressed": to_bool_int(attrs.get("suppressed")),
        "in_press": to_bool_int(attrs.get("inPress")),
        "open_access": to_bool_int(attrs.get("openAccess")),
        "platform_id": attrs.get("platformId"),
        "retraction_doi": attrs.get("retractionDoi"),
        "retraction_date": attrs.get("retractionDate"),
        "retraction_related_urls": to_text(attrs.get("retractionRelatedUrls")),
        "unpaywall_data_suppressed": to_bool_int(attrs.get("unpaywallDataSuppressed")),
        "expression_of_concern_doi": attrs.get("expressionOfConcernDoi"),
        "within_library_holdings": to_bool_int(attrs.get("withinLibraryHoldings")),
        "noodletools_export_link": attrs.get("noodleToolsExportLink"),
        "avoid_unpaywall_publisher_links": to_bool_int(
            attrs.get("avoidUnpaywallPublisherLinks")
        ),
        "browzine_web_in_context_link": attrs.get("browzineWebInContextLink"),
        "content_location": attrs.get("contentLocation"),
        "libkey_content_location": attrs.get("libkeyContentLocation"),
        "full_text_file": attrs.get("fullTextFile"),
        "libkey_full_text_file": attrs.get("libkeyFullTextFile"),
        "nomad_fallback_url": attrs.get("nomadFallbackURL"),
    }
