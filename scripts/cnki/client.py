"""Client for CNKI overseas journal metadata pages."""

from __future__ import annotations

import asyncio
import html
import json
import os
import re
import unicodedata
from typing import Any
from urllib.parse import urlencode, urljoin

import httpx

from scripts.shared.cnki_urls import (
    CNKI_CHINESE_LANGUAGE,
    with_cnki_chinese_language,
)
from scripts.shared.request_pools import (
    build_async_client_pool,
    build_proxy_pool,
    close_async_client_pool,
    request_pool_key,
    select_async_client,
)

BASE_URL = "https://oversea.cnki.net"
JOURNAL_PRODUCT_CODE = "BOJHD70J"
DEFAULT_PCODE = "CJFD,CCJD"
CNKI_REQUEST_ATTEMPTS = 3
CNKI_RETRY_BASE_SECONDS = 1.0
DEFAULT_HEADERS = {
    "User-Agent": (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/124.0.0.0 Safari/537.36"
    ),
    "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.5",
}
ATTRIBUTE_RE = re.compile(
    r"([:\w-]+)\s*=\s*(\"[^\"]*\"|'[^']*'|[^\s>]+)",
    re.S,
)
TAG_RE = re.compile(r"<[^>]+>")


class CnkiRequestError(RuntimeError):
    """Raised when CNKI returns a blocked or unusable response."""


