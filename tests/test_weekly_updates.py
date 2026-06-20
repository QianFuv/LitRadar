"""Tests for weekly update query helpers."""

from __future__ import annotations

import sqlite3
import unittest
from datetime import UTC, datetime
from pathlib import Path
from tempfile import TemporaryDirectory
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

    def test_parse_weekly_manifest_uses_notifiable_ids_only(self) -> None:
        """
        Ensure historical backfill IDs are not shown as weekly updates.

        Returns:
            None.
        """
        manifest = weekly.parse_weekly_manifest(
            {
                "db_name": "alpha.sqlite",
                "generated_at": "2026-01-08T00:00:00Z",
                "notifiable_article_ids": [3, 2, 3, 1],
                "backfill_article_ids": [99, 100],
            }
        )

        self.assertIsNotNone(manifest)
        if manifest is None:
            return
        self.assertEqual(manifest.article_ids, [3, 2, 1])

    async def test_get_weekly_updates_aggregates_all_databases(self) -> None:
        """
        Ensure weekly aggregation returns every database with notifiable articles.

        Returns:
            None.
        """
        manifests = [
            weekly.WeeklyManifestSummary(
                db_name="alpha.sqlite",
                run_id="alpha-run",
                generated_at=datetime(2026, 1, 8, tzinfo=UTC),
                article_ids=[1],
            ),
            weekly.WeeklyManifestSummary(
                db_name="beta.sqlite",
                run_id="beta-run",
                generated_at=datetime(2026, 1, 7, tzinfo=UTC),
                article_ids=[2],
            ),
        ]
        fetched_batches: list[list[int]] = []

        async def fake_fetch_articles_by_ids(
            db: aiosqlite.Connection,
            article_ids: list[int],
        ) -> list[weekly.WeeklyArticleRecord]:
            """
            Return fake rows for the requested database batch.

            Args:
                db: Unused database connection.
                article_ids: Requested article identifiers.

            Returns:
                Weekly article records matching the requested identifiers.
            """
            del db
            fetched_batches.append(article_ids)
            return [
                weekly.WeeklyArticleRecord(**build_weekly_article_row(item))
                for item in article_ids
            ]

        with TemporaryDirectory() as tmp:
            index_dir = Path(tmp)
            (index_dir / "alpha.sqlite").touch()
            (index_dir / "beta.sqlite").touch()
            with (
                patch.object(weekly, "INDEX_DIR", index_dir),
                patch.object(
                    weekly, "load_weekly_manifest_payloads", return_value=manifests
                ),
                patch.object(
                    weekly, "fetch_articles_by_ids", fake_fetch_articles_by_ids
                ),
            ):
                response = await weekly.get_weekly_updates()

        self.assertEqual(
            [item.db_name for item in response.databases],
            ["alpha.sqlite", "beta.sqlite"],
        )
        self.assertEqual(fetched_batches, [[1], [2]])

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
