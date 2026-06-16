"""Tests for weekly update query helpers."""

from __future__ import annotations

import sqlite3
import unittest
from typing import Any, cast
from unittest.mock import patch

import aiosqlite

import paper_scanner.api.queries.weekly as weekly


def build_weekly_article_row(article_id: int) -> dict[str, Any]:
    """
    Build one weekly article row payload.

    Args:
        article_id: Article identifier.

    Returns:
        Row-like mapping accepted by the weekly response model.
    """
    return {
        "article_id": article_id,
        "journal_id": 1,
        "issue_id": None,
        "title": f"Article {article_id}",
        "date": "2026-01-01",
        "authors": "Test Author",
        "abstract": "Test abstract.",
        "doi": f"10.1000/{article_id}",
        "platform_id": str(article_id),
        "permalink": f"https://example.test/{article_id}",
        "full_text_file": None,
        "open_access": 0,
        "in_press": 0,
        "journal_title": "Test Journal",
        "volume": None,
        "number": None,
    }


class WeeklyUpdatesQueryTest(unittest.IsolatedAsyncioTestCase):
    """Verify weekly update query behavior."""

    async def test_fetch_articles_by_ids_batches_large_manifest_lists(self) -> None:
        """
        Ensure large weekly manifests stay under SQLite variable limits.

        Returns:
            None.
        """
        article_ids = list(range(1, weekly.SQLITE_QUERY_BATCH_SIZE * 2 + 7))
        rows_by_id = {
            article_id: build_weekly_article_row(article_id)
            for article_id in article_ids
        }
        batch_sizes: list[int] = []

        async def fake_fetch_all(
            db: aiosqlite.Connection,
            query: str,
            params: list[int],
        ) -> list[dict[str, Any]]:
            """
            Return fake rows while enforcing a SQLite variable ceiling.

            Args:
                db: Unused database connection.
                query: SQL query text.
                params: Query parameters.

            Returns:
                Fake article rows matching the parameter order.
            """
            del db, query
            batch_sizes.append(len(params))
            if len(params) > weekly.SQLITE_QUERY_BATCH_SIZE:
                raise sqlite3.OperationalError("too many SQL variables")
            return [rows_by_id[article_id] for article_id in params]

        with patch("paper_scanner.api.queries.weekly.fetch_all", fake_fetch_all):
            records = await weekly.fetch_articles_by_ids(
                cast(aiosqlite.Connection, object()),
                article_ids,
            )

        self.assertEqual(
            batch_sizes,
            [
                weekly.SQLITE_QUERY_BATCH_SIZE,
                weekly.SQLITE_QUERY_BATCH_SIZE,
                6,
            ],
        )
        self.assertEqual(
            [int(record.article_id) for record in records],
            article_ids,
        )


if __name__ == "__main__":
    unittest.main()