class CnkiClient:
    """Fetch journal and article metadata from CNKI overseas."""

    def __init__(self, timeout: int = 20) -> None:
        """
        Initialize the CNKI client.

        Args:
            timeout: HTTP request timeout in seconds.
        """
        self.proxy_pool = build_proxy_pool(os.getenv("PROXY_POOL"))
        self._clients = build_async_client_pool(
            timeout=timeout,
            follow_redirects=False,
            headers=DEFAULT_HEADERS,
            proxy_pool=self.proxy_pool,
        )

    async def aclose(self) -> None:
        """
        Close HTTP resources.

        Returns:
            None.
        """
        await close_async_client_pool(self._clients)

    async def resolve_journal(self, row: dict[str, str]) -> dict[str, Any] | None:
        """
        Resolve one CSV journal row to a CNKI journal detail payload.

        Args:
            row: Source CSV row.

        Returns:
            CNKI journal detail payload or None.
        """
        issn = (row.get("issn") or "").strip()
        title = (row.get("title") or "").strip()
        if title:
            details = await self._resolve_search_results(
                await self.search_journals(
                    field_name="TI",
                    value=title,
                    operator="%",
                    search_type="刊名(曾用刊名)",
                ),
                title,
                issn,
            )
            if details:
                return details

        if issn:
            return await self._resolve_search_results(
                await self.search_journals(
                    field_name="SN",
                    value=issn,
                    operator="=",
                    search_type="ISSN",
                ),
                title,
                issn,
            )
        return None

    async def _resolve_search_results(
        self,
        candidates: list[dict[str, Any]],
        title: str,
        issn: str,
    ) -> dict[str, Any] | None:
        """
        Resolve search candidates to a matching CNKI journal detail payload.

        Args:
            candidates: CNKI search result candidates.
            title: Expected journal title.
            issn: Expected journal ISSN.

        Returns:
            Matching CNKI journal detail payload or None.
        """
        for candidate in candidates:
            details = await self.get_journal_detail(str(candidate["detail_url"]))
            if details and _journal_detail_matches(details, title, issn):
                return details
        return None

    async def search_journals(
        self,
        field_name: str,
        value: str,
        operator: str,
        search_type: str,
    ) -> list[dict[str, Any]]:
        """
        Search CNKI overseas journals.

        Args:
            field_name: CNKI search field name.
            value: Search value.
            operator: CNKI search operator.
            search_type: Human-readable CNKI search type.

        Returns:
            Search result candidates.
        """
        state = _journal_search_state(field_name, value, operator)
        payload = {
            "searchStateJson": json.dumps(state, ensure_ascii=False),
            "displaymode": 1,
            "pageindex": 1,
            "pagecount": 21,
            "index": "",
            "searchType": search_type,
            "parentcode": "",
            "clickName": "",
            "switchdata": "search",
        }
        text = await self._post_text(
            f"{BASE_URL}/knavi/journals/searchbaseinfo",
            data=payload,
            referer=f"{BASE_URL}/knavi",
        )
        return _parse_journal_search_results(text)

    async def get_journal_detail(self, detail_url: str) -> dict[str, Any] | None:
        """
        Fetch and parse one CNKI overseas journal detail page.

        Args:
            detail_url: CNKI journal detail URL.

        Returns:
            Journal detail payload or None.
        """
        resolved_url = with_cnki_chinese_language(detail_url)
        text = await self._get_text(resolved_url)
        pykm = _input_value(text, "pykm")
        if not pykm:
            return None
        pcode = _input_value(text, "pCode") or DEFAULT_PCODE
        token = _input_value(text, "time")
        visible_text = _strip_tags(text)
        return {
            "detail_url": resolved_url,
            "pykm": pykm,
            "pcode": pcode,
            "time": token,
            "title": _input_value(text, "shareChName") or _title_text(text),
            "issn": _regex_group(r"ISSN\s*[:：]\s*([0-9Xx-]+)", visible_text),
            "cn": _regex_group(r"CN\s*[:：]\s*([0-9A-Za-z/-]+)", visible_text),
            "impact_factor": _regex_group(
                r"(?:复合影响因子|Combined IF)\s*[:：]\s*([0-9.]+)",
                visible_text,
            ),
            "cover_url": _image_url(text),
            "raw_text": visible_text,
        }

    async def get_year_issues(self, journal: dict[str, Any]) -> list[dict[str, Any]]:
        """
        Fetch publication issues for one CNKI journal.

        Args:
            journal: CNKI journal detail payload.

        Returns:
            Issue payloads in upstream order.
        """
        pykm = str(journal["pykm"])
        payload = {
            "pIdx": 0,
            "time": str(journal.get("time") or ""),
            "isEpublish": "",
            "pcode": str(journal.get("pcode") or DEFAULT_PCODE),
        }
        text = await self._post_text(
            f"{BASE_URL}/knavi/journals/{pykm}/yearList",
            data=payload,
            referer=str(journal["detail_url"]),
        )
        return _parse_year_issues(text)

    async def get_issue_articles(
        self,
        journal: dict[str, Any],
        issue: dict[str, Any],
    ) -> list[dict[str, Any]]:
        """
        Fetch article summaries for one CNKI issue.

        Args:
            journal: CNKI journal detail payload.
            issue: CNKI issue payload.

        Returns:
            Article summary payloads.
        """
        pykm = str(journal["pykm"])
        params = {
            "yearIssue": str(issue["year_issue"]),
            "pageIdx": 0,
            "pcode": str(journal.get("pcode") or DEFAULT_PCODE),
            "isEpublish": "",
            "language": CNKI_CHINESE_LANGUAGE,
        }
        text = await self._post_text(
            f"{BASE_URL}/knavi/journals/{pykm}/papers",
            data={},
            params=params,
            referer=str(journal["detail_url"]),
        )
        return _parse_issue_articles(text, issue)

    async def get_article_detail(self, article_url: str) -> dict[str, Any]:
        """
        Fetch and parse one CNKI article detail page.

        Args:
            article_url: CNKI article abstract URL.

        Returns:
            Article detail payload.
        """
        resolved_url = with_cnki_chinese_language(article_url)
        text = await self._get_text(resolved_url, referer=BASE_URL)
        filename = _input_value(text, "paramfilename") or _input_value(
            text, "param-filename"
        )
        dbcode = _input_value(text, "paramdbcode") or _input_value(text, "param-dbcode")
        dbname = _input_value(text, "paramdbname") or _input_value(text, "param-dbname")
        visible_text = _strip_tags(text)
        title = _first_block_text(
            text, r'<p\s+class="title-one"[^>]*>(.*?)</p>'
        ) or _title_text(text)
        authors = _author_text(text) or None
        page_range = _regex_group(r"页码\s*[:：]\s*([0-9A-Za-z\-–—]+)", visible_text)
        online_time = _row_value(text, "在线公开时间") or _row_value(
            text, "Online Release Time"
        )
        doi = _row_value(text, "DOI")
        permalink = _openlink_url(dbcode, dbname, filename) or resolved_url
        return {
            "article_url": resolved_url,
            "platform_id": filename,
            "dbcode": dbcode,
            "dbname": dbname,
            "title": title,
            "authors": authors,
            "abstract": _input_value(text, "abstract_text"),
            "doi": doi,
            "online_release_date": _date_part(online_time),
            "pages": page_range,
            "html_read_url": _link_with_text(text, "HTML阅读"),
            "permalink": permalink,
            "content_location": permalink,
        }

    async def _get_text(self, url: str, referer: str | None = None) -> str:
        """
        Send a GET request and return response text.

        Args:
            url: Request URL.
            referer: Optional referer header.

        Returns:
            Response text.
        """
        headers = _request_headers(referer)
        return await self._request_text_with_retries("GET", url, headers=headers)

    async def _post_text(
        self,
        url: str,
        data: dict[str, Any],
        referer: str | None = None,
        params: dict[str, Any] | None = None,
    ) -> str:
        """
        Send a POST request and return response text.

        Args:
            url: Request URL.
            data: Form data.
            referer: Optional referer header.
            params: Optional query parameters.

        Returns:
            Response text.
        """
        headers = _request_headers(referer)
        return await self._request_text_with_retries(
            "POST",
            url,
            data=data,
            params=params,
            headers=headers,
        )

    async def _request_text_with_retries(
        self,
        method: str,
        url: str,
        data: dict[str, Any] | None = None,
        params: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
    ) -> str:
        """
        Send a request with proxy failover and return response text.

        Args:
            method: HTTP method.
            url: Request URL.
            data: Optional form data.
            params: Optional query parameters.
            headers: Optional request headers.

        Returns:
            Response text.
        """
        request_key = request_pool_key(url, params)
        total_attempts = max(CNKI_REQUEST_ATTEMPTS, len(self._clients))
        for attempt in range(total_attempts):
            client = select_async_client(self._clients, request_key, attempt)
            try:
                response = await client.request(
                    method,
                    url,
                    data=data,
                    params=params,
                    headers=headers,
                )
                return _checked_text(response, url)
            except (httpx.TransportError, CnkiRequestError):
                if attempt >= total_attempts - 1:
                    raise
                await asyncio.sleep(_cnki_retry_delay(attempt, len(self._clients)))
        raise RuntimeError("CNKI retry loop exited unexpectedly.")


