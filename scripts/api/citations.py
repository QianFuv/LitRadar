"""Minimal citation exporters for favorites downloads."""

from __future__ import annotations

import re
from collections.abc import Iterable
from xml.etree import ElementTree

from scripts.api.models import FavoriteArticleResponse


def _bibtex_escape(value: str) -> str:
    """
    Escape a value for BibTeX field output.

    Args:
        value: Raw field value.

    Returns:
        Escaped BibTeX string.
    """
    return (
        value.replace("\\", "\\\\")
        .replace("{", "\\{")
        .replace("}", "\\}")
        .replace("\n", " ")
    )


def _citation_key(article: FavoriteArticleResponse) -> str:
    """
    Build a stable citation key for one article.

    Args:
        article: Favorite article record.

    Returns:
        Citation key string.
    """
    return f"paper_scanner_{article.db_name.replace('.', '_')}_{article.article_id}"


def _extract_year(date_value: str | None) -> str:
    """
    Extract a four-digit year from a date string.

    Args:
        date_value: Raw article date.

    Returns:
        Four-digit year or an empty string.
    """
    if not date_value:
        return ""

    match = re.search(r"\b(\d{4})\b", date_value)
    return match.group(1) if match else ""


def _split_authors(authors: str | None) -> list[str]:
    """
    Split a free-form author string into individual names.

    Args:
        authors: Raw author string.

    Returns:
        List of author names.
    """
    if not authors:
        return []

    if ";" in authors:
        parts = authors.split(";")
    elif "\n" in authors:
        parts = authors.splitlines()
    elif " and " in authors:
        parts = authors.split(" and ")
    else:
        parts = [authors]

    return [part.strip(" ,;") for part in parts if part.strip(" ,;")]


def _iter_fields(article: FavoriteArticleResponse) -> Iterable[tuple[str, str]]:
    """
    Yield common citation fields for one article.

    Args:
        article: Favorite article record.

    Returns:
        Iterable of field name/value tuples.
    """
    authors = _split_authors(article.authors)
    year = _extract_year(article.date)
    issn = article.issn or article.eissn

    if article.title:
        yield "title", article.title
    if authors:
        yield "author", " and ".join(authors)
    if article.journal_title:
        yield "journal", article.journal_title
    if year:
        yield "year", year
    if article.volume:
        yield "volume", article.volume
    if article.number:
        yield "number", article.number
    if issn:
        yield "issn", issn
    if article.doi:
        yield "doi", article.doi
        yield "url", f"https://doi.org/{article.doi}"
    elif article.full_text_file:
        yield "url", article.full_text_file
    if article.abstract:
        yield "abstract", article.abstract


def to_bibtex(articles: list[FavoriteArticleResponse]) -> str:
    """
    Export favorites as BibTeX.

    Args:
        articles: Favorite article records.

    Returns:
        BibTeX text.
    """
    entries: list[str] = []

    for article in articles:
        fields = list(_iter_fields(article))
        lines = ["@article{" + _citation_key(article) + ","] + [
            f"  {name} = {{{_bibtex_escape(value)}}}," for name, value in fields
        ]
        lines.append("}")
        entries.append("\n".join(lines))

    if not entries:
        return ""

    return "\n\n".join(entries) + "\n"


def to_ris(articles: list[FavoriteArticleResponse]) -> str:
    """
    Export favorites as RIS.

    Args:
        articles: Favorite article records.

    Returns:
        RIS text.
    """
    lines: list[str] = []

    for article in articles:
        lines.append("TY  - JOUR")
        for author in _split_authors(article.authors):
            lines.append(f"AU  - {author}")
        if article.title:
            lines.append(f"TI  - {article.title}")
        if article.journal_title:
            lines.append(f"JO  - {article.journal_title}")
        year = _extract_year(article.date)
        if year:
            lines.append(f"PY  - {year}")
        if article.date:
            lines.append(f"DA  - {article.date}")
        if article.volume:
            lines.append(f"VL  - {article.volume}")
        if article.number:
            lines.append(f"IS  - {article.number}")
        if article.issn or article.eissn:
            lines.append(f"SN  - {article.issn or article.eissn}")
        if article.doi:
            lines.append(f"DO  - {article.doi}")
        if article.abstract:
            lines.append(f"AB  - {article.abstract.replace(chr(10), ' ')}")
        lines.append("ER  - ")
        lines.append("")

    return "\n".join(lines)


def to_endnote(articles: list[FavoriteArticleResponse]) -> str:
    """
    Export favorites as EndNote XML.

    Args:
        articles: Favorite article records.

    Returns:
        EndNote XML text.
    """
    root = ElementTree.Element("xml")
    records = ElementTree.SubElement(root, "records")

    for article in articles:
        record = ElementTree.SubElement(records, "record")
        ref_type = ElementTree.SubElement(
            record,
            "ref-type",
            {"name": "Journal Article"},
        )
        ref_type.text = "17"

        contributors = ElementTree.SubElement(record, "contributors")
        authors_node = ElementTree.SubElement(contributors, "authors")
        for author in _split_authors(article.authors):
            author_node = ElementTree.SubElement(authors_node, "author")
            author_node.text = author

        titles = ElementTree.SubElement(record, "titles")
        if article.title:
            title_node = ElementTree.SubElement(titles, "title")
            title_node.text = article.title
        if article.journal_title:
            secondary_title_node = ElementTree.SubElement(
                titles,
                "secondary-title",
            )
            secondary_title_node.text = article.journal_title

        periodical = ElementTree.SubElement(record, "periodical")
        if article.journal_title:
            full_title_node = ElementTree.SubElement(periodical, "full-title")
            full_title_node.text = article.journal_title

        dates = ElementTree.SubElement(record, "dates")
        year = _extract_year(article.date)
        if year:
            year_node = ElementTree.SubElement(dates, "year")
            year_node.text = year

        if article.volume:
            volume_node = ElementTree.SubElement(record, "volume")
            volume_node.text = article.volume
        if article.number:
            number_node = ElementTree.SubElement(record, "number")
            number_node.text = article.number
        if article.issn or article.eissn:
            issn_node = ElementTree.SubElement(record, "issn")
            issn_node.text = article.issn or article.eissn
        if article.abstract:
            abstract_node = ElementTree.SubElement(record, "abstract")
            abstract_node.text = article.abstract
        if article.doi:
            doi_node = ElementTree.SubElement(record, "electronic-resource-num")
            doi_node.text = article.doi

        if article.doi or article.full_text_file:
            urls = ElementTree.SubElement(record, "urls")
            related_urls = ElementTree.SubElement(urls, "related-urls")
            url_node = ElementTree.SubElement(related_urls, "url")
            if article.doi:
                url_node.text = f"https://doi.org/{article.doi}"
            else:
                url_node.text = article.full_text_file

    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        + ElementTree.tostring(root, encoding="unicode")
        + "\n"
    )
