"""Client for Crossref, OpenAlex, and Semantic Scholar metadata sources."""

from __future__ import annotations

import asyncio
import os
import random
import time
from collections.abc import Mapping
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from functools import partial
from typing import Any
from urllib.parse import quote

import httpx

from paper_scanner.index.stats import (
    ApiStatsKey,
    IndexStatsRecorder,
    NoOpIndexStatsRecorder,
)
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
    SEMANTIC_SCHOLAR_SOURCE,
    ScholarlyRequestThrottles,
    build_scholarly_request_throttles,
)

CROSSREF_BASE_URL = "https://api.crossref.org/v1"
OPENALEX_BASE_URL = "https://api.openalex.org"
SEMANTIC_SCHOLAR_BASE_URL = "https://api.semanticscholar.org/graph/v1"
SEMANTIC_SCHOLAR_BATCH_SIZE = 500
SEMANTIC_SCHOLAR_FIELDS = "externalIds,url,isOpenAccess,openAccessPdf"
OPENALEX_SOURCE_FIELDS = "id,display_name,issn_l,issn,works_count"
OPENALEX_WORK_FIELDS = (
    "id,doi,title,display_name,publication_year,publication_date,language,"
    "cited_by_count,is_retracted,primary_location,locations,open_access,"
    "best_oa_location,authorships,ids,biblio,abstract_inverted_index,topics,"
    "primary_topic,funders,awards"
)
DEFAULT_USER_AGENT = "Paper-Scanner/0.1 (mailto:paper-scanner@example.invalid)"
RETRY_STATUS_CODES = {429, 500, 502, 503, 504}
DEFAULT_MAX_RETRIES = 6
BASE_RETRY_SECONDS = 2.0
MAX_RETRY_SECONDS = 60.0


class ScholarlyConfigurationError(RuntimeError):
    """Raised when required scholarly source configuration is missing."""


