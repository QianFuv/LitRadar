"""Client for Crossref, OpenAlex, and Unpaywall metadata sources."""

from __future__ import annotations

import asyncio
import os
import random
from collections.abc import Mapping
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from typing import Any
from urllib.parse import quote

import httpx

from scripts.shared.converters import chunked

CROSSREF_BASE_URL = "https://api.crossref.org/v1"
OPENALEX_BASE_URL = "https://api.openalex.org"
UNPAYWALL_BASE_URL = "https://api.unpaywall.org/v2"
DEFAULT_USER_AGENT = "Paper-Scanner/0.1 (mailto:paper-scanner@example.invalid)"
RETRY_STATUS_CODES = {429, 500, 502, 503, 504}
DEFAULT_MAX_RETRIES = 6
BASE_RETRY_SECONDS = 2.0
MAX_RETRY_SECONDS = 60.0


class ScholarlyClient:
    """
    Fetch article metadata from Crossref, OpenAlex, and Unpaywall.

    Args:
        timeout: HTTP request timeout in seconds.
        mailto: Contact email for Crossref polite pool requests.
        openalex_api_key: Optional OpenAlex API key.
        unpaywall_email: Contact email required by Unpaywall.
    """

    def __init__(
        self,
        timeout: int = 20,
        mailto: str | None = None,
        openalex_api_key: str | None = None,
        unpaywall_email: str | None = None,
    ) -> None:
        self.mailto = mailto or os.getenv("CROSSREF_MAILTO") or ""
        self.openalex_api_key = openalex_api_key or os.getenv("OPENALEX_API_KEY") or ""
        self.unpaywall_email = (
            unpaywall_email or os.getenv("UNPAYWALL_EMAIL") or self.mailto
        )
        self._client = httpx.AsyncClient(
            timeout=timeout,
            headers={"User-Agent": self._build_user_agent()},
            follow_redirects=True,
        )

    async def aclose(self) -> None:
        """
        Close the underlying HTTP client.

        Returns:
            None.
        """
        await self._client.aclose()

    async def fetch_journal_works(
        self,
        issn: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Fetch Crossref journal article works by ISSN.

        Args:
            issn: Journal ISSN.
            from_pub_date: Optional minimum publication date.
            until_pub_date: Optional maximum publication date.

        Returns:
            List of Crossref work payloads.
        """
        filters = ["type:journal-article"]
        if from_pub_date:
            filters.append(f"from-pub-date:{from_pub_date}")
        if until_pub_date:
            filters.append(f"until-pub-date:{until_pub_date}")

        params: dict[str, str | int] = {
            "rows": 1000,
            "cursor": "*",
            "filter": ",".join(filters),
            "sort": "published",
            "order": "asc",
        }
        if self.mailto:
            params["mailto"] = self.mailto

        works: list[dict[str, Any]] = []
        while True:
            response = await self._get_with_retries(
                f"{CROSSREF_BASE_URL}/journals/{quote(issn)}/works",
                params=params,
            )
            message = response.json().get("message") or {}
            items = message.get("items") or []
            works.extend(item for item in items if isinstance(item, dict))
            next_cursor = message.get("next-cursor")
            if not items or not next_cursor or len(items) < int(params["rows"]):
                break
            params["cursor"] = next_cursor
            await asyncio.sleep(0)
        return works

    async def fetch_openalex_by_dois(
        self, dois: list[str], batch_size: int = 100
    ) -> dict[str, dict[str, Any]]:
        """
        Fetch OpenAlex work enrichment by DOI.

        Args:
            dois: DOI list.
            batch_size: Number of DOI filters per request.

        Returns:
            Mapping from normalized DOI to OpenAlex work payload.
        """
        normalized = [doi for doi in {_normalize_doi(doi) for doi in dois} if doi]
        results: dict[str, dict[str, Any]] = {}
        for batch in chunked(normalized, batch_size):
            filter_value = "|".join(f"https://doi.org/{doi}" for doi in batch)
            params: dict[str, str | int] = {
                "filter": f"doi:{filter_value}",
                "per-page": len(batch),
                "select": (
                    "id,doi,title,display_name,publication_year,publication_date,"
                    "language,cited_by_count,is_retracted,primary_location,"
                    "locations,open_access,best_oa_location,authorships,ids,biblio,"
                    "abstract_inverted_index,topics,primary_topic,funders,awards"
                ),
            }
            if self.openalex_api_key:
                params["api_key"] = self.openalex_api_key
            if self.mailto:
                params["mailto"] = self.mailto
            try:
                response = await self._get_with_retries(
                    f"{OPENALEX_BASE_URL}/works", params=params
                )
            except httpx.HTTPError:
                await asyncio.sleep(0)
                continue
            for item in response.json().get("results") or []:
                if not isinstance(item, dict):
                    continue
                doi = _normalize_doi(item.get("doi"))
                if doi:
                    results[doi] = item
            await asyncio.sleep(0)
        return results

    async def fetch_unpaywall_by_dois(
        self, dois: list[str], request_workers: int = 4
    ) -> dict[str, dict[str, Any]]:
        """
        Fetch Unpaywall OA records by DOI.

        Args:
            dois: DOI list.
            request_workers: Maximum concurrent requests.

        Returns:
            Mapping from normalized DOI to Unpaywall payload.
        """
        if not self.unpaywall_email:
            return {}

        normalized = [doi for doi in {_normalize_doi(doi) for doi in dois} if doi]
        semaphore = asyncio.Semaphore(max(1, request_workers))
        results: dict[str, dict[str, Any]] = {}

        async def fetch_one(doi: str) -> None:
            """
            Fetch one DOI record.

            Args:
                doi: Normalized DOI.

            Returns:
                None.
            """
            async with semaphore:
                try:
                    response = await self._get_with_retries(
                        f"{UNPAYWALL_BASE_URL}/{quote(doi, safe='')}",
                        params={"email": self.unpaywall_email},
                        allowed_status_codes={404},
                    )
                except httpx.HTTPError:
                    return
                if response.status_code == 404:
                    return
                payload = response.json()
                if isinstance(payload, dict):
                    results[doi] = payload

        await asyncio.gather(*(fetch_one(doi) for doi in normalized))
        return results

    async def _get_with_retries(
        self,
        url: str,
        params: Mapping[str, str | int] | None = None,
        allowed_status_codes: set[int] | None = None,
        max_retries: int = DEFAULT_MAX_RETRIES,
    ) -> httpx.Response:
        """
        Send a GET request with retry handling for transient upstream limits.

        Args:
            url: Request URL.
            params: Query parameters.
            allowed_status_codes: Status codes that should be returned as success.
            max_retries: Maximum retry attempts after the initial request.

        Returns:
            HTTP response.
        """
        allowed = allowed_status_codes or set()
        for attempt in range(max_retries + 1):
            try:
                response = await self._client.get(url, params=params)
            except httpx.TransportError:
                if attempt >= max_retries:
                    raise
                await asyncio.sleep(_retry_delay(None, attempt))
                continue

            if response.status_code in allowed:
                return response
            if response.status_code not in RETRY_STATUS_CODES:
                response.raise_for_status()
                return response
            if attempt >= max_retries:
                response.raise_for_status()
            await asyncio.sleep(_retry_delay(response, attempt))

        raise RuntimeError("Retry loop exited unexpectedly.")

    def _build_user_agent(self) -> str:
        """
        Build a request User-Agent.

        Returns:
            User-Agent string.
        """
        if self.mailto:
            return f"Paper-Scanner/0.1 (mailto:{self.mailto})"
        return DEFAULT_USER_AGENT


def _normalize_doi(value: Any) -> str | None:
    """
    Normalize DOI-like values to a lowercase bare DOI.

    Args:
        value: Raw DOI value.

    Returns:
        Normalized DOI or None.
    """
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    lowered = text.lower()
    for prefix in ("https://doi.org/", "http://doi.org/", "doi:"):
        if lowered.startswith(prefix):
            lowered = lowered[len(prefix) :]
            break
    return lowered.strip() or None


def _retry_delay(response: httpx.Response | None, attempt: int) -> float:
    """
    Calculate retry delay from Retry-After or exponential backoff.

    Args:
        response: Optional HTTP response.
        attempt: Zero-based retry attempt.

    Returns:
        Delay in seconds.
    """
    if response is not None:
        retry_after = response.headers.get("Retry-After")
        parsed_delay = _parse_retry_after(retry_after)
        if parsed_delay is not None:
            return min(MAX_RETRY_SECONDS, parsed_delay)
    backoff = min(MAX_RETRY_SECONDS, BASE_RETRY_SECONDS * (2**attempt))
    return backoff + random.uniform(0, 1)


def _parse_retry_after(value: str | None) -> float | None:
    """
    Parse a Retry-After header value.

    Args:
        value: Header value.

    Returns:
        Delay in seconds or None.
    """
    if not value:
        return None
    try:
        return max(0.0, float(value))
    except ValueError:
        pass
    try:
        retry_at = parsedate_to_datetime(value)
    except (TypeError, ValueError):
        return None
    if retry_at.tzinfo is None:
        retry_at = retry_at.replace(tzinfo=UTC)
    return max(0.0, (retry_at - datetime.now(UTC)).total_seconds())
