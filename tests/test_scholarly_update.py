"""Regression tests for index updates."""

from __future__ import annotations

import os
import unittest
from pathlib import Path
from typing import Any, cast

import aiosqlite
import httpx

from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import (
    upsert_articles,
    upsert_issues,
    upsert_journal,
    upsert_meta,
)
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.fetcher import (
    JournalPathError,
    process_cnki_journal,
    process_scholarly_journal,
)
from paper_scanner.index.main import validate_required_source_config
from paper_scanner.index.stats import IndexStatsRecorder
from paper_scanner.index.transforms import (
    build_cnki_article_record,
    build_cnki_issue_record,
    build_cnki_journal_record,
    build_journal_id,
    build_meta_record,
    build_scholarly_article_record,
    build_scholarly_issue_record,
    build_scholarly_journal_record,
)

TEST_CSV_PATH = Path("test.csv")
OPENALEX_KEY_ENV = "OPENALEX_API_KEY_POOL"
SEMANTIC_SCHOLAR_KEY_ENV = "SEMANTIC_SCHOLAR_API_KEY_POOL"


class FakeScholarlyClient:
    """
    Fake scholarly client that records update fetch inputs.
    """

    def __init__(self, works: list[dict[str, Any]]) -> None:
        """
        Initialize the fake client.

        Args:
            works: Crossref works returned by the fake journal request.
        """
        self.works = works
        self.fetch_args: list[dict[str, str | None]] = []
        self.openalex_doi_batches: list[list[str]] = []
        self.semantic_scholar_doi_batches: list[list[str]] = []

    async def fetch_journal_works(
        self,
        issn: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Return Crossref works while recording the date window.

        Args:
            issn: Journal ISSN.
            from_pub_date: Optional lower publication date.
            until_pub_date: Optional upper publication date.

        Returns:
            Fake Crossref works.
        """
        self.fetch_args.append(
            {
                "issn": issn,
                "from_pub_date": from_pub_date,
                "until_pub_date": until_pub_date,
            }
        )
        return self.works

    async def fetch_openalex_by_dois(
        self, dois: list[str], batch_size: int = 100
    ) -> dict[str, dict[str, Any]]:
        """
        Record DOI enrichment requests.

        Args:
            dois: DOI list.
            batch_size: Requested batch size.

        Returns:
            Empty enrichment map.
        """
        self.openalex_doi_batches.append(list(dois))
        return {}

    async def fetch_semantic_scholar_by_dois(
        self, dois: list[str], batch_size: int = 500
    ) -> dict[str, dict[str, Any]]:
        """
        Record Semantic Scholar requests.

        Args:
            dois: DOI list.
            batch_size: Requested batch size.

        Returns:
            Empty OA map.
        """
        self.semantic_scholar_doi_batches.append(list(dois))
        return {}


class OpenAlexFallbackScholarlyClient(FakeScholarlyClient):
    """
    Fake scholarly client that falls back from Crossref to OpenAlex.
    """

    def __init__(self, openalex_works: list[dict[str, Any]]) -> None:
        """
        Initialize the fake client.

        Args:
            openalex_works: OpenAlex works returned by fallback requests.
        """
        super().__init__([])
        self.openalex_works = openalex_works
        self.source_lookup_issns: list[str] = []
        self.source_work_requests: list[dict[str, str | None]] = []

    async def fetch_journal_works(
        self,
        issn: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Raise a Crossref 404 while recording the lookup.

        Args:
            issn: Journal ISSN.
            from_pub_date: Optional lower publication date.
            until_pub_date: Optional upper publication date.

        Raises:
            httpx.HTTPStatusError: Always raised with a 404 response.
        """
        self.fetch_args.append(
            {
                "issn": issn,
                "from_pub_date": from_pub_date,
                "until_pub_date": until_pub_date,
            }
        )
        request = httpx.Request("GET", f"https://api.crossref.org/journals/{issn}")
        response = httpx.Response(404, request=request)
        raise httpx.HTTPStatusError("not found", request=request, response=response)

    async def fetch_openalex_source_by_issns(self, issns: list[str]) -> dict[str, Any]:
        """
        Return a fixed OpenAlex source.

        Args:
            issns: ISSN candidates.

        Returns:
            Fake OpenAlex source payload.
        """
        self.source_lookup_issns = list(issns)
        return {
            "id": "https://openalex.org/S88198767",
            "display_name": "Cognition",
            "issn_l": "0010-0277",
            "issn": ["0010-0277", "1873-7838"],
            "works_count": len(self.openalex_works),
        }

    async def fetch_openalex_works_by_source(
        self,
        source_id: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Return fallback OpenAlex works.

        Args:
            source_id: OpenAlex source id.
            from_pub_date: Optional lower publication date.
            until_pub_date: Optional upper publication date.

        Returns:
            Fake OpenAlex works.
        """
        self.source_work_requests.append(
            {
                "source_id": source_id,
                "from_pub_date": from_pub_date,
                "until_pub_date": until_pub_date,
            }
        )
        return self.openalex_works


class OpenAlexTitleFallbackScholarlyClient(OpenAlexFallbackScholarlyClient):
    """
    Fake scholarly client that resolves OpenAlex source by title.
    """

    def __init__(self, openalex_works: list[dict[str, Any]]) -> None:
        """
        Initialize the fake client.

        Args:
            openalex_works: OpenAlex works returned by fallback requests.
        """
        super().__init__(openalex_works)
        self.source_lookup_titles: list[str] = []

    async def fetch_openalex_source_by_issns(
        self, issns: list[str]
    ) -> dict[str, Any] | None:
        """
        Return no ISSN source match while recording the lookup.

        Args:
            issns: ISSN candidates.

        Returns:
            None.
        """
        self.source_lookup_issns = list(issns)
        return None

    async def fetch_openalex_source_by_title(
        self,
        title: str,
    ) -> dict[str, Any] | None:
        """
        Return a source with ISSNs that differ from the CSV row.

        Args:
            title: Journal title.

        Returns:
            Fake OpenAlex source payload.
        """
        self.source_lookup_titles.append(title)
        return {
            "id": "https://openalex.org/S9551102",
            "display_name": "International journal of central banking",
            "issn_l": "1815-4654",
            "issn": ["1815-4654"],
            "works_count": len(self.openalex_works),
        }


class FailingScholarlyClient(FakeScholarlyClient):
    """
    Fake scholarly client that raises a non-fallback Crossref error.
    """

    def __init__(self, status_code: int) -> None:
        """
        Initialize the fake client.

        Args:
            status_code: HTTP status code raised by Crossref lookup.
        """
        super().__init__([])
        self.status_code = status_code
        self.did_lookup_openalex_source = False

    async def fetch_journal_works(
        self,
        issn: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Raise a fixed Crossref HTTP error.

        Args:
            issn: Journal ISSN.
            from_pub_date: Optional lower publication date.
            until_pub_date: Optional upper publication date.

        Raises:
            httpx.HTTPStatusError: Always raised with the configured status.
        """
        request = httpx.Request("GET", f"https://api.crossref.org/journals/{issn}")
        response = httpx.Response(self.status_code, request=request)
        raise httpx.HTTPStatusError("failed", request=request, response=response)

    async def fetch_openalex_source_by_issns(
        self, issns: list[str]
    ) -> dict[str, Any] | None:
        """
        Record unexpected fallback source lookup.

        Args:
            issns: ISSN candidates.

        Returns:
            None.
        """
        self.did_lookup_openalex_source = True
        return None


class FakeCnkiClient:
    """
    Fake CNKI client that records fetched issue article lists.
    """

    def __init__(self, issues: list[dict[str, Any]]) -> None:
        """
        Initialize the fake client.

        Args:
            issues: CNKI issues returned by the fake year list request.
        """
        self.issues = issues
        self.details = {
            "pykm": "TEST",
            "pcode": "CJFD",
            "time": "token",
            "detail_url": "https://example.test/journal",
            "title": "CNKI Test Journal",
            "issn": "1234-5678",
        }
        self.issue_article_requests: list[str] = []
        self.article_detail_requests: list[str] = []

    async def resolve_journal(self, row: dict[str, str]) -> dict[str, Any]:
        """
        Return fixed CNKI journal details.

        Args:
            row: Source CSV row.

        Returns:
            CNKI-like journal details.
        """
        return self.details

    async def get_year_issues(self, journal: dict[str, Any]) -> list[dict[str, Any]]:
        """
        Return fake issues in upstream order.

        Args:
            journal: CNKI journal details.

        Returns:
            Fake CNKI issue payloads.
        """
        return self.issues

    async def get_issue_articles(
        self,
        journal: dict[str, Any],
        issue: dict[str, Any],
    ) -> list[dict[str, Any]]:
        """
        Return one fake article summary for the issue.

        Args:
            journal: CNKI journal details.
            issue: CNKI issue payload.

        Returns:
            Fake article summaries.
        """
        key = cnki_issue_key(issue)
        self.issue_article_requests.append(key)
        return [build_cnki_summary(issue, f"fetch-{key}")]

    async def get_article_detail(self, article_url: str) -> dict[str, Any]:
        """
        Return fake article details for a summary URL.

        Args:
            article_url: Article URL from the summary.

        Returns:
            Fake CNKI article details.
        """
        self.article_detail_requests.append(article_url)
        platform_id = article_url.rsplit("/", maxsplit=1)[-1]
        return build_cnki_detail(platform_id)


class MissingCnkiClient:
    """
    Fake CNKI client that cannot resolve journal details.
    """

    async def resolve_journal(self, row: dict[str, str]) -> None:
        """
        Return no CNKI journal details.

        Args:
            row: Source CSV row.

        Returns:
            None.
        """
        return None


class IndexConfigTest(unittest.TestCase):
    """
    Verify required index runtime configuration.
    """

    def test_scholarly_rows_require_openalex_key(self) -> None:
        """
        Ensure scholarly indexing fails before anonymous OpenAlex downgrade.
        """
        previous_openalex_key = os.environ.get(OPENALEX_KEY_ENV)
        previous_semantic_scholar_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ.pop(OPENALEX_KEY_ENV, None)
        os.environ[SEMANTIC_SCHOLAR_KEY_ENV] = "s2-key"
        try:
            with self.assertRaisesRegex(
                SystemExit,
                "OpenAlex API key is required",
            ):
                validate_required_source_config([{"source": "scholarly"}])
            validate_required_source_config([{"source": "cnki"}])
        finally:
            _restore_env(OPENALEX_KEY_ENV, previous_openalex_key)
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_semantic_scholar_key)

    def test_scholarly_rows_require_semantic_scholar_key(self) -> None:
        """
        Ensure scholarly indexing fails before silent S2 downgrade.
        """
        previous_openalex_key = os.environ.get(OPENALEX_KEY_ENV)
        previous_semantic_scholar_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ[OPENALEX_KEY_ENV] = "openalex-key"
        os.environ.pop(SEMANTIC_SCHOLAR_KEY_ENV, None)
        try:
            with self.assertRaisesRegex(
                SystemExit,
                "Semantic Scholar API key is required",
            ):
                validate_required_source_config([{"source": "scholarly"}])
            validate_required_source_config([{"source": "cnki"}])
        finally:
            _restore_env(OPENALEX_KEY_ENV, previous_openalex_key)
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_semantic_scholar_key)


