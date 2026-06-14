"""Client for Crossref, OpenAlex, and Unpaywall metadata sources."""

from __future__ import annotations

import asyncio
import os
import random
from collections.abc import Mapping
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from functools import partial
from typing import Any
from urllib.parse import quote

import httpx

from paper_scanner.shared.converters import chunked
from paper_scanner.shared.request_pools import (
    build_async_client_pool,
    build_proxy_pool,
    build_value_pool,
    close_async_client_pool,
    request_pool_key,
    select_async_client,
    select_pool_value,
)
from paper_scanner.sources.scholarly.limits import (
    CROSSREF_SOURCE,
    OPENALEX_SOURCE,
    ScholarlyRequestThrottles,
    build_scholarly_request_throttles,
)

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
        worker_id: Current worker process identifier for shared source throttles.
        process_count: Total process count sharing upstream source limits.
    """

    def __init__(
        self,
        timeout: int = 20,
        worker_id: int = 0,
        process_count: int = 1,
        request_throttles: ScholarlyRequestThrottles | None = None,
    ) -> None:
        self.mailto_pool = build_value_pool(os.getenv("CROSSREF_MAILTO_POOL"))
        self.openalex_api_key_pool = build_value_pool(
            os.getenv("OPENALEX_API_KEY_POOL")
        )
        self.unpaywall_email_pool = build_value_pool(os.getenv("UNPAYWALL_EMAIL_POOL"))
        self.proxy_pool = build_proxy_pool(os.getenv("PROXY_POOL"))
        self._clients = build_async_client_pool(
            timeout=timeout,
            headers={"User-Agent": self._build_user_agent()},
            follow_redirects=True,
            proxy_pool=self.proxy_pool,
        )
        self._request_throttles = (
            request_throttles
            or build_scholarly_request_throttles(
                worker_id=worker_id,
                process_count=process_count,
            )
        )

    async def aclose(self) -> None:
        """
        Close the underlying HTTP client.

        Returns:
            None.
        """
        await close_async_client_pool(self._clients)

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
        works: list[dict[str, Any]] = []
        while True:
            response = await self._get_with_retries(
                f"{CROSSREF_BASE_URL}/journals/{quote(issn)}/works",
                params=params,
                param_pools={"mailto": self.mailto_pool},
                source=CROSSREF_SOURCE,
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
            try:
                response = await self._get_with_retries(
                    f"{OPENALEX_BASE_URL}/works",
                    params=params,
                    param_pools={
                        "api_key": self.openalex_api_key_pool,
                        "mailto": self.mailto_pool,
                    },
                    source=OPENALEX_SOURCE,
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
        if not self.unpaywall_email_pool:
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
                        params={},
                        param_pools={"email": self.unpaywall_email_pool},
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
        param_pools: Mapping[str, list[str]] | None = None,
        allowed_status_codes: set[int] | None = None,
        max_retries: int = DEFAULT_MAX_RETRIES,
        source: str | None = None,
    ) -> httpx.Response:
        """
        Send a GET request with retry handling for transient upstream limits.

        Args:
            url: Request URL.
            params: Query parameters.
            param_pools: Query parameter value pools for retry failover.
            allowed_status_codes: Status codes that should be returned as success.
            max_retries: Maximum retry attempts after the initial request.
            source: Optional source identifier for request throttling.

        Returns:
            HTTP response.
        """
        allowed = allowed_status_codes or set()
        request_key = request_pool_key(url, params)
        total_attempts = max_retries + 1
        failover_count = _failover_count(param_pools, len(self._clients))
        for attempt in range(total_attempts):
            request_params = _params_for_attempt(
                params,
                param_pools,
                request_key,
                attempt,
            )
            try:
                client = select_async_client(
                    self._clients,
                    request_key,
                    attempt,
                )
                request_operation = partial(client.get, url, params=request_params)
                response = await self._request_throttles.run(
                    source,
                    request_operation,
                )
            except httpx.TransportError:
                if attempt >= total_attempts - 1:
                    raise
                await asyncio.sleep(
                    _retry_delay_for_attempt(None, attempt, failover_count)
                )
                continue

            if response.status_code in allowed:
                return response
            if response.status_code not in RETRY_STATUS_CODES:
                response.raise_for_status()
                return response
            if attempt >= total_attempts - 1:
                response.raise_for_status()
            await asyncio.sleep(
                _retry_delay_for_attempt(response, attempt, failover_count)
            )

        raise RuntimeError("Retry loop exited unexpectedly.")

    def _build_user_agent(self) -> str:
        """
        Build a request User-Agent.

        Returns:
            User-Agent string.
        """
        mailto = next(iter(self.mailto_pool), "")
        if mailto:
            return f"Paper-Scanner/0.1 (mailto:{mailto})"
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


def _params_for_attempt(
    params: Mapping[str, str | int] | None,
    param_pools: Mapping[str, list[str]] | None,
    request_key: str,
    attempt: int,
) -> dict[str, str | int]:
    """
    Build request parameters with retry-specific pool values.

    Args:
        params: Base query parameters.
        param_pools: Query parameter value pools.
        request_key: Stable request selection key.
        attempt: Retry attempt offset.

    Returns:
        Request parameters for the attempt.
    """
    request_params: dict[str, str | int] = dict(params or {})
    for name, pool in (param_pools or {}).items():
        value = select_pool_value(pool, request_key, attempt)
        if value:
            request_params[name] = value
    return request_params


def _failover_count(
    param_pools: Mapping[str, list[str]] | None,
    client_count: int,
) -> int:
    """
    Calculate the number of retry slots before backoff is needed.

    Args:
        param_pools: Query parameter value pools.
        client_count: HTTP client pool size.

    Returns:
        Largest pool size used by a request.
    """
    counts = [max(1, client_count)]
    counts.extend(len(pool) for pool in (param_pools or {}).values() if pool)
    return max(counts)


def _retry_delay_for_attempt(
    response: httpx.Response | None,
    attempt: int,
    failover_count: int,
) -> float:
    """
    Calculate retry delay while prioritizing unused pool failover.

    Args:
        response: Optional HTTP response.
        attempt: Zero-based retry attempt.
        failover_count: Number of failover candidates.

    Returns:
        Delay in seconds.
    """
    if attempt + 1 < failover_count:
        return 0.0
    return _retry_delay(response, attempt)


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