def _journal_search_state(field_name: str, value: str, operator: str) -> dict[str, Any]:
    """
    Build the CNKI journal search state payload.

    Args:
        field_name: CNKI search field name.
        value: Search value.
        operator: CNKI search operator.

    Returns:
        Search state payload.
    """
    return {
        "StateID": "",
        "Platfrom": "",
        "QueryTime": "",
        "Account": "knavi",
        "ClientToken": "",
        "Language": "",
        "CNode": {"PCode": JOURNAL_PRODUCT_CODE, "SMode": "", "OperateT": 0},
        "QNode": {
            "SelectT": "",
            "Select_Fields": "",
            "S_DBCodes": "",
            "Subscribed": "",
            "QGroup": [
                {
                    "Key": "subject",
                    "Logic": 1,
                    "Items": [
                        {
                            "Key": "txt_1",
                            "Title": "",
                            "Logic": 1,
                            "Name": field_name,
                            "Operate": operator,
                            "Value": value,
                            "ExtendType": 0,
                            "ExtendValue": "",
                            "Value2": "",
                        }
                    ],
                    "ChildItems": [],
                }
            ],
            "OrderBy": "OTA|DESC",
            "GroupBy": "",
            "Additon": "",
        },
    }


def _parse_journal_search_results(text: str) -> list[dict[str, Any]]:
    """
    Parse CNKI journal search results.

    Args:
        text: Search response HTML.

    Returns:
        Search result candidates.
    """
    candidates: list[dict[str, Any]] = []
    seen: set[str] = set()
    pattern = re.compile(
        r'<a[^>]+href="([^"]*?/knavi/detail\?[^"]+)"[^>]*>(.*?)</a>',
        re.S,
    )
    for match in pattern.finditer(text):
        detail_url = html.unescape(urljoin(BASE_URL, match.group(1)))
        if detail_url in seen:
            continue
        seen.add(detail_url)
        candidates.append(
            {
                "detail_url": detail_url,
                "title": _strip_tags(match.group(2)),
            }
        )
    return candidates