class ScholarlyUpdateTest(unittest.IsolatedAsyncioTestCase):
    """
    Verify scholarly update scope stays limited to recent issues.
    """

    async def test_update_enriches_latest_existing_issue_and_new_issues_only(
        self,
    ) -> None:
        """
        Ensure old issue DOI values are excluded from update enrichment.
        """
        row = {
            "source": "scholarly",
            "title": "Test Journal",
            "issn": "1234-5678",
            "id": "1234-5678",
            "area": "testing",
        }
        journal_id = build_journal_id(row)
        assert journal_id is not None

        old_work = build_work("10.1/old", 1, "1")
        latest_work = build_work("10.1/latest", 2, "2")
        new_work = build_work("10.1/new", 3, "3")
        old_issue = build_scholarly_issue_record(journal_id, old_work)
        latest_issue = build_scholarly_issue_record(journal_id, latest_work)
        assert old_issue is not None
        assert latest_issue is not None
        old_article = build_scholarly_article_record(
            old_work, None, None, journal_id, old_issue["issue_id"]
        )
        latest_article = build_scholarly_article_record(
            latest_work, None, None, journal_id, latest_issue["issue_id"]
        )
        assert old_article is not None
        assert latest_article is not None

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                await upsert_journal(
                    db,
                    build_scholarly_journal_record(
                        journal_id, row, [old_work, latest_work]
                    ),
                )
                await upsert_meta(db, build_meta_record(journal_id, TEST_CSV_PATH, row))
                await upsert_issues(db, [old_issue, latest_issue])
                await upsert_articles(db, [old_article, latest_article])
                await db.commit()

                client = FakeScholarlyClient([old_work, latest_work, new_work])
                await process_scholarly_journal(
                    db,
                    cast(Any, client),
                    TEST_CSV_PATH,
                    row,
                    request_workers=4,
                    show_year_progress=False,
                    resume=True,
                    update=True,
                )

                self.assertEqual(
                    client.fetch_args,
                    [
                        {
                            "issn": "1234-5678",
                            "from_pub_date": "2025-01-01",
                            "until_pub_date": None,
                        }
                    ],
                )
                self.assertEqual(
                    client.openalex_doi_batches, [["10.1/latest", "10.1/new"]]
                )
                self.assertEqual(
                    client.semantic_scholar_doi_batches,
                    [["10.1/latest", "10.1/new"]],
                )
                rows = await db.fetchall("SELECT doi FROM articles ORDER BY doi")
                self.assertEqual(
                    [row[0] for row in rows],
                    ["10.1/latest", "10.1/new", "10.1/old"],
                )
            finally:
                await db.close()

    async def test_missing_issn_fails_and_records_path_stats(self) -> None:
        """
        Ensure missing scholarly identifiers are explicit path failures.
        """
        row = {
            "source": "scholarly",
            "title": "No ISSN Journal",
            "issn": "",
            "id": "",
            "area": "testing",
        }
        stats_recorder = IndexStatsRecorder("run-missing-issn", "test.csv")

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                with self.assertRaisesRegex(JournalPathError, "missing ISSN"):
                    await process_scholarly_journal(
                        db,
                        cast(Any, FakeScholarlyClient([])),
                        TEST_CSV_PATH,
                        row,
                        request_workers=4,
                        show_year_progress=False,
                        resume=True,
                        update=False,
                        stats_recorder=stats_recorder,
                    )
            finally:
                await db.close()

        path_stats = next(iter(stats_recorder.stats.path_stats.values()))
        self.assertEqual(path_stats.status, "failed")
        self.assertEqual(path_stats.error_type, "JournalPathError")

    async def test_crossref_404_uses_openalex_fallback_and_updates_meta(
        self,
    ) -> None:
        """
        Ensure OpenAlex fallback writes articles and resolved journal meta.
        """
        row = {
            "source": "scholarly",
            "title": "Cognition",
            "issn": "1873-7838",
            "id": "1873-7838",
            "area": "testing",
            "all_issns": "1873-7838",
        }
        journal_id = build_journal_id(row)
        assert journal_id is not None
        stats_recorder = IndexStatsRecorder("run-openalex-fallback", "test.csv")
        client = OpenAlexFallbackScholarlyClient(
            [
                {
                    "id": "https://openalex.org/W1",
                    "doi": "https://doi.org/10.1/fallback",
                    "title": "Fallback Article",
                    "publication_date": "2025-02-03",
                    "biblio": {
                        "volume": "12",
                        "issue": "1",
                        "first_page": "1",
                        "last_page": "9",
                    },
                    "authorships": [{"author": {"display_name": "Fallback Author"}}],
                    "open_access": {"is_oa": True},
                    "best_oa_location": {
                        "pdf_url": "https://openalex.test/fallback.pdf",
                        "landing_page_url": "https://openalex.test/fallback",
                    },
                }
            ]
        )

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                await process_scholarly_journal(
                    db,
                    cast(Any, client),
                    TEST_CSV_PATH,
                    row,
                    request_workers=4,
                    show_year_progress=False,
                    resume=False,
                    update=False,
                    stats_recorder=stats_recorder,
                )

                journal = await db.fetchone(
                    """
                    SELECT platform_journal_id, title, issn, eissn
                    FROM journals
                    WHERE journal_id = ?
                    """,
                    (journal_id,),
                )
                self.assertEqual(
                    tuple(journal),
                    ("S88198767", "Cognition", "0010-0277", "1873-7838"),
                )
                meta = await db.fetchone(
                    """
                    SELECT csv_issn, resolved_source, resolved_source_id,
                           resolved_title, resolved_issn, resolved_eissn
                    FROM journal_meta
                    WHERE journal_id = ?
                    """,
                    (journal_id,),
                )
                self.assertEqual(
                    tuple(meta),
                    (
                        "1873-7838",
                        "openalex",
                        "S88198767",
                        "Cognition",
                        "0010-0277",
                        "1873-7838",
                    ),
                )
                article = await db.fetchone(
                    """
                    SELECT title, doi, authors, open_access, full_text_file,
                           content_location
                    FROM articles
                    """
                )
                self.assertEqual(
                    tuple(article),
                    (
                        "Fallback Article",
                        "10.1/fallback",
                        "Fallback Author",
                        1,
                        "https://openalex.test/fallback.pdf",
                        "https://openalex.test/fallback",
                    ),
                )
            finally:
                await db.close()

        self.assertEqual(client.source_lookup_issns, ["1873-7838"])
        self.assertEqual(
            client.source_work_requests,
            [
                {
                    "source_id": "https://openalex.org/S88198767",
                    "from_pub_date": None,
                    "until_pub_date": None,
                }
            ],
        )
        self.assertEqual(client.openalex_doi_batches, [])
        self.assertEqual(client.semantic_scholar_doi_batches, [["10.1/fallback"]])
        path_stats = next(iter(stats_recorder.stats.path_stats.values()))
        self.assertEqual(path_stats.status, "succeeded")
        self.assertEqual(path_stats.works_count, 1)

    async def test_crossref_404_uses_openalex_title_fallback_and_updates_meta(
        self,
    ) -> None:
        """
        Ensure title fallback handles OpenAlex sources with mismatched ISSNs.
        """
        row = {
            "source": "scholarly",
            "title": "International Journal of Central Banking",
            "issn": "1815-7556",
            "id": "1815-7556",
            "area": "testing",
            "all_issns": "1815-7556",
        }
        journal_id = build_journal_id(row)
        assert journal_id is not None
        client = OpenAlexTitleFallbackScholarlyClient(
            [
                {
                    "id": "https://openalex.org/W2",
                    "doi": "https://doi.org/10.2/title-fallback",
                    "title": "Central Banking Article",
                    "publication_date": "2025-03-04",
                    "biblio": {
                        "volume": "21",
                        "issue": "2",
                        "first_page": "10",
                        "last_page": "20",
                    },
                    "authorships": [{"author": {"display_name": "Bank Author"}}],
                    "open_access": {"is_oa": False},
                    "best_oa_location": None,
                }
            ]
        )

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                await process_scholarly_journal(
                    db,
                    cast(Any, client),
                    TEST_CSV_PATH,
                    row,
                    request_workers=4,
                    show_year_progress=False,
                    resume=False,
                    update=False,
                )

                journal = await db.fetchone(
                    """
                    SELECT platform_journal_id, title, issn, eissn
                    FROM journals
                    WHERE journal_id = ?
                    """,
                    (journal_id,),
                )
                self.assertEqual(
                    tuple(journal),
                    (
                        "S9551102",
                        "International journal of central banking",
                        "1815-4654",
                        None,
                    ),
                )
                meta = await db.fetchone(
                    """
                    SELECT csv_issn, resolved_source, resolved_source_id,
                           resolved_title, resolved_issn, resolved_eissn
                    FROM journal_meta
                    WHERE journal_id = ?
                    """,
                    (journal_id,),
                )
                self.assertEqual(
                    tuple(meta),
                    (
                        "1815-7556",
                        "openalex",
                        "S9551102",
                        "International journal of central banking",
                        "1815-4654",
                        None,
                    ),
                )
            finally:
                await db.close()

        self.assertEqual(client.source_lookup_issns, ["1815-7556"])
        self.assertEqual(
            client.source_lookup_titles,
            ["International Journal of Central Banking"],
        )
        self.assertEqual(
            client.source_work_requests,
            [
                {
                    "source_id": "https://openalex.org/S9551102",
                    "from_pub_date": None,
                    "until_pub_date": None,
                }
            ],
        )
        self.assertEqual(client.openalex_doi_batches, [])
        self.assertEqual(
            client.semantic_scholar_doi_batches,
            [["10.2/title-fallback"]],
        )

    async def test_crossref_non_404_does_not_use_openalex_fallback(self) -> None:
        """
        Ensure non-404 Crossref failures stay fail-loud.
        """
        row = {
            "source": "scholarly",
            "title": "Rate Limited Journal",
            "issn": "1234-5678",
            "id": "1234-5678",
            "area": "testing",
        }
        client = FailingScholarlyClient(500)

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                with self.assertRaises(httpx.HTTPStatusError):
                    await process_scholarly_journal(
                        db,
                        cast(Any, client),
                        TEST_CSV_PATH,
                        row,
                        request_workers=4,
                        show_year_progress=False,
                        resume=False,
                        update=False,
                    )
            finally:
                await db.close()

        self.assertFalse(client.did_lookup_openalex_source)


