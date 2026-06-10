"""Regression tests for index updates."""

from __future__ import annotations

import unittest
from pathlib import Path
from typing import Any, cast

import aiosqlite

from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import (
    upsert_articles,
    upsert_issues,
    upsert_journal,
    upsert_meta,
)
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.fetcher import process_cnki_journal, process_scholarly_journal
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
        self.unpaywall_doi_batches: list[list[str]] = []

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

    async def fetch_unpaywall_by_dois(
        self, dois: list[str], request_workers: int = 4
    ) -> dict[str, dict[str, Any]]:
        """
        Record Unpaywall requests.

        Args:
            dois: DOI list.
            request_workers: Requested concurrency.

        Returns:
            Empty OA map.
        """
        self.unpaywall_doi_batches.append(list(dois))
        return {}


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
                    client.unpaywall_doi_batches, [["10.1/latest", "10.1/new"]]
                )
                rows = await db.fetchall("SELECT doi FROM articles ORDER BY doi")
                self.assertEqual(
                    [row[0] for row in rows],
                    ["10.1/latest", "10.1/new", "10.1/old"],
                )
            finally:
                await db.close()


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