def _journal_detail_matches(details: dict[str, Any], title: str, issn: str) -> bool:
    """
    Check whether a CNKI journal detail payload matches a source CSV row.

    Args:
        details: CNKI journal detail payload.
        title: Expected journal title.
        issn: Expected journal ISSN.

    Returns:
        Whether the detail payload matches the source row.
    """
    detail_title = str(details.get("title") or "")
    if title:
        return _journal_titles_match(title, detail_title) or _journal_title_in_text(
            title, str(details.get("raw_text") or "")
        )
    return bool(issn and _normalize_issn(issn) == _normalize_issn(details.get("issn")))


def _journal_titles_match(expected: str, actual: str) -> bool:
    """
    Compare journal titles with punctuation and width normalization.

    Args:
        expected: Expected source CSV title.
        actual: CNKI detail title.

    Returns:
        Whether the titles refer to the same journal.
    """
    normalized_expected = _normalize_journal_title(expected)
    normalized_actual = _normalize_journal_title(actual)
    if not normalized_expected or not normalized_actual:
        return False
    return normalized_expected == normalized_actual


def _journal_title_in_text(expected: str, text: str) -> bool:
    """
    Check whether a journal title appears as a separated title token.

    Args:
        expected: Expected source CSV title.
        text: CNKI visible detail text.

    Returns:
        Whether the title appears as its own separated token.
    """
    normalized_expected = _normalize_journal_title(expected)
    if not normalized_expected:
        return False
    for part in re.split(r"[\s,;:，；：、|/\\()（）《》〈〉“”‘’]+", text):
        if _normalize_journal_title(part) == normalized_expected:
            return True
    return False


def _normalize_journal_title(value: str | None) -> str:
    """
    Normalize a journal title for CNKI matching.

    Args:
        value: Raw title.

    Returns:
        Normalized title.
    """
    text = unicodedata.normalize("NFKC", _clean_text(value) or "").casefold()
    return re.sub(r"[\s\"'.,:;!?()\[\]{}<>《》〈〉“”‘’·\-–—_/\\]+", "", text)


def _normalize_issn(value: Any) -> str:
    """
    Normalize an ISSN for exact comparison.

    Args:
        value: Raw ISSN value.

    Returns:
        Normalized ISSN.
    """
    return re.sub(r"[^0-9Xx]", "", str(value or "")).upper()


def _parse_year_issues(text: str) -> list[dict[str, Any]]:
    """
    Parse CNKI year issue tree HTML.

    Args:
        text: Year issue tree HTML.

    Returns:
        Issue payloads.
    """
    issues: list[dict[str, Any]] = []
    for match in re.finditer(r"<a\b[^>]*>(.*?)</a>", text, re.S):
        tag = match.group(0)
        attrs = _attrs(tag)
        element_id = attrs.get("id", "")
        if not element_id.startswith("yq"):
            continue
        key = element_id[2:]
        year = _int_or_none(key[:4])
        if year is None:
            continue
        label = _strip_tags(match.group(1))
        number = _issue_number(key, label)
        year_issue = attrs.get("value")
        if not year_issue:
            continue
        issues.append(
            {
                "year": year,
                "number": number,
                "title": label,
                "year_issue": html.unescape(year_issue),
            }
        )
    return issues


