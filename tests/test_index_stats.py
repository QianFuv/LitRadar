"""Tests for index run API and path statistics."""

from __future__ import annotations

import json
import unittest

import aiosqlite

from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import persist_index_run_stats
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.stats import IndexStatsRecorder


class IndexStatsTest(unittest.IsolatedAsyncioTestCase):
    """Verify index statistics aggregation and persistence."""

    async def test_stats_aggregate_attempts_paths_and_secret_free_payloads(
        self,
    ) -> None:
        """
        Ensure API and path counters merge without leaking credentials.
        """
        recorder = IndexStatsRecorder(
            run_id="run-1",
            csv_file="journals.csv",
            started_at="2026-06-14T00:00:00",
        )
        path_key = recorder.record_path_started(
            "scholarly", "journal", 1, "Test Journal"
        )
        api_key = recorder.record_api_call(
            "openalex",
            "works",
            "GET",
            "https://api.openalex.org/works?api_key=SECRET&mailto=a@example.test",
        )
        recorder.record_api_attempt(
            api_key,
            status_code=429,
            did_succeed=False,
            elapsed_ms=10.4,
            error=(
                "https://api.openalex.org/works?"
                "api_key=SECRET&token=TOKEN&mailto=a@example.test"
            ),
            did_retry=True,
        )
        recorder.record_api_attempt(
            api_key,
            status_code=200,
            did_succeed=True,
            elapsed_ms=20.0,
        )
        recorder.record_path_counts(
            path_key,
            works_count=3,
            issues_count=2,
            articles_written_count=2,
        )
        recorder.record_path_finished("succeeded", path_key)

        other_recorder = IndexStatsRecorder("run-1", "journals.csv")
        other_recorder.set_current_path("scholarly", "journal", 1, "Test Journal")
        other_key = other_recorder.record_api_call(
            "openalex",
            "works",
            "GET",
            "https://api.openalex.org/works?api_key=SECOND_SECRET",
        )
        other_recorder.record_api_attempt(
            other_key,
            status_code=None,
            did_succeed=False,
            elapsed_ms=5.0,
            error=RuntimeError("proxy=http://proxy.test?token=PROXY_TOKEN"),
        )
        recorder.merge(other_recorder.to_dict())

        api_stats = next(iter(recorder.stats.api_stats.values()))
        path_stats = next(iter(recorder.stats.path_stats.values()))
        payload = json.dumps(recorder.to_dict())

        self.assertEqual(api_stats.logical_calls, 2)
        self.assertEqual(api_stats.attempts, 3)
        self.assertEqual(api_stats.successes, 1)
        self.assertEqual(api_stats.failures, 2)
        self.assertEqual(api_stats.retry_count, 1)
        self.assertEqual(api_stats.status_codes[429], 1)
        self.assertEqual(api_stats.rate_limit_failures, 1)
        self.assertEqual(api_stats.transport_errors, 1)
        self.assertEqual(api_stats.key.url_path, "/works")
        self.assertEqual(path_stats.works_count, 3)
        self.assertEqual(path_stats.issues_count, 2)
        self.assertEqual(path_stats.articles_written_count, 2)
        self.assertNotIn("SECRET", payload)
        self.assertNotIn("TOKEN", payload)
        self.assertNotIn("PROXY_TOKEN", payload)
        self.assertNotIn("mailto=a@example.test", payload)

    async def test_persist_index_run_stats_writes_queryable_rows(self) -> None:
        """
        Ensure index run statistics are persisted to SQLite tables.
        """
        recorder = IndexStatsRecorder(
            run_id="run-2",
            csv_file="journals.csv",
            started_at="2026-06-14T00:00:00",
        )
        path_key = recorder.record_path_started("cnki", "journal", 2, "CNKI Journal")
        recorder.record_path_counts(
            path_key,
            issues_count=4,
            article_summaries_count=8,
            article_details_count=8,
            articles_written_count=7,
            articles_deleted_no_authors_count=1,
        )
        recorder.record_path_finished("failed", path_key, RuntimeError("CNKI failed"))
        api_key = recorder.record_api_call(
            "cnki",
            "issue_articles",
            "POST",
            "https://oversea.cnki.net/knavi/journals/TEST/papers?token=SECRET",
        )
        recorder.record_api_attempt(
            api_key,
            status_code=503,
            did_succeed=False,
            elapsed_ms=15.0,
            error="temporary outage",
            did_retry=True,
        )
        recorder.stats.finish("failed", "one journal failed", "2026-06-14T00:01:00")

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                await persist_index_run_stats(db, recorder.stats)
                run_row = await db.fetchone(
                    """
                    SELECT status, total_journals, failed_journals, error_summary
                    FROM index_runs
                    WHERE run_id = ?
                    """,
                    ("run-2",),
                )
                path_row = await db.fetchone(
                    """
                    SELECT status, issues_count, article_details_count,
                        articles_deleted_no_authors_count, error_type
                    FROM index_path_stats
                    WHERE run_id = ?
                    """,
                    ("run-2",),
                )
                api_row = await db.fetchone(
                    """
                    SELECT service, endpoint, url_path, attempts, failures,
                        retry_count, status_codes_json
                    FROM index_api_call_stats
                    WHERE run_id = ?
                    """,
                    ("run-2",),
                )
            finally:
                await db.close()

        self.assertEqual(run_row, ("failed", 1, 1, "one journal failed"))
        self.assertEqual(path_row, ("failed", 4, 8, 1, "RuntimeError"))
        assert api_row is not None
        self.assertEqual(
            api_row[:6],
            ("cnki", "issue_articles", "/knavi/journals/TEST/papers", 1, 1, 1),
        )
        self.assertEqual(json.loads(str(api_row[6])), {"503": 1})
