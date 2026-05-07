"""Runtime request pool helpers for metadata clients."""

from __future__ import annotations

import hashlib
import re
from collections.abc import Mapping
from typing import Any
from urllib.parse import urlsplit

import httpx

GOPROXY_HTTP_PORTS = {7776, 7777}
GOPROXY_SOCKS_PORTS = {7779, 7780}
SUPPORTED_PROXY_SCHEMES = {"http", "https", "socks5", "socks5h"}


def build_value_pool(pool_value: str | None, fallback: str | None = None) -> list[str]:
    """
    Build a unique ordered value pool from configured values.

    Args:
        pool_value: Raw pool value with comma, semicolon, or newline separators.
        fallback: Optional single value to prepend.

    Returns:
        Unique value list in selection order.
    """
    pool: list[str] = []
    for value in (fallback or "", pool_value or ""):
        for part in re.split(r"[,;\n]+", value):
            item = part.strip()
            if item and item not in pool:
                pool.append(item)
    return pool


def select_pool_value(pool: list[str], key: str, attempt: int = 0) -> str | None:
    """
    Select a stable pool value for a key.

    Args:
        pool: Candidate values.
        key: Stable selection key.
        attempt: Retry attempt offset.

    Returns:
        Selected value or None when the pool is empty.
    """
    if not pool:
        return None
    digest = hashlib.blake2b(key.encode("utf-8"), digest_size=8).digest()
    index = (int.from_bytes(digest, "big") + attempt) % len(pool)
    return pool[index]


def build_proxy_pool(pool_value: str | None) -> list[str]:
    """
    Build normalized proxy URLs from runtime configuration.

    Args:
        pool_value: Raw proxy pool value.

    Returns:
        Normalized proxy URL list.
    """
    pool: list[str] = []
    for value in build_value_pool(pool_value):
        proxy = normalize_proxy_url(value)
        if proxy and proxy not in pool:
            pool.append(proxy)
    return pool


def normalize_proxy_url(value: str) -> str | None:
    """
    Normalize common proxy and GoProxy shorthand values.

    Args:
        value: Raw proxy value.

    Returns:
        Normalized proxy URL or None.
    """
    text = value.strip()
    if not text:
        return None
    parsed = urlsplit(text)
    scheme = parsed.scheme.lower()
    if scheme in SUPPORTED_PROXY_SCHEMES:
        return text
    if scheme == "socks":
        return f"socks5://{text.split('://', 1)[1]}"
    if scheme == "goproxy":
        return _normalize_goproxy_url(text)
    if "://" in text:
        return text
    port = _proxy_port(text)
    if port in GOPROXY_SOCKS_PORTS:
        return f"socks5://{text}"
    return f"http://{text}"


def _normalize_goproxy_url(value: str) -> str | None:
    """
    Normalize a GoProxy shorthand URL.

    Args:
        value: Raw GoProxy shorthand URL.

    Returns:
        Normalized proxy URL or None.
    """
    parsed = urlsplit(value)
    target = parsed.netloc or parsed.path.strip("/")
    if not target:
        return None
    port = _proxy_port(target)
    if port is None:
        target = f"{target}:7777"
        port = 7777
    if port in GOPROXY_SOCKS_PORTS:
        return f"socks5://{target}"
    if port in GOPROXY_HTTP_PORTS:
        return f"http://{target}"
    return f"http://{target}"


def _proxy_port(value: str) -> int | None:
    """
    Parse the port from a proxy host value.

    Args:
        value: Proxy host value without a scheme.

    Returns:
        Parsed port or None.
    """
    try:
        return urlsplit(f"//{value}").port
    except ValueError:
        return None


def request_pool_key(
    url: str,
    params: Mapping[str, Any] | None = None,
) -> str:
    """
    Build a stable key for request pool selection.

    Args:
        url: Request URL.
        params: Optional query parameters.

    Returns:
        Request selection key.
    """
    if not params:
        return url
    parts = [url]
    for key in sorted(params):
        parts.append(f"{key}={params[key]}")
    return "&".join(parts)


def build_async_client_pool(
    *,
    timeout: int,
    headers: dict[str, str] | None,
    follow_redirects: bool,
    proxy_pool: list[str],
) -> list[httpx.AsyncClient]:
    """
    Build AsyncClient instances for direct or proxied requests.

    Args:
        timeout: HTTP request timeout in seconds.
        headers: Default request headers.
        follow_redirects: Whether clients should follow redirects.
        proxy_pool: Proxy URLs for client routing.

    Returns:
        AsyncClient list.
    """
    if not proxy_pool:
        return [
            httpx.AsyncClient(
                timeout=timeout,
                headers=headers,
                follow_redirects=follow_redirects,
            )
        ]
    return [
        httpx.AsyncClient(
            timeout=timeout,
            headers=headers,
            follow_redirects=follow_redirects,
            proxy=proxy,
            trust_env=False,
        )
        for proxy in proxy_pool
    ]


def select_async_client(
    clients: list[httpx.AsyncClient],
    key: str,
    attempt: int = 0,
) -> httpx.AsyncClient:
    """
    Select a stable AsyncClient for a request key.

    Args:
        clients: AsyncClient pool.
        key: Stable request key.
        attempt: Retry attempt offset.

    Returns:
        Selected AsyncClient.
    """
    if len(clients) == 1:
        return clients[0]
    index_text = select_pool_value(
        [str(index) for index in range(len(clients))],
        key,
        attempt,
    )
    return clients[int(index_text or "0")]


async def close_async_client_pool(clients: list[httpx.AsyncClient]) -> None:
    """
    Close all clients in a pool.

    Args:
        clients: AsyncClient pool.

    Returns:
        None.
    """
    for client in clients:
        await client.aclose()
