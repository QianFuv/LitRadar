"""CNKI URL helpers."""

from __future__ import annotations

from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

CNKI_OVERSEA_HOST = "oversea.cnki.net"
CNKI_CHINESE_LANGUAGE = "CHS"
CNKI_DETAIL_PATH = "/openlink/detail"
CNKI_OPENLINK_DETAIL_EN_PATH = "/openlink/detailen"
CNKI_PATH_PREFIXES = ("/kcms", "/knavi", "/openlink")


def is_cnki_oversea_url(url: str) -> bool:
    """
    Check whether a URL points to CNKI overseas.

    Args:
        url: URL to check.

    Returns:
        True when the URL is an overseas CNKI URL.
    """
    text = str(url).strip()
    if not text:
        return False
    parts = urlsplit(text)
    if parts.netloc:
        return (parts.hostname or "").lower() == CNKI_OVERSEA_HOST
    return parts.path.lower().startswith(CNKI_PATH_PREFIXES)


def with_cnki_chinese_language(url: str) -> str:
    """
    Force a CNKI overseas URL to use the Chinese interface.

    Args:
        url: URL to normalize.

    Returns:
        URL with CNKI overseas Chinese language parameters.
    """
    text = str(url).strip()
    if not text:
        return text
    parts = urlsplit(text)
    if parts.netloc and not is_cnki_oversea_url(text):
        return text
    normalized_path = parts.path.lower()
    if not parts.netloc and not is_cnki_oversea_url(text):
        return text
    path = (
        CNKI_DETAIL_PATH
        if normalized_path == CNKI_OPENLINK_DETAIL_EN_PATH
        else parts.path
    )
    query = {
        key: value
        for key, value in parse_qsl(parts.query, keep_blank_values=True)
        if key.lower() not in {"language", "uniplatform"}
    }
    query["uniplatform"] = "OVERSEA"
    query["language"] = CNKI_CHINESE_LANGUAGE
    return urlunsplit(
        (
            parts.scheme,
            parts.netloc,
            path,
            urlencode(query),
            parts.fragment,
        )
    )