def _parse_issue_articles(
    text: str,
    issue: dict[str, Any],
) -> list[dict[str, Any]]:
    """
    Parse article rows from one CNKI issue HTML response.

    Args:
        text: Issue article HTML.
        issue: Issue payload.

    Returns:
        Article summary payloads.
    """
    articles: list[dict[str, Any]] = []
    current_section = ""
    block_pattern = re.compile(
        r'<dt\b[^>]*class="[^"]*\btit\b[^"]*"[^>]*>(.*?)</dt>|'
        r'(<dd\b[^>]*class="[^"]*\brow\b[^"]*"[^>]*>.*?</dd>)',
        re.S,
    )
    for match in block_pattern.finditer(text):
        if match.group(1) is not None:
            current_section = _strip_tags(match.group(1))
            continue
        row_html = match.group(2) or ""
        article = _parse_article_row(row_html, issue, current_section)
        if article:
            articles.append(article)
    return articles


def _parse_article_row(
    row_html: str,
    issue: dict[str, Any],
    section: str,
) -> dict[str, Any] | None:
    """
    Parse one CNKI issue article row.

    Args:
        row_html: Article row HTML.
        issue: Issue payload.
        section: Current issue section.

    Returns:
        Article summary payload or None.
    """
    link_match = re.search(
        r'<a[^>]+href="([^"]*?/kcms2/article/abstract\?[^"]+)"[^>]*>(.*?)</a>',
        row_html,
        re.S,
    )
    if not link_match:
        return None
    article_url = with_cnki_chinese_language(
        urljoin(BASE_URL, html.unescape(link_match.group(1)))
    )
    platform_id = _regex_group(
        r'<b[^>]+name=["\']encrypt["\'][^>]+id=["\']([^"\']+)["\']',
        row_html,
    )
    authors = _span_title(row_html, "author")
    pages = _span_title(row_html, "company")
    return {
        "article_url": article_url,
        "platform_id": platform_id,
        "title": _strip_tags(link_match.group(2)),
        "authors": authors,
        "pages": pages,
        "section": section,
        "is_free": 1 if "免费" in _strip_tags(row_html) or "Free" in row_html else 0,
        "date": f"{int(issue['year']):04d}-01-01",
    }


def _attrs(tag: str) -> dict[str, str]:
    """
    Parse HTML tag attributes.

    Args:
        tag: Raw HTML tag.

    Returns:
        Attribute mapping.
    """
    attrs: dict[str, str] = {}
    for key, value in ATTRIBUTE_RE.findall(tag):
        attrs[key.lower()] = html.unescape(value.strip("\"'"))
    return attrs


def _input_value(text: str, element_id: str) -> str | None:
    """
    Read a hidden input value by id.

    Args:
        text: HTML text.
        element_id: Input id.

    Returns:
        Input value or None.
    """
    for match in re.finditer(r"<input\b[^>]*>", text, re.S):
        attrs = _attrs(match.group(0))
        if attrs.get("id") == element_id:
            value = attrs.get("value", "")
            return value.strip() or None
    return None


def _span_title(text: str, class_name: str) -> str | None:
    """
    Read the title attribute from a span class.

    Args:
        text: HTML text.
        class_name: Span class name.

    Returns:
        Attribute value or None.
    """
    pattern = re.compile(
        rf'<span\b[^>]*class="[^"]*\b{re.escape(class_name)}\b[^"]*"[^>]*>',
        re.S,
    )
    match = pattern.search(text)
    if not match:
        return None
    value = _attrs(match.group(0)).get("title")
    return _clean_text(value)


