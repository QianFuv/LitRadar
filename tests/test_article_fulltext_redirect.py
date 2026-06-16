"""Tests for article full-text redirects."""

from __future__ import annotations

import sqlite3
import unittest
from unittest.mock import patch

import aiosqlite

from paper_scanner.api.queries.articles import (
    _is_cnki_protected_fulltext_url,
    redirect_article_fulltext,
)


async def create_fulltext_db(
    library_id: str,
    full_text_file: str | None,
    permalink: str | None,
) -> aiosqlite.Connection:
    """
    Build an in-memory article database for full-text redirect tests.

    Args:
        library_id: Journal source identifier.
        full_text_file: Article full-text URL value.
        permalink: Article permalink value.

    Returns:
        Open database connection with one article row.
    """
    db = await aiosqlite.connect(":memory:")
    db.row_factory = sqlite3.Row
    await db.executescript(
        """
        CREATE TABLE journals (
            journal_id INTEGER PRIMARY KEY,
            library_id TEXT,
            title TEXT,
            issn TEXT
        );
        CREATE TABLE issues (
            issue_id INTEGER PRIMARY KEY,
            publication_year INTEGER,
            number TEXT
        );
        CREATE TABLE articles (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            title TEXT,
            doi TEXT,
            platform_id TEXT,
            full_text_file TEXT,
            permalink TEXT
        );
        """
    )
    await db.execute(
        """
        INSERT INTO journals (journal_id, library_id, title, issn)
        VALUES (?, ?, ?, ?)
        """,
        (1, library_id, "Test Journal", "1000-0000"),
    )
    await db.execute(
        """
        INSERT INTO issues (issue_id, publication_year, number)
        VALUES (?, ?, ?)
        """,
        (2, 2026, "01"),
    )
    await db.execute(
        """
        INSERT INTO articles (
            article_id,
            journal_id,
            issue_id,
            title,
            doi,
            platform_id,
            full_text_file,
            permalink
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            10,
            1,
            2,
            "Test article",
            None,
            "TEST202601001",
            full_text_file,
            permalink,
        ),
    )
    await db.commit()
    return db


class ArticleFulltextRedirectTest(unittest.IsolatedAsyncioTestCase):
    """Test source-specific article full-text redirect behavior."""

    async def test_cnki_article_uses_permalink(self) -> None:
        """
        Ensure CNKI articles always use the article detail page.

        Returns:
            None.
        """
        db = await create_fulltext_db(
            "cnki",
            "https://example.test/fulltext.pdf",
            "https://oversea.cnki.net/openlink/detail?filename=CNKI202601001",
        )
        try:
            with patch(
                "paper_scanner.api.queries.articles._fulltext_redirect_url",
                side_effect=lambda url: str(url),
            ):
                response = await redirect_article_fulltext(10, db)

            self.assertEqual(
                response.headers["location"],
                "https://oversea.cnki.net/openlink/detail?filename=CNKI202601001",
            )
        finally:
            await db.close()

    async def test_non_cnki_article_uses_fulltext_file(self) -> None:
        """
        Ensure non-CNKI articles keep using stored full-text URLs first.

        Returns:
            None.
        """
        db = await create_fulltext_db(
            "scholarly",
            "https://example.test/fulltext.pdf",
            "https://doi.org/10.1000/test",
        )
        try:
            with patch(
                "paper_scanner.api.queries.articles._fulltext_redirect_url",
                side_effect=lambda url: str(url),
            ):
                response = await redirect_article_fulltext(10, db)

            self.assertEqual(
                response.headers["location"],
                "https://example.test/fulltext.pdf",
            )
        finally:
            await db.close()

    def test_cnki_protected_fulltext_detection(self) -> None:
        """
        Verify CNKI order-entry URL detection.

        Returns:
            None.
        """
        self.assertTrue(
            _is_cnki_protected_fulltext_url(
                "https://o.oversea.cnki.net/barnew/download/order?id=abc"
            )
        )
        self.assertFalse(
            _is_cnki_protected_fulltext_url(
                "https://oversea.cnki.net/openlink/detail?filename=abc"
            )
        )


if __name__ == "__main__":
    unittest.main()
