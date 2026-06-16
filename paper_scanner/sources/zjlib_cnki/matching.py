"""Exact metadata matching helpers for Zhejiang Library CNKI full text."""

from __future__ import annotations

import html
import re
import unicodedata
from dataclasses import dataclass

AUTHOR_SEPARATOR_RE = re.compile(r"[;；,，、]+")
IGNORED_TEXT_RE = re.compile(
    r"[\s\"'.,:;!?()\[\]{}<>《》〈〉“”‘’·\-–—_/\\|，。？！：；（）【】]+"
)


@dataclass(frozen=True)
class ArticleIdentity:
    """Article metadata required for exact full-text matching."""

    title: str
    authors: str
    journal_title: str


def normalize_exact_text(value: object) -> str:
    """
    Normalize text for exact CNKI metadata comparison.

    Args:
        value: Raw metadata value.

    Returns:
        Canonical text used for equality checks.
    """
    text = html.unescape(str(value or ""))
    text = unicodedata.normalize("NFKC", text).casefold()
    return IGNORED_TEXT_RE.sub("", text)


def split_author_names(value: object) -> list[str]:
    """
    Split an author list into normalized author tokens.

    Args:
        value: Raw author list.

    Returns:
        Normalized author names in source order.
    """
    text = unicodedata.normalize("NFKC", html.unescape(str(value or ""))).strip()
    if not text:
        return []
    names = [name for name in AUTHOR_SEPARATOR_RE.split(text) if name.strip()]
    if not names:
        names = [text]
    return [normalize_exact_text(name) for name in names if normalize_exact_text(name)]


def do_authors_match(expected: object, actual: object) -> bool:
    """
    Compare author lists using normalized exact order.

    Args:
        expected: Expected author list.
        actual: Candidate author list.

    Returns:
        True when both normalized author lists are non-empty and equal.
    """
    expected_names = split_author_names(expected)
    actual_names = split_author_names(actual)
    return bool(expected_names and actual_names and expected_names == actual_names)


def do_titles_match(expected: object, actual: object) -> bool:
    """
    Compare article titles using normalized exact text.

    Args:
        expected: Expected title.
        actual: Candidate title.

    Returns:
        True when both normalized titles are non-empty and equal.
    """
    expected_title = normalize_exact_text(expected)
    actual_title = normalize_exact_text(actual)
    return bool(expected_title and actual_title and expected_title == actual_title)


def do_journals_match(expected: object, actual: object) -> bool:
    """
    Compare journal titles using normalized exact text.

    Args:
        expected: Expected journal title.
        actual: Candidate journal title.

    Returns:
        True when both normalized journal names are non-empty and equal.
    """
    expected_journal = normalize_exact_text(expected)
    actual_journal = normalize_exact_text(actual)
    return bool(
        expected_journal and actual_journal and expected_journal == actual_journal
    )


def does_article_metadata_match(
    expected: ArticleIdentity,
    actual: ArticleIdentity,
) -> bool:
    """
    Check whether article title, authors, and journal all match exactly.

    Args:
        expected: Expected article metadata from the index database.
        actual: Candidate article metadata parsed from CNKI.

    Returns:
        True when title, authors, and journal all match after normalization.
    """
    return (
        do_titles_match(expected.title, actual.title)
        and do_authors_match(expected.authors, actual.authors)
        and do_journals_match(expected.journal_title, actual.journal_title)
    )
