"""Tests for Semantic Scholar scholarly enrichment."""

from __future__ import annotations

import json
import os
import unittest
from collections.abc import Callable
from typing import Any

import httpx

from paper_scanner.index.transforms import build_scholarly_article_record
from paper_scanner.sources.scholarly.client import ScholarlyClient
from paper_scanner.sources.scholarly.limits import ScholarlyRequestThrottles

SEMANTIC_SCHOLAR_KEY_ENV = "SEMANTIC_SCHOLAR_API_KEY_POOL"


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

    async def test_fetch_semantic_scholar_by_dois_skips_without_key(self) -> None:
        """
        Ensure missing S2 configuration disables optional enrichment.
        """
        previous_key = os.environ.get(SEMANTIC_SCHOLAR_KEY_ENV)
        os.environ.pop(SEMANTIC_SCHOLAR_KEY_ENV, None)
        client = ScholarlyClient(request_throttles=ScholarlyRequestThrottles([]))
        try:
            result = await client.fetch_semantic_scholar_by_dois(["10.1/a"])
        finally:
            await client.aclose()
            _restore_env(SEMANTIC_SCHOLAR_KEY_ENV, previous_key)

        self.assertEqual(result, {})


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
) -> ScholarlyClient:
    """
    Build a scholarly client backed by an httpx mock transport.

    Args:
        handler: Mock transport request handler.

    Returns:
        Scholarly client using the mock transport.
    """
    client = ScholarlyClient(request_throttles=ScholarlyRequestThrottles([]))
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