class CnkiUpdateTest(unittest.IsolatedAsyncioTestCase):
    """
    Verify CNKI update scope stays limited to recent issues.
    """

    async def test_update_fetches_latest_existing_issue_and_newer_issues_only(
        self,
    ) -> None:
        """
        Ensure older CNKI issue article lists are excluded during update.
        """
        row = {
            "source": "cnki",
            "title": "CNKI Test Journal",
            "issn": "1234-5678",
            "id": "CNKI Test Journal",
            "area": "testing",
        }
        journal_id = build_journal_id(row)
        assert journal_id is not None

        old_issue = build_cnki_issue(2024, "01")
        latest_issue = build_cnki_issue(2025, "01")
        new_issue = build_cnki_issue(2026, "01")
        client = FakeCnkiClient([new_issue, latest_issue, old_issue])
        journal_code = str(client.details["pykm"])
        old_issue_record = build_cnki_issue_record(journal_id, journal_code, old_issue)
        latest_issue_record = build_cnki_issue_record(
            journal_id, journal_code, latest_issue
        )
        assert old_issue_record is not None
        assert latest_issue_record is not None
        old_article = build_cnki_article_record(
            build_cnki_detail("seed-old"),
            build_cnki_summary(old_issue, "seed-old"),
            journal_id,
            old_issue_record["issue_id"],
        )
        latest_article = build_cnki_article_record(
            build_cnki_detail("seed-latest"),
            build_cnki_summary(latest_issue, "seed-latest"),
            journal_id,
            latest_issue_record["issue_id"],
        )
        assert old_article is not None
        assert latest_article is not None

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                await upsert_journal(
                    db,
                    build_cnki_journal_record(journal_id, row, client.details),
                )
                await upsert_meta(db, build_meta_record(journal_id, TEST_CSV_PATH, row))
                await upsert_issues(db, [old_issue_record, latest_issue_record])
                await upsert_articles(db, [old_article, latest_article])
                await db.commit()

                await process_cnki_journal(
                    db,
                    cast(Any, client),
                    TEST_CSV_PATH,
                    row,
                    issue_batch_size=10,
                    request_workers=4,
                    show_year_progress=False,
                    resume=True,
                    update=True,
                )

                self.assertEqual(
                    client.issue_article_requests,
                    ["2026:01", "2025:01"],
                )
            finally:
                await db.close()

    async def test_missing_cnki_details_fails_and_records_path_stats(self) -> None:
        """
        Ensure unresolved CNKI journals are explicit path failures.
        """
        row = {
            "source": "cnki",
            "title": "Missing CNKI Journal",
            "issn": "1234-5678",
            "id": "Missing CNKI Journal",
            "area": "testing",
        }
        stats_recorder = IndexStatsRecorder("run-missing-cnki", "test.csv")

        async with aiosqlite.connect(":memory:") as raw_db:
            await init_db(raw_db)
            db = LocalDatabaseClient(raw_db)
            await db.start()
            try:
                with self.assertRaisesRegex(JournalPathError, "No CNKI details"):
                    await process_cnki_journal(
                        db,
                        cast(Any, MissingCnkiClient()),
                        TEST_CSV_PATH,
                        row,
                        issue_batch_size=10,
                        request_workers=4,
                        show_year_progress=False,
                        resume=True,
                        update=False,
                        stats_recorder=stats_recorder,
                    )
            finally:
                await db.close()

        path_stats = next(iter(stats_recorder.stats.path_stats.values()))
        self.assertEqual(path_stats.status, "failed")
        self.assertEqual(path_stats.error_type, "JournalPathError")