def _first_block_text(text: str, pattern: str) -> str | None:
    """
    Extract and clean the first regex capture block.

    Args:
        text: HTML text.
        pattern: Regex pattern with one capture group.

    Returns:
        Clean block text or None.
    """
    match = re.search(pattern, text, re.S)
    if not match:
        return None
    return _strip_tags(match.group(1))


def _author_text(text: str) -> str | None:
    """
    Parse article author names from the detail page.

    Args:
        text: Article detail HTML.

    Returns:
        Semicolon-delimited author names or None.
    """
    block = _regex_group(
        r'<h3\b[^>]*class="[^"]*\bauthor\b[^"]*"[^>]*id="authorpart"[^>]*>(.*?)</h3>',
        text,
    )
    if not block:
        return None
    names = [
        _strip_tags(match.group(1))
        for match in re.finditer(r"<span\b[^>]*>(.*?)</span>", block, re.S)
    ]
    cleaned = [name for name in names if name]
    return "; ".join(cleaned) if cleaned else None


def _row_value(text: str, label: str) -> str | None:
    """
    Read a CNKI detail row value by label.

    Args:
        text: Article detail HTML.
        label: Row label.

    Returns:
        Row value or None.
    """
    pattern = re.compile(
        rf'<span\b[^>]*class="[^"]*\browtit\b[^"]*"[^>]*>\s*'
        rf"{re.escape(label)}\s*[:：]?\s*</span>\s*<p[^>]*>(.*?)</p>",
        re.S,
    )
    match = pattern.search(text)
    if not match:
        return None
    return _strip_tags(match.group(1))


def _link_with_text(text: str, label: str) -> str | None:
    """
    Return the first link that contains visible label text.

    Args:
        text: HTML text.
        label: Visible link label.

    Returns:
        Absolute URL or None.
    """
    for match in re.finditer(r'<a\b[^>]+href="([^"]+)"[^>]*>(.*?)</a>', text, re.S):
        if label not in _strip_tags(match.group(2)):
            continue
        return with_cnki_chinese_language(
            urljoin(BASE_URL, html.unescape(match.group(1)))
        )
    return None


def _openlink_url(
    dbcode: str | None, dbname: str | None, filename: str | None
) -> str | None:
    """
    Build a stable CNKI openlink URL.

    Args:
        dbcode: CNKI database code.
        dbname: CNKI database name.
        filename: CNKI article filename.

    Returns:
        Stable openlink URL or None.
    """
    if not dbcode or not dbname or not filename:
        return None
    query = urlencode(
        {
            "dbcode": dbcode,
            "dbname": dbname,
            "filename": filename,
            "uniplatform": "OVERSEA",
            "language": CNKI_CHINESE_LANGUAGE,
        }
    )
    return f"{BASE_URL}/openlink/detailen?{query}"


def _checked_text(response: httpx.Response, url: str) -> str:
    """
    Validate a CNKI response and return text.

    Args:
        response: HTTP response.
        url: Request URL.

    Returns:
        Response text.

    Raises:
        CnkiRequestError: If CNKI returns a blocked or invalid response.
    """
    if response.status_code in {403, 429}:
        raise CnkiRequestError(f"CNKI blocked request {response.status_code}: {url}")
    if 300 <= response.status_code < 400:
        location = response.headers.get("location") or ""
        raise CnkiRequestError(
            f"CNKI redirected request {response.status_code}: {location}"
        )
    if response.status_code >= 400:
        raise CnkiRequestError(f"CNKI request failed {response.status_code}: {url}")
    text = response.text
    lowered = text.lower()
    if (
        "captcha" in lowered or "访问异常" in text or "安全验证" in text
    ) and not _looks_like_cnki_content(text):
        raise CnkiRequestError(f"CNKI verification required: {url}")
    return text


