"""CNKI URL helpers."""

from __future__ import annotations

from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

CNKI_OVERSEA_HOST = "oversea.cnki.net"
CNKI_CHINESE_LANGUAGE = "chs"


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
    if parts.netloc and parts.netloc.lower() != CNKI_OVERSEA_HOST:
        return text
    if not parts.netloc and not parts.path.startswith(("/kcms", "/knavi", "/openlink")):
        return text
    query = dict(parse_qsl(parts.query, keep_blank_values=True))
    query["uniplatform"] = "OVERSEA"
    query["language"] = CNKI_CHINESE_LANGUAGE
    return urlunsplit(
        (
            parts.scheme,
            parts.netloc,
            parts.path,
            urlencode(query),
            parts.fragment,
        )
    )
