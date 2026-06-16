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


class ArticleFulltextRedirectTest(unittest.IsolatedAsyncioTestCase):
    """Test source-specific article full-text redirect behavior."""

    async def test_cnki_protected_fulltext_uses_permalink(self) -> None:
        """
        Ensure CNKI protected order links fall back to article detail pages.

        Returns:
            None.
        """
        db = await aiosqlite.connect(":memory:")
        db.row_factory = sqlite3.Row
        try:
            await db.executescript(
                """
                CREATE TABLE journals (
                    journal_id INTEGER PRIMARY KEY,
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
                "INSERT INTO journals (journal_id, title, issn) VALUES (?, ?, ?)",
                (1, "CNKI Journal", "1000-0000"),
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
                    "CNKI article",
                    None,
                    "CNKI202601001",
                    "https://o.oversea.cnki.net/barnew/download/order?id=abc",
                    "https://oversea.cnki.net/openlink/detail?filename=CNKI202601001",
                ),
            )
            await db.commit()

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