def _cnki_retry_delay(attempt: int, client_count: int) -> float:
    """
    Calculate retry delay while prioritizing unused proxy failover.

    Args:
        attempt: Zero-based retry attempt.
        client_count: HTTP client pool size.

    Returns:
        Delay in seconds.
    """
    if attempt + 1 < client_count:
        return 0.0
    return CNKI_RETRY_BASE_SECONDS * (attempt + 1)


def _looks_like_cnki_content(text: str) -> bool:
    """
    Detect whether a response still contains expected CNKI content.

    Args:
        text: Response text.

    Returns:
        Whether the response appears to be usable CNKI content.
    """
    markers = (
        'id="abstract_text"',
        'id="pykm"',
        'id="YearIssueTree"',
        'class="name"',
        "/knavi/detail?",
    )
    return any(marker in text for marker in markers)


def _request_headers(referer: str | None) -> dict[str, str]:
    """
    Build request headers for CNKI AJAX calls.

    Args:
        referer: Optional referer URL.

    Returns:
        Request headers.
    """
    headers = {"X-Requested-With": "XMLHttpRequest"}
    if referer:
        headers["Referer"] = referer
    return headers


def _issue_number(key: str, label: str) -> str:
    """
    Extract a stable issue number from a CNKI year issue key.

    Args:
        key: CNKI year issue key.
        label: Visible issue label.

    Returns:
        Issue number.
    """
    suffix = key[4:]
    if suffix:
        return suffix.lstrip("0") or "0"
    match = re.search(r"([0-9]+|S[0-9]+)", label, re.I)
    return match.group(1) if match else label


def _image_url(text: str) -> str | None:
    """
    Extract a journal cover image URL.

    Args:
        text: Journal detail HTML.

    Returns:
        Absolute image URL or None.
    """
    for match in re.finditer(r"<img\b[^>]+>", text, re.S):
        attrs = _attrs(match.group(0))
        source = attrs.get("src")
        if not source:
            continue
        if "cover" in source.lower() or "journal" in source.lower():
            return urljoin(BASE_URL, source)
    return None


def _title_text(text: str) -> str | None:
    """
    Extract a page title.

    Args:
        text: HTML text.

    Returns:
        Clean title or None.
    """
    title = _first_block_text(text, r"<title>(.*?)</title>")
    if not title:
        return None
    return re.sub(r"\s*-\s*中国知网\s*$", "", title).strip() or title


def _strip_tags(value: str | None) -> str:
    """
    Strip HTML tags and normalize whitespace.

    Args:
        value: Raw HTML text.

    Returns:
        Clean text.
    """
    if not value:
        return ""
    text = TAG_RE.sub(" ", value)
    return _clean_text(html.unescape(text)) or ""


def _clean_text(value: str | None) -> str | None:
    """
    Normalize whitespace in text.

    Args:
        value: Raw text.

    Returns:
        Clean text or None.
    """
    if value is None:
        return None
    text = re.sub(r"\s+", " ", html.unescape(str(value))).strip()
    return text or None


def _regex_group(pattern: str, text: str) -> str | None:
    """
    Return the first regex capture group.

    Args:
        pattern: Regex pattern with one capture group.
        text: Source text.

    Returns:
        Captured value or None.
    """
    match = re.search(pattern, text, re.S)
    if not match:
        return None
    return _clean_text(match.group(1))


def _int_or_none(value: str) -> int | None:
    """
    Convert text to an integer when possible.

    Args:
        value: Raw text.

    Returns:
        Integer or None.
    """
    try:
        return int(value)
    except ValueError:
        return None


def _date_part(value: str | None) -> str | None:
    """
    Return the date portion from a datetime value.

    Args:
        value: Raw datetime value.

    Returns:
        Date string or None.
    """
    text = _clean_text(value)
    if not text:
        return None
    match = re.search(r"\d{4}-\d{2}-\d{2}", text)
    return match.group(0) if match else None
