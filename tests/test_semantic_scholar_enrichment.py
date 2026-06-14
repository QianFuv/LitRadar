"""Tests for Semantic Scholar scholarly enrichment."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from typing import Any

import aiosqlite
import httpx

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.models import ArticleRecord
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.stats import IndexStatsRecorder
from paper_scanner.index.transforms import build_scholarly_article_record
from paper_scanner.sources.cnki import CnkiClient
from paper_scanner.sources.scholarly.client import (
    ScholarlyClient,
    ScholarlyConfigurationError,
)
from paper_scanner.sources.scholarly.limits import (
    OPENALEX_SOURCE,
    ScholarlyRequestThrottles,
)

SEMANTIC_SCHOLAR_KEY_ENV = "SEMANTIC_SCHOLAR_API_KEY_POOL"
UNPAYWALL_EMAIL_ENV = "UNPAYWALL_EMAIL_POOL"
REMOVED_ARTICLE_FIELDS = {
    "sync_id",
    "ill_url",
    "link_resolver_openurl_link",
    "email_article_request_link",
    "retraction_date",
    "retraction_related_urls",
    "unpaywall_data_suppressed",
    "expression_of_concern_doi",
    "noodletools_export_link",
    "avoid_unpaywall_publisher_links",
    "nomad_fallback_url",
}
RETAINED_ARTICLE_FIELDS = {"suppressed", "within_library_holdings"}


class ArticleSchemaCleanupTest(unittest.IsolatedAsyncioTestCase):
    """
    Verify article compatibility fields are removed from new surfaces.
    """

    async def test_new_articles_schema_omits_removed_fields(self) -> None:
        """
        Ensure new article tables exclude unused compatibility columns.
        """
        async with aiosqlite.connect(":memory:") as db:
            await init_db(db)
            cursor = await db.execute("PRAGMA table_info(articles)")
            rows = await cursor.fetchall()
            await cursor.close()

        columns = {str(row[1]) for row in rows}
        self.assertFalse(REMOVED_ARTICLE_FIELDS & columns)
        self.assertTrue(columns >= RETAINED_ARTICLE_FIELDS)

    def test_article_record_omits_removed_fields(self) -> None:
        """
        Ensure API article records expose only retained status fields.
        """
        field_names = set(ArticleRecord.model_fields)
        self.assertFalse(REMOVED_ARTICLE_FIELDS & field_names)
        self.assertTrue(field_names >= RETAINED_ARTICLE_FIELDS)


class SemanticScholarRuntimeConfigTest(unittest.TestCase):
    """
    Verify Semantic Scholar runtime setting integration.
    """

    def setUp(self) -> None:
        """
        Redirect auth database and clear managed environment values.

        Returns:
            None.
        """
        self.previous_auth_db_path = auth_db.AUTH_DB_PATH
        self.temp_dir = tempfile.TemporaryDirectory()
        auth_db.AUTH_DB_PATH = Path(self.temp_dir.name) / "auth.sqlite"
        self.previous_env_values = {
            SEMANTIC_SCHOLAR_KEY_ENV: os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV),
            UNPAYWALL_EMAIL_ENV: os.environ.get(UNPAYWALL_EMAIL_ENV),
        }
        os.environ.pop(SEMANTIC_SCHOLAR_KEY_ENV, None)
        os.environ.pop(UNPAYWALL_EMAIL_ENV, None)

    def tearDown(self) -> None:
        """
        Restore auth database path and environment values.

        Returns:
            None.
        """
        auth_db.AUTH_DB_PATH = self.previous_auth_db_path
        for name, value in self.previous_env_values.items():
            _restore_env(name, value)
        self.temp_dir.cleanup()

    def test_runtime_settings_include_s2_key_and_not_unpaywall(self) -> None:
        """
        Ensure runtime setting listing exposes S2 key metadata.
        """
        os.environ[SEMANTIC_SCHOLAR_KEY_ENV] = "s2-key"
        auth_db.init_auth_db()

        settings = {item["field"]: item for item in auth_db.list_runtime_settings()}

        self.assertIn("semantic_scholar_api_key_pool", settings)
        self.assertNotIn("unpaywall_email_pool", settings)
        s2_setting = settings["semantic_scholar_api_key_pool"]
        self.assertEqual(s2_setting["key"], SEMANTIC_SCHOLAR_KEY_ENV)
        self.assertEqual(s2_setting["value"], "s2-key")
        self.assertTrue(s2_setting["is_secret"])

    def test_runtime_settings_accept_s2_and_reject_unpaywall(self) -> None:
        """
        Ensure setting updates use the current managed definition set.
        """
        auth_db.init_auth_db()

        settings = {
            item["field"]: item
            for item in auth_db.upsert_runtime_settings(
                {"semantic_scholar_api_key_pool": "s2-key"}
            )
        }

        self.assertEqual(settings["semantic_scholar_api_key_pool"]["value"], "s2-key")
        with self.assertRaisesRegex(ValueError, "Unknown runtime setting"):
            auth_db.upsert_runtime_settings({"unpaywall_email_pool": "old-email"})


class SemanticScholarClientTest(unittest.IsolatedAsyncioTestCase):
    """
    Verify Semantic Scholar batch request behavior.
    """

    async def test_fetch_semantic_scholar_by_dois_posts_batch(self) -> None:
        """
        Ensure S2 DOI enrichment uses batch POST with an API key header.
        """
        previous_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ[SEMANTIC_SCHOLAR_KEY_ENV] = "key-one"
        requests: list[httpx.Request] = []

        def handler(request: httpx.Request) -> httpx.Response:
            """
            Return a fake S2 batch response.

            Args:
                request: Captured HTTP request.

            Returns:
                Fake HTTP response.
            """
            requests.append(request)
            body = json.loads(request.content.decode("utf-8"))
            self.assertEqual(request.method, "POST")
            self.assertEqual(request.url.path, "/graph/v1/paper/batch")
            self.assertEqual(request.headers.get("x-api-key"), "key-one")
            self.assertEqual(
                request.url.params.get("fields"),
                "externalIds,url,isOpenAccess,openAccessPdf",
            )
            self.assertEqual(set(body["ids"]), {"DOI:10.1/a", "DOI:10.1/b"})
            return httpx.Response(
                200,
                json=[
                    {
                        "externalIds": {"DOI": "10.1/A"},
                        "isOpenAccess": True,
                        "openAccessPdf": {"url": "https://s2.test/a.pdf"},
                    },
                    None,
                    {
                        "externalIds": {"DOI": "10.1/B"},
                        "isOpenAccess": False,
                        "openAccessPdf": None,
                    },
                ],
            )

        client = await _mock_scholarly_client(handler)
        try:
            result = await client.fetch_semantic_scholar_by_dois(
                ["https://doi.org/10.1/A", "10.1/B", "10.1/A"]
            )
        finally:
            await client.aclose()
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_key)

        self.assertEqual(len(requests), 1)
        self.assertEqual(
            result["10.1/a"]["openAccessPdf"]["url"], "https://s2.test/a.pdf"
        )
        self.assertFalse(result["10.1/b"]["isOpenAccess"])

    async def test_fetch_semantic_scholar_by_dois_caps_batch_size(self) -> None:
        """
        Ensure S2 batch requests never exceed the official 500 ID limit.
        """
        previous_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ[SEMANTIC_SCHOLAR_KEY_ENV] = "key-one"
        batch_sizes: list[int] = []

        def handler(request: httpx.Request) -> httpx.Response:
            """
            Capture one fake S2 batch request.

            Args:
                request: Captured HTTP request.

            Returns:
                Empty fake HTTP response.
            """
            body = json.loads(request.content.decode("utf-8"))
            batch_sizes.append(len(body["ids"]))
            return httpx.Response(200, json=[])

        client = await _mock_scholarly_client(handler)
        try:
            await client.fetch_semantic_scholar_by_dois(
                [f"10.1/{index}" for index in range(501)],
                batch_size=999,
            )
        finally:
            await client.aclose()
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_key)

        self.assertEqual(batch_sizes, [500, 1])

    async def test_fetch_semantic_scholar_by_dois_fails_without_key(self) -> None:
        """
        Ensure missing S2 configuration fails required enrichment.
        """
        previous_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ.pop(SEMANTIC_SCHOLAR_KEY_ENV, None)
        client = ScholarlyClient(request_throttles=ScholarlyRequestThrottles([]))
        try:
            with self.assertRaisesRegex(
                ScholarlyConfigurationError,
                "Semantic Scholar API key is required",
            ):
                await client.fetch_semantic_scholar_by_dois(["10.1/a"])
        finally:
            await client.aclose()
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_key)

    async def test_fetch_semantic_scholar_by_dois_raises_http_error(self) -> None:
        """
        Ensure S2 request failures are recorded and not swallowed.
        """
        previous_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ[SEMANTIC_SCHOLAR_KEY_ENV] = "key-one"
        stats_recorder = IndexStatsRecorder("run-1", "journals.csv")
        stats_recorder.set_current_path("scholarly", "journal", 1, "Test Journal")

        def handler(request: httpx.Request) -> httpx.Response:
            """
            Return a failing fake S2 response.

            Args:
                request: Captured HTTP request.

            Returns:
                Fake HTTP response.
            """
            return httpx.Response(400, json={"error": "bad request"})

        client = await _mock_scholarly_client(handler, stats_recorder)
        try:
            with self.assertRaises(httpx.HTTPStatusError):
                await client.fetch_semantic_scholar_by_dois(["10.1/a"])
        finally:
            await client.aclose()
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_key)

        api_stats = next(iter(stats_recorder.stats.api_stats.values()))
        self.assertEqual(api_stats.key.service, "semantic_scholar")
        self.assertEqual(api_stats.key.endpoint, "paper_batch")
        self.assertEqual(api_stats.logical_calls, 1)
        self.assertEqual(api_stats.attempts, 1)
        self.assertEqual(api_stats.failures, 1)
        self.assertEqual(api_stats.status_codes[400], 1)

    async def test_openalex_retry_exhaustion_records_failure(self) -> None:
        """
        Ensure retry-exhausted OpenAlex requests record rate-limit failures.
        """
        stats_recorder = IndexStatsRecorder("run-2", "journals.csv")
        stats_recorder.set_current_path("scholarly", "journal", 1, "Test Journal")

        def handler(request: httpx.Request) -> httpx.Response:
            """
            Return a retryable fake OpenAlex response.

            Args:
                request: Captured HTTP request.

            Returns:
                Fake HTTP response.
            """
            return httpx.Response(429, json={"error": "rate limited"})

        client = await _mock_scholarly_client(handler, stats_recorder)
        try:
            with self.assertRaises(httpx.HTTPStatusError):
                await client._get_with_retries(
                    "https://api.openalex.org/works?api_key=SECRET",
                    max_retries=0,
                    source=OPENALEX_SOURCE,
                    endpoint="works",
                )
        finally:
            await client.aclose()

        api_stats = next(iter(stats_recorder.stats.api_stats.values()))
        self.assertEqual(api_stats.key.service, "openalex")
        self.assertEqual(api_stats.key.endpoint, "works")
        self.assertEqual(api_stats.key.url_path, "/works")
        self.assertEqual(api_stats.logical_calls, 1)
        self.assertEqual(api_stats.attempts, 1)
        self.assertEqual(api_stats.failures, 1)
        self.assertEqual(api_stats.rate_limit_failures, 1)
        self.assertEqual(api_stats.status_codes[429], 1)


class CnkiClientStatsTest(unittest.IsolatedAsyncioTestCase):
    """
    Verify CNKI request statistics.
    """

    async def test_get_year_issues_records_endpoint_stats(self) -> None:
        """
        Ensure CNKI endpoint calls record success statistics.
        """
        stats_recorder = IndexStatsRecorder("run-3", "cnki.csv")
        stats_recorder.set_current_path("cnki", "journal", 2, "CNKI Journal")

        def handler(request: httpx.Request) -> httpx.Response:
            """
            Return a fake CNKI year issue tree.

            Args:
                request: Captured HTTP request.

            Returns:
                Fake CNKI response.
            """
            self.assertEqual(request.method, "POST")
            self.assertEqual(request.url.path, "/knavi/journals/TEST/yearList")
            return httpx.Response(
                200,
                text='<a id="yq202401" value="202401">2024年第01期</a>',
            )

        client = await _mock_cnki_client(handler, stats_recorder)
        try:
            issues = await client.get_year_issues(
                {
                    "pykm": "TEST",
                    "pcode": "CJFD",
                    "time": "token",
                    "detail_url": "https://oversea.cnki.net/knavi/detail?x=1",
                }
            )
        finally:
            await client.aclose()

        api_stats = next(iter(stats_recorder.stats.api_stats.values()))
        self.assertEqual(len(issues), 1)
        self.assertEqual(api_stats.key.service, "cnki")
        self.assertEqual(api_stats.key.endpoint, "year_issues")
        self.assertEqual(api_stats.key.method, "POST")
        self.assertEqual(api_stats.key.url_path, "/knavi/journals/TEST/yearList")
        self.assertEqual(api_stats.logical_calls, 1)
        self.assertEqual(api_stats.attempts, 1)
        self.assertEqual(api_stats.successes, 1)


class SemanticScholarTransformTest(unittest.TestCase):
    """
    Verify Semantic Scholar fields map into article records.
    """

    def test_s2_pdf_and_oa_map_without_using_s2_page_as_content_location(self) -> None:
        """
        Ensure S2 contributes PDF and OA state but not the landing page field.
        """
        record = build_scholarly_article_record(
            _build_crossref_work("10.1/s2", url="https://publisher.test/article"),
            {
                "best_oa_location": {
                    "pdf_url": "https://openalex.test/article.pdf",
                    "landing_page_url": "https://publisher.test/oa",
                },
                "open_access": {"is_oa": False},
            },
            {
                "url": "https://www.semanticscholar.org/paper/example",
                "isOpenAccess": True,
                "openAccessPdf": {"url": "https://s2.test/article.pdf"},
            },
            1,
            None,
        )

        assert record is not None
        self.assertEqual(record["open_access"], 1)
        self.assertEqual(record["full_text_file"], "https://s2.test/article.pdf")
        self.assertEqual(record["content_location"], "https://publisher.test/oa")

    def test_content_location_falls_back_to_doi_without_s2_page(self) -> None:
        """
        Ensure S2 website URLs do not become content locations.
        """
        record = build_scholarly_article_record(
            _build_crossref_work("10.1/fallback", url=None),
            None,
            {
                "url": "https://www.semanticscholar.org/paper/fallback",
                "isOpenAccess": False,
            },
            1,
            None,
        )

        assert record is not None
        self.assertEqual(record["open_access"], 0)
        self.assertEqual(record["content_location"], "https://doi.org/10.1/fallback")
        self.assertIsNone(record["full_text_file"])


async def _mock_scholarly_client(
    handler: Callable[[httpx.Request], httpx.Response],
    stats_recorder: IndexStatsRecorder | None = None,
) -> ScholarlyClient:
    """
    Build a scholarly client backed by an httpx mock transport.

    Args:
        handler: Mock transport request handler.
        stats_recorder: Optional index statistics recorder.

    Returns:
        Scholarly client using the mock transport.
    """
    client = ScholarlyClient(
        request_throttles=ScholarlyRequestThrottles([]),
        stats_recorder=stats_recorder,
    )
    await client.aclose()
    client._clients = [httpx.AsyncClient(transport=httpx.MockTransport(handler))]
    return client


async def _mock_cnki_client(
    handler: Callable[[httpx.Request], httpx.Response],
    stats_recorder: IndexStatsRecorder,
) -> CnkiClient:
    """
    Build a CNKI client backed by an httpx mock transport.

    Args:
        handler: Mock transport request handler.
        stats_recorder: Index statistics recorder.

    Returns:
        CNKI client using the mock transport.
    """
    client = CnkiClient(stats_recorder=stats_recorder)
    await client.aclose()
    client._clients = [httpx.AsyncClient(transport=httpx.MockTransport(handler))]
    return client


def _restore_env(name: str, value: str | None) -> None:
    """
    Restore one environment variable.

    Args:
        name: Environment variable name.
        value: Previous value or None when absent.

    Returns:
        None.
    """
    if value is None:
        os.environ.pop(name, None)
        return
    os.environ[name] = value


def _build_crossref_work(doi: str, url: str | None) -> dict[str, Any]:
    """
    Build a minimal Crossref work payload.

    Args:
        doi: DOI value.
        url: Optional Crossref URL.

    Returns:
        Crossref-like work payload.
    """
    work: dict[str, Any] = {
        "DOI": doi,
        "title": [f"Article {doi}"],
        "published": {"date-parts": [[2026, 1, 1]]},
    }
    if url is not None:
        work["URL"] = url
    return work