class ScholarlyClient:
    """
    Fetch article metadata from Crossref, OpenAlex, and Semantic Scholar.

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
        stats_recorder: IndexStatsRecorder | NoOpIndexStatsRecorder | None = None,
    ) -> None:
        self.mailto_pool = build_value_pool(os.getenv("CROSSREF_MAILTO_POOL"))
        self.openalex_api_key_pool = build_value_pool(
            os.getenv("OPENALEX_API_KEY_POOL")
        )
        self.semantic_scholar_api_key_pool = build_value_pool(
            os.getenv("SEMANTIC_SCHOLAR_API_KEY_POOL")
        )
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
        self._stats_recorder = stats_recorder or NoOpIndexStatsRecorder()

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
                endpoint="journal_works",
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

    async def fetch_openalex_source_by_issns(
        self, issns: list[str]
    ) -> dict[str, Any] | None:
        """
        Fetch an OpenAlex source matching one of the supplied ISSNs.

        Args:
            issns: ISSN candidates in lookup order.

        Returns:
            Matching OpenAlex source payload or None.
        """
        for issn in issns:
            response = await self._get_with_retries(
                f"{OPENALEX_BASE_URL}/sources",
                params={
                    "filter": f"issn:{issn}",
                    "per-page": 5,
                    "select": OPENALEX_SOURCE_FIELDS,
                },
                param_pools={
                    "api_key": self.openalex_api_key_pool,
                    "mailto": self.mailto_pool,
                },
                source=OPENALEX_SOURCE,
                endpoint="sources",
            )
            for item in response.json().get("results") or []:
                if not isinstance(item, dict):
                    continue
                if _openalex_source_matches_issn(item, issn):
                    return item
            await asyncio.sleep(0)
        return None

    async def fetch_openalex_source_by_title(
        self,
        title: str,
    ) -> dict[str, Any] | None:
        """
        Fetch an OpenAlex source matching a title exactly.

        Args:
            title: Journal title.

        Returns:
            Matching OpenAlex source payload or None.
        """
        normalized_title = _normalize_source_title(title)
        if not normalized_title:
            return None
        response = await self._get_with_retries(
            f"{OPENALEX_BASE_URL}/sources",
            params={
                "search": title,
                "per-page": 5,
                "select": OPENALEX_SOURCE_FIELDS,
            },
            param_pools={
                "api_key": self.openalex_api_key_pool,
                "mailto": self.mailto_pool,
            },
            source=OPENALEX_SOURCE,
            endpoint="source_search",
        )
        for item in response.json().get("results") or []:
            if not isinstance(item, dict):
                continue
            if _openalex_source_matches_title(item, normalized_title):
                return item
        return None

    async def fetch_openalex_works_by_source(
        self,
        source_id: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Fetch OpenAlex article works by source identifier.

        Args:
            source_id: OpenAlex source id or URL.
            from_pub_date: Optional minimum publication date.
            until_pub_date: Optional maximum publication date.

        Returns:
            List of OpenAlex work payloads.
        """
        source_key = _openalex_short_source_id(source_id)
        if not source_key:
            return []
        filters = [f"primary_location.source.id:{source_key}", "type:article"]
        if from_pub_date:
            filters.append(f"from_publication_date:{from_pub_date}")
        if until_pub_date:
            filters.append(f"to_publication_date:{until_pub_date}")
        params: dict[str, str | int] = {
            "filter": ",".join(filters),
            "per-page": 200,
            "cursor": "*",
            "sort": "publication_date:asc",
            "select": OPENALEX_WORK_FIELDS,
        }
        works: list[dict[str, Any]] = []
        while True:
            response = await self._get_with_retries(
                f"{OPENALEX_BASE_URL}/works",
                params=params,
                param_pools={
                    "api_key": self.openalex_api_key_pool,
                    "mailto": self.mailto_pool,
                },
                source=OPENALEX_SOURCE,
                endpoint="source_works",
            )
            message = response.json()
            items = message.get("results") or []
            works.extend(item for item in items if isinstance(item, dict))
            next_cursor = (message.get("meta") or {}).get("next_cursor")
            if not items or not next_cursor or len(items) < int(params["per-page"]):
                break
            params["cursor"] = str(next_cursor)
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
                "select": OPENALEX_WORK_FIELDS,
            }
            response = await self._get_with_retries(
                f"{OPENALEX_BASE_URL}/works",
                params=params,
                param_pools={
                    "api_key": self.openalex_api_key_pool,
                    "mailto": self.mailto_pool,
                },
                source=OPENALEX_SOURCE,
                endpoint="works",
            )
            for item in response.json().get("results") or []:
                if not isinstance(item, dict):
                    continue
                doi = _normalize_doi(item.get("doi"))
                if doi:
                    results[doi] = item
            await asyncio.sleep(0)
        return results

    async def fetch_semantic_scholar_by_dois(
        self, dois: list[str], batch_size: int = SEMANTIC_SCHOLAR_BATCH_SIZE
    ) -> dict[str, dict[str, Any]]:
        """
        Fetch Semantic Scholar OA records by DOI.

        Args:
            dois: DOI list.
            batch_size: Number of DOI IDs per request.

        Returns:
            Mapping from normalized DOI to Semantic Scholar payload.
        """
        normalized = [doi for doi in {_normalize_doi(doi) for doi in dois} if doi]
        if not normalized:
            return {}
        if not self.semantic_scholar_api_key_pool:
            raise ScholarlyConfigurationError(
                "Semantic Scholar API key is required for DOI enrichment."
            )

        results: dict[str, dict[str, Any]] = {}
        effective_batch_size = min(
            max(1, batch_size),
            SEMANTIC_SCHOLAR_BATCH_SIZE,
        )
        for batch in chunked(normalized, effective_batch_size):
            try:
                response = await self._post_with_retries(
                    f"{SEMANTIC_SCHOLAR_BASE_URL}/paper/batch",
                    params={"fields": SEMANTIC_SCHOLAR_FIELDS},
                    json_body={"ids": [f"DOI:{doi}" for doi in batch]},
                    header_pools={
                        "x-api-key": self.semantic_scholar_api_key_pool,
                    },
                    source=SEMANTIC_SCHOLAR_SOURCE,
                    endpoint="paper_batch",
                )
            except httpx.HTTPStatusError as exc:
                if _is_semantic_scholar_no_valid_ids_response(exc.response):
                    await asyncio.sleep(0)
                    continue
                raise
            payload = response.json()
            if isinstance(payload, list):
                for item in payload:
                    if not isinstance(item, dict):
                        continue
                    doi = _semantic_scholar_doi(item)
                    if doi:
                        results[doi] = item
            await asyncio.sleep(0)
        return results

    async def _post_with_retries(
        self,
        url: str,
        params: Mapping[str, str | int] | None = None,
        json_body: dict[str, Any] | None = None,
        param_pools: Mapping[str, list[str]] | None = None,
        header_pools: Mapping[str, list[str]] | None = None,
        allowed_status_codes: set[int] | None = None,
        max_retries: int = DEFAULT_MAX_RETRIES,
        source: str | None = None,
        endpoint: str | None = None,
    ) -> httpx.Response:
        """
        Send a POST request with retry handling for transient upstream limits.

        Args:
            url: Request URL.
            params: Query parameters.
            json_body: JSON request body.
            param_pools: Query parameter value pools for retry failover.
            header_pools: Header value pools for retry failover.
            allowed_status_codes: Status codes that should be returned as success.
            max_retries: Maximum retry attempts after the initial request.
            source: Optional source identifier for request throttling.
            endpoint: Optional endpoint label for statistics.

        Returns:
            HTTP response.
        """
        return await self._request_with_retries(
            "POST",
            url,
            params=params,
            json_body=json_body,
            param_pools=param_pools,
            header_pools=header_pools,
            allowed_status_codes=allowed_status_codes,
            max_retries=max_retries,
            source=source,
            endpoint=endpoint,
        )

    async def _get_with_retries(
        self,
        url: str,
        params: Mapping[str, str | int] | None = None,
        param_pools: Mapping[str, list[str]] | None = None,
        allowed_status_codes: set[int] | None = None,
        max_retries: int = DEFAULT_MAX_RETRIES,
        source: str | None = None,
        endpoint: str | None = None,
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
            endpoint: Optional endpoint label for statistics.

        Returns:
            HTTP response.
        """
        return await self._request_with_retries(
            "GET",
            url,
            params=params,
            param_pools=param_pools,
            allowed_status_codes=allowed_status_codes,
            max_retries=max_retries,
            source=source,
            endpoint=endpoint,
        )

    async def _request_with_retries(
        self,
        method: str,
        url: str,
        params: Mapping[str, str | int] | None = None,
        json_body: dict[str, Any] | None = None,
        param_pools: Mapping[str, list[str]] | None = None,
        header_pools: Mapping[str, list[str]] | None = None,
        allowed_status_codes: set[int] | None = None,
        max_retries: int = DEFAULT_MAX_RETRIES,
        source: str | None = None,
        endpoint: str | None = None,
    ) -> httpx.Response:
        """
        Send an HTTP request with retry handling for transient upstream limits.

        Args:
            method: HTTP method.
            url: Request URL.
            params: Query parameters.
            json_body: Optional JSON request body.
            param_pools: Query parameter value pools for retry failover.
            header_pools: Header value pools for retry failover.
            allowed_status_codes: Status codes that should be returned as success.
            max_retries: Maximum retry attempts after the initial request.
            source: Optional source identifier for request throttling.
            endpoint: Optional endpoint label for statistics.

        Returns:
            HTTP response.
        """
        allowed = allowed_status_codes or set()
        api_stats_key = self._stats_recorder.record_api_call(
            service=source or "http",
            endpoint=endpoint or "request",
            method=method,
            url=url,
        )
        request_key = request_pool_key(f"{method.upper()} {url}", params)
        total_attempts = max_retries + 1
        failover_count = _failover_count(
            param_pools,
            len(self._clients),
            header_pools=header_pools,
        )
        for attempt in range(total_attempts):
            request_params = _params_for_attempt(
                params,
                param_pools,
                request_key,
                attempt,
            )
            request_headers = _headers_for_attempt(
                header_pools,
                request_key,
                attempt,
            )
            request_kwargs: dict[str, Any] = {"params": request_params}
            if json_body is not None:
                request_kwargs["json"] = json_body
            if request_headers:
                request_kwargs["headers"] = request_headers
            started_at = time.monotonic()
            try:
                client = select_async_client(
                    self._clients,
                    request_key,
                    attempt,
                )
                request_operation = partial(
                    client.request,
                    method,
                    url,
                    **request_kwargs,
                )
                response = await self._request_throttles.run(
                    source,
                    request_operation,
                )
            except httpx.TransportError as exc:
                self._record_api_attempt(
                    api_stats_key,
                    attempt,
                    status_code=None,
                    did_succeed=False,
                    started_at=started_at,
                    error=exc,
                )
                if attempt >= total_attempts - 1:
                    raise
                await asyncio.sleep(
                    _retry_delay_for_attempt(None, attempt, failover_count)
                )
                continue

            if response.status_code in allowed:
                self._record_api_attempt(
                    api_stats_key,
                    attempt,
                    status_code=response.status_code,
                    did_succeed=True,
                    started_at=started_at,
                )
                return response
            if response.status_code not in RETRY_STATUS_CODES:
                try:
                    response.raise_for_status()
                except httpx.HTTPStatusError as exc:
                    self._record_api_attempt(
                        api_stats_key,
                        attempt,
                        status_code=response.status_code,
                        did_succeed=False,
                        started_at=started_at,
                        error=exc,
                    )
                    raise
                self._record_api_attempt(
                    api_stats_key,
                    attempt,
                    status_code=response.status_code,
                    did_succeed=True,
                    started_at=started_at,
                )
                return response
            if attempt >= total_attempts - 1:
                try:
                    response.raise_for_status()
                except httpx.HTTPStatusError as exc:
                    self._record_api_attempt(
                        api_stats_key,
                        attempt,
                        status_code=response.status_code,
                        did_succeed=False,
                        started_at=started_at,
                        error=exc,
                    )
                    raise
            self._record_api_attempt(
                api_stats_key,
                attempt,
                status_code=response.status_code,
                did_succeed=False,
                started_at=started_at,
                error=f"HTTP {response.status_code}",
            )
            await asyncio.sleep(
                _retry_delay_for_attempt(response, attempt, failover_count)
            )

        raise RuntimeError("Retry loop exited unexpectedly.")

    def _record_api_attempt(
        self,
        api_stats_key: ApiStatsKey,
        attempt: int,
        status_code: int | None,
        did_succeed: bool,
        started_at: float,
        error: BaseException | str | None = None,
    ) -> None:
        """
        Record one scholarly API attempt.

        Args:
            api_stats_key: API statistics key.
            attempt: Zero-based attempt number.
            status_code: HTTP status code when available.
            did_succeed: Whether the attempt succeeded.
            started_at: Monotonic start time.
            error: Attempt error when available.

        Returns:
            None.
        """
        self._stats_recorder.record_api_attempt(
            api_stats_key,
            status_code=status_code,
            did_succeed=did_succeed,
            elapsed_ms=(time.monotonic() - started_at) * 1000,
            error=error,
            did_retry=attempt > 0,
        )

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


def _semantic_scholar_doi(item: dict[str, Any]) -> str | None:
    """
    Extract a normalized DOI from a Semantic Scholar paper payload.

    Args:
        item: Semantic Scholar paper payload.

    Returns:
        Normalized DOI or None.
    """
    external_ids = item.get("externalIds")
    if not isinstance(external_ids, dict):
        return None
    return _normalize_doi(external_ids.get("DOI"))


def _is_semantic_scholar_no_valid_ids_response(response: httpx.Response) -> bool:
    """
    Return whether an S2 response means none of the batch IDs are known.

    Args:
        response: Semantic Scholar batch response.

    Returns:
        Whether the response is the no-valid-paper-ids sentinel.
    """
    if response.status_code != 400:
        return False
    try:
        payload = response.json()
    except ValueError:
        return False
    if not isinstance(payload, dict):
        return False
    return str(payload.get("error") or "").strip().lower() == (
        "no valid paper ids given"
    )


def _openalex_source_matches_issn(item: dict[str, Any], issn: str) -> bool:
    """
    Return whether an OpenAlex source contains an ISSN.

    Args:
        item: OpenAlex source payload.
        issn: ISSN to match.

    Returns:
        Whether the source contains the ISSN.
    """
    target = _normalize_issn(issn)
    if not target:
        return False
    candidates = [_normalize_issn(item.get("issn_l"))]
    raw_issns = item.get("issn")
    if isinstance(raw_issns, list):
        candidates.extend(_normalize_issn(value) for value in raw_issns)
    return target in {candidate for candidate in candidates if candidate}


def _openalex_source_matches_title(
    item: dict[str, Any],
    normalized_title: str,
) -> bool:
    """
    Return whether an OpenAlex source title matches exactly.

    Args:
        item: OpenAlex source payload.
        normalized_title: Normalized target title.

    Returns:
        Whether the display name exactly matches the target title.
    """
    return _normalize_source_title(item.get("display_name")) == normalized_title


def _normalize_issn(value: Any) -> str:
    """
    Normalize an ISSN for comparison.

    Args:
        value: Raw ISSN.

    Returns:
        Normalized ISSN.
    """
    if value is None:
        return ""
    return str(value).strip().replace("-", "").upper()


def _normalize_source_title(value: Any) -> str:
    """
    Normalize a source title for exact comparisons.

    Args:
        value: Raw title value.

    Returns:
        Case-folded title with collapsed whitespace.
    """
    return " ".join(str(value or "").split()).casefold()


def _openalex_short_source_id(value: Any) -> str | None:
    """
    Extract a compact OpenAlex source identifier.

    Args:
        value: OpenAlex source URL or id.

    Returns:
        Compact source id.
    """
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    return text.rsplit("/", maxsplit=1)[-1]


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


def _headers_for_attempt(
    header_pools: Mapping[str, list[str]] | None,
    request_key: str,
    attempt: int,
) -> dict[str, str]:
    """
    Build request headers with retry-specific pool values.

    Args:
        header_pools: Header value pools.
        request_key: Stable request selection key.
        attempt: Retry attempt offset.

    Returns:
        Request headers for the attempt.
    """
    request_headers: dict[str, str] = {}
    for name, pool in (header_pools or {}).items():
        value = select_pool_value(pool, request_key, attempt)
        if value:
            request_headers[name] = value
    return request_headers


def _failover_count(
    param_pools: Mapping[str, list[str]] | None,
    client_count: int,
    *,
    header_pools: Mapping[str, list[str]] | None = None,
) -> int:
    """
    Calculate the number of retry slots before backoff is needed.

    Args:
        param_pools: Query parameter value pools.
        client_count: HTTP client pool size.
        header_pools: Header value pools.

    Returns:
        Largest pool size used by a request.
    """
    counts = [max(1, client_count)]
    counts.extend(len(pool) for pool in (param_pools or {}).values() if pool)
    counts.extend(len(pool) for pool in (header_pools or {}).values() if pool)
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