def build_work(doi: str, month: int, issue: str) -> dict[str, Any]:
    """
    Build a minimal Crossref work payload.

    Args:
        doi: DOI value.
        month: Publication month.
        issue: Issue number.

    Returns:
        Crossref-like work payload.
    """
    return {
        "DOI": doi,
        "ISSN": ["1234-5678"],
        "URL": f"https://doi.org/{doi}",
        "title": [f"Article {doi}"],
        "author": [{"given": "Test", "family": "Author"}],
        "published": {"date-parts": [[2025, month, 1]]},
        "volume": "1",
        "issue": issue,
    }


def build_cnki_issue(year: int, number: str) -> dict[str, Any]:
    """
    Build a minimal CNKI issue payload.

    Args:
        year: Publication year.
        number: Issue number.

    Returns:
        CNKI-like issue payload.
    """
    return {
        "year": year,
        "number": number,
        "title": f"{year}年第{number}期",
        "year_issue": f"{year}{number}",
    }


def cnki_issue_key(issue: dict[str, Any]) -> str:
    """
    Build a stable display key for a fake CNKI issue.

    Args:
        issue: CNKI issue payload.

    Returns:
        Issue key used by test assertions.
    """
    return f"{issue['year']}:{issue['number']}"


def _restore_env(name: str, value: str | None) -> None:
    """
    Restore one environment variable.

    Args:
        name: Environment variable name.
        value: Previous value, or None when absent.

    Returns:
        None.
    """
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def build_cnki_summary(
    issue: dict[str, Any],
    platform_id: str,
) -> dict[str, Any]:
    """
    Build a minimal CNKI article summary payload.

    Args:
        issue: CNKI issue payload.
        platform_id: Fake CNKI article identifier.

    Returns:
        CNKI-like article summary payload.
    """
    return {
        "article_url": f"https://example.test/article/{platform_id}",
        "platform_id": platform_id,
        "title": f"CNKI article {platform_id}",
        "authors": "Test Author",
        "pages": "1-2",
        "section": "Articles",
        "is_free": 0,
        "date": f"{int(issue['year']):04d}-01-01",
    }


def build_cnki_detail(platform_id: str) -> dict[str, Any]:
    """
    Build a minimal CNKI article detail payload.

    Args:
        platform_id: Fake CNKI article identifier.

    Returns:
        CNKI-like article detail payload.
    """
    return {
        "article_url": f"https://example.test/article/{platform_id}",
        "platform_id": platform_id,
        "title": f"CNKI article {platform_id}",
        "authors": "Test Author",
        "abstract": "Test abstract.",
        "doi": None,
        "online_release_date": "2025-01-01",
        "pages": "1-2",
        "html_read_url": None,
        "permalink": f"https://example.test/article/{platform_id}",
        "content_location": f"https://example.test/article/{platform_id}",
    }
