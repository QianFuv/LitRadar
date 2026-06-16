"""Client for Zhejiang Library mediated CNKI full-text access."""

from __future__ import annotations

import base64
import codecs
import html
import json
import re
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import datetime
from http.cookiejar import Cookie
from typing import Any
from urllib.parse import quote, unquote_plus, urlencode, urljoin, urlparse, urlunparse

import httpx

from paper_scanner.sources.zjlib_cnki.matching import (
    ArticleIdentity,
    does_article_metadata_match,
)

WWW_BASE_URL = "https://www.zjlib.cn"
SHARE_BASE_URL = "https://share.zjlib.cn"
ZYPROXY_BASE_URL = "https://http-10--18--17--173.elib.zyproxy.zjlib.cn"
ENTRY_URL = f"{SHARE_BASE_URL}/entry/area/35594/2120"
LIBRARY_REFER = "http://10.18.17.173/kns55/"
WFWFID = "2120"
BFF_ORG_ID = "1916318653650423810"
DEFAULT_TIMEOUT = 30.0
TOKEN_EXPIRY_SKEW_SECONDS = 300
DEFAULT_HEADERS = {
    "User-Agent": (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/148.0.0.0 Safari/537.36"
    ),
    "Accept-Language": "zh-CN;q=0.9",
}
TAG_RE = re.compile(r"<[^>]+>")
HREF_RE = re.compile(r"<a\b[^>]+href\s*=\s*([\"'])(.*?)\1[^>]*>(.*?)</a>", re.I | re.S)
ATTRIBUTE_RE = re.compile(r"([:\w-]+)\s*=\s*(\"[^\"]*\"|'[^']*'|[^\s>]+)", re.S)


class ZjlibCnkiError(RuntimeError):
    """Raised when the Zhejiang Library CNKI flow cannot proceed."""


@dataclass(frozen=True)
class QrLogin:
    """QR login challenge returned by Zhejiang Library."""

    uuid: str
    status: str
    qr_code: str


@dataclass(frozen=True)
class SearchResult:
    """One CNKI result row."""

    index: int
    title: str
    detail_url: str
    file_name: str | None
    db_name: str | None
    db_code: str | None
    download_url: str | None = None


@dataclass(frozen=True)
class CnkiArticleCandidate:
    """CNKI candidate metadata parsed before PDF download."""

    result: SearchResult
    identity: ArticleIdentity
    detail_url: str
    pdf_url: str | None


@dataclass(frozen=True)
class DownloadedPdf:
    """Downloaded PDF bytes and response metadata."""

    filename: str
    final_url: str
    content_type: str
    byte_count: int
    content: bytes


@dataclass(frozen=True)
class ClientInfo:
    """Local client/session metadata safe to return through APIs."""

    has_bff_user_token: bool
    bff_user_token_exp: int | None
    bff_user_token_expires_at: str | None
    bff_user_token_seconds_remaining: int | None
    cookie_names: list[str]


class ZhejiangLibraryCnkiClient:
    """Run the Zhejiang Library, Share, zyproxy, and CNKI full-text flow."""

    def __init__(
        self,
        *,
        state_data: Mapping[str, Any] | None = None,
        timeout: float = DEFAULT_TIMEOUT,
        client: httpx.Client | None = None,
    ) -> None:
        """
        Initialize the client.

        Args:
            state_data: Optional persisted session state.
            timeout: HTTP timeout in seconds.
            client: Optional preconfigured HTTP client for tests.
        """
        self.bff_user_token: str | None = None
        self.qr_uuid: str | None = None
        self.last_brief_url: str | None = None
        self._owns_client = client is None
        self.client = client or httpx.Client(
            timeout=timeout,
            follow_redirects=False,
            headers=DEFAULT_HEADERS,
        )
        if state_data:
            self.load_state_data(state_data)

    def close(self) -> None:
        """
        Close HTTP resources owned by this client.

        Returns:
            None.
        """
        if self._owns_client:
            self.client.close()

    def __enter__(self) -> ZhejiangLibraryCnkiClient:
        """
        Enter a context-managed client.

        Returns:
            This client instance.
        """
        return self

    def __exit__(self, *_exc: object) -> None:
        """
        Exit a context-managed client.

        Returns:
            None.
        """
        self.close()

    def load_state_data(self, state: Mapping[str, Any]) -> None:
        """
        Load persisted token, QR UUID, and cookies from JSON-like data.

        Args:
            state: Persisted state mapping.

        Returns:
            None.
        """
        self.bff_user_token = str(state.get("bff_user_token") or "") or None
        self.qr_uuid = str(state.get("qr_uuid") or "") or None
        self.client.cookies.clear()
        for cookie_data in state.get("cookies") or []:
            if isinstance(cookie_data, Mapping):
                self.client.cookies.jar.set_cookie(_cookie_from_json(cookie_data))

    def to_state_data(self) -> dict[str, Any]:
        """
        Return JSON-serializable session state for server-side persistence.

        Returns:
            State data containing token, QR UUID, cookies, and save timestamp.
        """
        return {
            "bff_user_token": self.bff_user_token,
            "qr_uuid": self.qr_uuid,
            "cookies": [_cookie_to_json(cookie) for cookie in self.client.cookies.jar],
            "saved_at": int(time.time()),
        }

    def client_info(self) -> ClientInfo:
        """
        Return session metadata without exposing token or cookie values.

        Returns:
            Safe session metadata.
        """
        exp = _jwt_exp(self.bff_user_token)
        now = int(time.time())
        cookie_names = sorted({cookie.name for cookie in self.client.cookies.jar})
        return ClientInfo(
            has_bff_user_token=bool(self.bff_user_token),
            bff_user_token_exp=exp,
            bff_user_token_expires_at=(
                datetime.fromtimestamp(exp).isoformat(timespec="seconds")
                if exp
                else None
            ),
            bff_user_token_seconds_remaining=max(0, exp - now) if exp else None,
            cookie_names=cookie_names,
        )

    def start_qr_login(self) -> QrLogin:
        """
        Start Zhejiang Library QR login.

        Returns:
            QR login challenge data.
        """
        response = self.client.get(
            f"{WWW_BASE_URL}/bff-api/reader-sso-service/portal-pc-api/login/zfb-qr",
            headers=_www_headers(),
        )
        payload = _json_payload(response, "start QR login")
        data = _payload_data(payload, "start QR login")
        uuid = str(data.get("uuid") or "")
        qr_code = str(data.get("qrCode") or "")
        status = str(data.get("status") or "")
        if not uuid or not qr_code:
            raise ZjlibCnkiError("QR login response did not contain uuid/qrCode.")
        self.qr_uuid = uuid
        return QrLogin(uuid=uuid, status=status, qr_code=qr_code)

    def poll_qr_login(
        self,
        *,
        uuid: str | None = None,
        timeout_seconds: int = 180,
        interval_seconds: float = 2.0,
        on_status: Callable[[str], None] | None = None,
    ) -> str:
        """
        Poll QR login until a bff-user-token is available.

        Args:
            uuid: Optional QR UUID override.
            timeout_seconds: Maximum polling duration in seconds.
            interval_seconds: Delay between status checks.
            on_status: Optional callback for status changes.

        Returns:
            Completed bff-user-token.
        """
        qr_uuid = uuid or self.qr_uuid
        if not qr_uuid:
            raise ZjlibCnkiError("No QR uuid available. Run start-login first.")

        deadline = time.monotonic() + timeout_seconds
        last_status = ""
        while time.monotonic() < deadline:
            response = self.client.get(
                f"{WWW_BASE_URL}/bff-api/reader-sso-service/portal-pc-api/qr/status",
                params={"uuid": qr_uuid},
                headers=_www_headers(),
            )
            payload = _json_payload(response, "poll QR login")
            data = _payload_data(payload, "poll QR login")
            status = str(data.get("status") or "")
            if status != last_status:
                last_status = status
                if on_status:
                    on_status(status)
            if status == "COMPLETE":
                token = str(data.get("data") or "")
                if not token:
                    raise ZjlibCnkiError("QR login completed but did not return token.")
                self.bff_user_token = token
                self.client.cookies.set(
                    "userToken", token, domain="www.zjlib.cn", path="/"
                )
                return token
            if status in {"EXPIRED", "CANCEL", "CANCELED", "FAIL", "FAILED"}:
                raise ZjlibCnkiError(f"QR login ended with status {status}.")
            time.sleep(interval_seconds)
        raise ZjlibCnkiError(
            f"Timed out waiting for QR scan after {timeout_seconds} seconds."
        )

    def ensure_logged_in(
        self,
        *,
        timeout_seconds: int = 180,
        on_status: Callable[[str], None] | None = None,
    ) -> str:
        """
        Return a usable bff-user-token.

        Args:
            timeout_seconds: Maximum QR polling duration if no token exists.
            on_status: Optional login status callback.

        Returns:
            Usable bff-user-token.
        """
        if self.bff_user_token:
            exp = _jwt_exp(self.bff_user_token)
            if exp is not None and exp <= int(time.time()) + TOKEN_EXPIRY_SKEW_SECONDS:
                raise ZjlibCnkiError(
                    "bff-user-token is expired or expires within "
                    f"{TOKEN_EXPIRY_SKEW_SECONDS} seconds. Run QR login again."
                )
            return self.bff_user_token
        return self.poll_qr_login(timeout_seconds=timeout_seconds, on_status=on_status)

    def build_share_sso_url(self, refer_url: str = ENTRY_URL) -> str:
        """
        Build the Share SSO URL from the Zhejiang Library token.

        Args:
            refer_url: Share entry URL.

        Returns:
            Share SSO URL.
        """
        token = self.ensure_logged_in()
        response = self.client.get(
            f"{WWW_BASE_URL}/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl",
            params={"referURL": refer_url},
            headers=_www_headers(token),
        )
        payload = _json_payload(response, "build Share SSO URL")
        sso_url = str(payload.get("data") or "")
        if not sso_url.startswith(SHARE_BASE_URL):
            raise ZjlibCnkiError(
                "Share SSO URL response did not contain a share.zjlib.cn URL."
            )
        return sso_url

    def enter_share(self, sso_url: str, entry_url: str = ENTRY_URL) -> None:
        """
        Enter Share through protocolAuth and synchronize login cookies.

        Args:
            sso_url: Share SSO URL.
            entry_url: Share entry URL.

        Returns:
            None.
        """
        response = self.client.get(
            sso_url,
            headers=_html_headers(referer=WWW_BASE_URL + "/"),
            follow_redirects=False,
        )
        _raise_for_status(response, "enter Share protocolAuth")
        sync = _extract_share_cookie_sync(response.text)
        if sync:
            sync_url, data = sync
            response = self.client.post(
                sync_url,
                data=data,
                headers={
                    **_html_headers(referer=str(response.url)),
                    "Origin": SHARE_BASE_URL,
                },
                follow_redirects=True,
            )
            _raise_for_status(response, "sync Share login cookies")
        response = self.client.get(
            entry_url,
            headers=_html_headers(referer=sso_url),
            follow_redirects=True,
        )
        _raise_for_status(response, "open Share entry")
        self.client.get(
            f"{SHARE_BASE_URL}/engine2/header/user-info",
            params={"t": int(time.time() * 1000)},
            headers=_ajax_headers(referer=entry_url),
            follow_redirects=True,
        )

    def get_zyproxy_login_url(self, refer: str = LIBRARY_REFER) -> str:
        """
        Call Share library auth and return the zyproxy login redirect URL.

        Args:
            refer: Internal CNKI refer URL.

        Returns:
            zyproxy login URL.
        """
        response = self.client.get(
            f"{SHARE_BASE_URL}/sso/api/auth/library/vpn358",
            params={"wfwfid": WFWFID, "refer": refer},
            headers=_html_headers(referer=ENTRY_URL),
            follow_redirects=False,
        )
        _raise_for_status(response, "get zyproxy login URL")
        location = response.headers.get("location")
        if location:
            return urljoin(str(response.url), location)
        login_url = _extract_window_location(response.text, str(response.url))
        if "login.elib.zyproxy.zjlib.cn" not in login_url:
            raise ZjlibCnkiError(
                "Share library auth did not return login.elib redirect."
            )
        return login_url

    def enter_zyproxy(self, login_url: str) -> str:
        """
        Enter zyproxy and return the final proxied CNKI URL.

        Args:
            login_url: zyproxy login URL.

        Returns:
            Final proxied CNKI URL.
        """
        response = self.client.get(
            login_url,
            headers=_html_headers(referer=SHARE_BASE_URL + "/"),
            follow_redirects=True,
        )
        _raise_for_status(response, "enter zyproxy")
        final_url = str(response.url)
        if "elib.zyproxy.zjlib.cn" not in final_url:
            raise ZjlibCnkiError(
                f"Unexpected zyproxy final URL: {_redact_url(final_url)}"
            )
        if not _has_cookie(self.client.cookies.jar, "vpn358_sid"):
            raise ZjlibCnkiError("zyproxy login did not set vpn358_sid.")
        return final_url

    def search(self, keyword: str, *, limit: int = 10) -> list[SearchResult]:
        """
        Search CNKI through zyproxy and parse result rows.

        Args:
            keyword: Search keyword.
            limit: Maximum result rows to return.

        Returns:
            Search results in result-page order.
        """
        result_body, handler_body = _search_form_bodies(keyword)
        origin = ZYPROXY_BASE_URL
        kns_root = f"{ZYPROXY_BASE_URL}/kns55/"
        result_url = f"{ZYPROXY_BASE_URL}/kns55/brief/result.aspx"
        handler_url = f"{ZYPROXY_BASE_URL}/kns55/request/SearchHandler.ashx"
        post_headers = {
            **_html_headers(referer=kns_root),
            "Content-Type": "application/x-www-form-urlencoded",
            "Origin": origin,
        }
        response = self.client.post(
            result_url,
            content=result_body.encode("ascii"),
            headers=post_headers,
            follow_redirects=True,
        )
        _raise_for_status(response, "post CNKI result.aspx")
        response = self.client.post(
            handler_url,
            content=handler_body.encode("ascii"),
            headers={
                **post_headers,
                "Referer": result_url,
                "X-Requested-With": "XMLHttpRequest",
            },
            follow_redirects=True,
        )
        _raise_for_status(response, "post CNKI SearchHandler")
        params: dict[str, str | int] = {
            "pagename": "ASP.brief_result_aspx",
            "dbPrefix": "SCDB",
            "dbCatalog": "中国学术文献网络出版总库",
            "ConfigFile": "SCDB.xml",
            "research": "off",
            "t": int(time.time() * 1000),
        }
        response = self.client.get(
            f"{ZYPROXY_BASE_URL}/kns55/brief/brief.aspx",
            params=params,
            headers=_html_headers(referer=result_url),
            follow_redirects=True,
        )
        _raise_for_status(response, "get CNKI brief results")
        self.last_brief_url = str(response.url)
        return _parse_search_results(response.text, str(response.url))[:limit]

    def inspect_result_metadata(self, result: SearchResult) -> CnkiArticleCandidate:
        """
        Open a result detail page and parse metadata before download.

        Args:
            result: Search result to inspect.

        Returns:
            Candidate metadata and PDF URL.
        """
        response = self.client.get(
            result.detail_url,
            headers=_html_headers(
                referer=self.last_brief_url or f"{ZYPROXY_BASE_URL}/kns55/"
            ),
            follow_redirects=True,
        )
        _raise_for_status(response, "open CNKI detail")
        detail_url = str(response.url)
        identity = _extract_article_identity(response.text, fallback_title=result.title)
        pdf_url = _extract_pdf_download_url(response.text, detail_url)
        return CnkiArticleCandidate(
            result=result,
            identity=identity,
            detail_url=detail_url,
            pdf_url=pdf_url,
        )

    def download_pdf(
        self,
        pdf_url: str,
        *,
        title: str | None = None,
        referer: str | None = None,
    ) -> DownloadedPdf:
        """
        Download a PDF and return its bytes.

        Args:
            pdf_url: PDF download URL.
            title: Optional title used for the filename.
            referer: Optional HTTP referer.

        Returns:
            Downloaded PDF content and metadata.
        """
        response = self.client.get(
            pdf_url,
            headers=_html_headers(referer=referer or f"{ZYPROXY_BASE_URL}/kns55/"),
            follow_redirects=True,
        )
        _raise_for_status(response, "download PDF")
        content_type = response.headers.get("content-type", "")
        body = response.content
        if "pdf" not in content_type.lower() and not body.startswith(b"%PDF"):
            redacted_url = _redact_url(str(response.url))
            raise ZjlibCnkiError(
                "Download endpoint did not return PDF "
                f"(content-type={content_type!r}, url={redacted_url})."
            )
        filename_stem = _safe_filename(
            title or _title_from_pdf_url(str(response.url)) or "cnki"
        )
        filename = f"{filename_stem}.pdf"
        return DownloadedPdf(
            filename=filename,
            final_url=str(response.url),
            content_type=content_type,
            byte_count=len(body),
            content=body,
        )

    def download_matching_pdf(
        self,
        expected: ArticleIdentity,
        *,
        result_limit: int = 10,
    ) -> DownloadedPdf:
        """
        Search by title and download only an exact-matching article PDF.

        Args:
            expected: Expected article metadata.
            result_limit: Maximum search results to inspect.

        Returns:
            Downloaded PDF bytes and metadata.
        """
        results = self.search(expected.title, limit=result_limit)
        errors: list[str] = []
        for result in results:
            candidate = self.inspect_result_metadata(result)
            if not does_article_metadata_match(expected, candidate.identity):
                errors.append(f"{result.index}: metadata mismatch")
                continue
            if not candidate.pdf_url:
                errors.append(f"{result.index}: PDF link missing")
                continue
            return self.download_pdf(
                candidate.pdf_url,
                title=candidate.identity.title,
                referer=candidate.detail_url,
            )
        detail = " | ".join(errors) if errors else "no search results"
        raise ZjlibCnkiError(f"No exact CNKI full-text match found: {detail}")


def _www_headers(token: str | None = None) -> dict[str, str]:
    """
    Build Zhejiang Library JSON request headers.

    Args:
        token: Optional bff-user-token.

    Returns:
        Request headers.
    """
    headers = {
        **DEFAULT_HEADERS,
        "Accept": "*/*",
        "Referer": WWW_BASE_URL + "/",
        "bff-org-id": BFF_ORG_ID,
    }
    if token:
        headers["bff-user-token"] = token
    return headers


def _html_headers(referer: str | None = None) -> dict[str, str]:
    """
    Build browser-like HTML request headers.

    Args:
        referer: Optional referer.

    Returns:
        Request headers.
    """
    headers = {
        **DEFAULT_HEADERS,
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    }
    if referer:
        headers["Referer"] = referer
    return headers


def _ajax_headers(referer: str | None = None) -> dict[str, str]:
    """
    Build browser-like AJAX request headers.

    Args:
        referer: Optional referer.

    Returns:
        Request headers.
    """
    headers = {
        **DEFAULT_HEADERS,
        "Accept": "*/*",
        "X-Requested-With": "XMLHttpRequest",
    }
    if referer:
        headers["Referer"] = referer
    return headers


def _json_payload(response: httpx.Response, action: str) -> dict[str, Any]:
    """
    Validate a response and decode JSON.

    Args:
        response: HTTP response.
        action: Human-readable action label.

    Returns:
        JSON object payload.
    """
    _raise_for_status(response, action)
    try:
        payload = response.json()
    except ValueError as exc:
        raise ZjlibCnkiError(f"{action} returned non-JSON response.") from exc
    if isinstance(payload, dict) and payload.get("success") is False:
        raise ZjlibCnkiError(
            f"{action} failed: {payload.get('desc') or payload.get('message')}"
        )
    if not isinstance(payload, dict):
        raise ZjlibCnkiError(f"{action} returned non-object JSON response.")
    return payload


def _payload_data(payload: Mapping[str, Any], action: str) -> dict[str, Any]:
    """
    Extract object data from an API payload.

    Args:
        payload: JSON object payload.
        action: Human-readable action label.

    Returns:
        Payload data object.
    """
    data = payload.get("data")
    if not isinstance(data, dict):
        raise ZjlibCnkiError(f"{action} response did not contain object data.")
    return data


def _raise_for_status(response: httpx.Response, action: str) -> None:
    """
    Raise a domain error for HTTP failures.

    Args:
        response: HTTP response.
        action: Human-readable action label.

    Returns:
        None.
    """
    if response.status_code >= 400:
        redacted_url = _redact_url(str(response.url))
        raise ZjlibCnkiError(
            f"{action} failed with HTTP {response.status_code}: {redacted_url}"
        )


def _extract_window_location(text: str, base_url: str) -> str:
    """
    Extract a JavaScript window location redirect.

    Args:
        text: HTML or JavaScript text.
        base_url: Base URL for relative redirects.

    Returns:
        Absolute redirect URL.
    """
    patterns = [
        r"window\.location\.href\s*=\s*([\"'])(.*?)\1",
        r"location\.href\s*=\s*([\"'])(.*?)\1",
        r"window\.location\s*=\s*([\"'])(.*?)\1",
    ]
    for pattern in patterns:
        match = re.search(pattern, text, re.I | re.S)
        if match:
            return urljoin(base_url, _decode_js_string(match.group(2).strip()))
    raise ZjlibCnkiError("Could not find JavaScript window.location redirect.")


def _extract_share_cookie_sync(text: str) -> tuple[str, dict[str, str]] | None:
    """
    Extract Share cookie synchronization form data.

    Args:
        text: Share protocolAuth HTML.

    Returns:
        Sync URL and form data, or None.
    """
    sign = _extract_js_var(text, "sign")
    url = _extract_js_var(text, "url")
    domain_url = _extract_js_var(text, "domainUrl") or SHARE_BASE_URL
    portal_context_path = _extract_js_var(text, "portalContextPath") or "/entry"
    if not sign or not url or "sso-login/cookie/sync" not in text:
        return None
    domain_url = domain_url.rstrip("/")
    if domain_url.startswith("//"):
        domain_url = "https:" + domain_url
    sync_url = f"{domain_url}{portal_context_path}/sso-login/cookie/sync"
    return sync_url, {"sign": sign, "url": url}


def _extract_js_var(text: str, name: str) -> str | None:
    """
    Extract a JavaScript string variable.

    Args:
        text: JavaScript text.
        name: Variable name.

    Returns:
        Decoded variable value or None.
    """
    pattern = re.compile(rf"var\s+{re.escape(name)}\s*=\s*([\"'])(.*?)\1", re.S)
    match = pattern.search(text)
    if not match:
        return None
    return _decode_js_string(match.group(2).strip())


def _decode_js_string(value: str) -> str:
    """
    Decode common JavaScript string escapes.

    Args:
        value: Raw JavaScript string value.

    Returns:
        Decoded string.
    """
    unescaped = html.unescape(value)
    if "\\" not in unescaped:
        return unescaped
    try:
        decoded = codecs.decode(unescaped, "unicode_escape")
    except UnicodeDecodeError:
        decoded = unescaped
    return decoded.replace("\\/", "/")


def _parse_search_results(text: str, base_url: str) -> list[SearchResult]:
    """
    Parse CNKI search result rows.

    Args:
        text: Search result HTML.
        base_url: Base URL for relative links.

    Returns:
        Parsed search results.
    """
    results: list[SearchResult] = []
    detail_pattern = re.compile(
        r"<a\b[^>]+href\s*=\s*([\"'])(?P<href>[^\"']*?/kns55/detail/detail\.aspx[^\"']*)\1"
        r"[^>]*>(?P<body>.*?)</a>",
        flags=re.I | re.S,
    )
    seen: set[str] = set()
    lowered = text.lower()
    for detail_match in detail_pattern.finditer(text):
        detail_url = urljoin(base_url, html.unescape(detail_match.group("href")))
        if detail_url in seen:
            continue
        seen.add(detail_url)
        row_start = max(lowered.rfind("<tr", 0, detail_match.start()), 0)
        row_end = lowered.find("</tr>", detail_match.end())
        row = (
            text[row_start : row_end + len("</tr>")]
            if row_end >= 0
            else text[row_start:]
        )
        query = _query_dict(urlparse(detail_url).query)
        title = (
            _extract_anchor_title(detail_match.group("body"))
            or query.get("FileName")
            or f"result-{len(results) + 1}"
        )
        results.append(
            SearchResult(
                index=len(results) + 1,
                title=title,
                detail_url=detail_url,
                file_name=query.get("FileName"),
                db_name=query.get("DbName"),
                db_code=query.get("DbCode"),
                download_url=_extract_result_download_url(row, base_url),
            )
        )
    return results


def _extract_article_identity(text: str, fallback_title: str = "") -> ArticleIdentity:
    """
    Extract title, authors, and journal from a CNKI detail page.

    Args:
        text: Detail page HTML.
        fallback_title: Fallback result-page title.

    Returns:
        Article identity metadata.
    """
    title = (
        _meta_content(text, "citation_title")
        or _first_block_text(text, r"<h1\b[^>]*>(.*?)</h1>")
        or _first_block_text(text, r"<h2\b[^>]*>(.*?)</h2>")
        or _title_text(text)
        or fallback_title
    )
    authors = _meta_content_list(text, "citation_author")
    author_text = (
        "; ".join(authors)
        if authors
        else _author_text(text) or _row_value(text, "作者")
    )
    journal_title = (
        _meta_content(text, "citation_journal_title")
        or _row_value(text, "刊名")
        or _row_value(text, "来源")
        or _regex_group(r"(?:刊名|来源)\s*[:：]\s*([^\r\n<]+)", _strip_tags(text))
    )
    return ArticleIdentity(
        title=title or "",
        authors=author_text or "",
        journal_title=journal_title or "",
    )


def _extract_anchor_title(body: str) -> str | None:
    """
    Extract a readable title from a search result anchor body.

    Args:
        body: Anchor HTML body.

    Returns:
        Clean title or None.
    """
    script_title = re.search(r"ReplaceJiankuohao\('(?P<title>.*?)'\)", body, re.S)
    if script_title:
        return _strip_tags(script_title.group("title"))
    return _strip_tags(body) or None


def _extract_result_download_url(row: str, base_url: str) -> str | None:
    """
    Extract an old KNS row download URL.

    Args:
        row: Search result row HTML.
        base_url: Base URL for relative links.

    Returns:
        Absolute download URL or None.
    """
    for _quote, raw_href, _body in HREF_RE.findall(row):
        href = _clean_href(raw_href)
        if "download.aspx" in href.lower():
            return urljoin(base_url, href)
    return None


def _extract_pdf_download_url(text: str, base_url: str) -> str | None:
    """
    Extract an explicit PDF download URL from a detail page.

    Args:
        text: Detail page HTML.
        base_url: Base URL for relative links.

    Returns:
        Absolute PDF download URL or None.
    """
    for _quote, raw_href, body in HREF_RE.findall(text):
        href = _clean_href(raw_href)
        if "download.aspx" not in href.lower():
            continue
        visible = _strip_tags(body)
        if "dflag=pdfdown" in href.lower() or "PDF" in visible.upper():
            return urljoin(base_url, href)
    return None


def _search_form_bodies(keyword: str) -> tuple[str, str]:
    """
    Build CNKI old KNS search form bodies.

    Args:
        keyword: Search keyword.

    Returns:
        result.aspx body and SearchHandler.ashx body.
    """
    encoded_keyword = quote(keyword, safe="")
    timestamp = quote(time.strftime("%a %b %d %Y %H:%M:%S GMT%z"), safe="")
    common = {
        "dbPrefix": "SCDB",
        "db_opt": "中国学术文献网络出版总库",
        "txt_1_sel": "题名",
        "txt_1_value1": keyword,
        "txt_1_relation": "#CNKI_AND",
        "txt_1_special1": "=",
        "txt_extension": "xls",
    }
    result_fields = {
        **common,
        "hidTabChange": "",
        "hidDivIDS": "",
        "txt_i": "1",
        "txt_c": "7",
        "currentid": "txt_1_value1",
        "action": "scdbsearch",
    }
    handler_fields = {
        "action": "",
        "NaviCode": "*",
        "PageName": "ASP.brief_result_aspx",
        "DbPrefix": "SCDB",
        "DbCatalog": "中国学术文献网络出版总库",
        "ConfigFile": "SCDB.xml",
        **common,
        "his": "0",
        "__": timestamp,
    }
    result_body = urlencode(result_fields)
    handler_body = urlencode(handler_fields)
    return (
        _replace_form_value(
            result_body, "txt_1_value1", encoded_keyword, is_encoded=True
        ),
        _replace_form_value(
            handler_body, "txt_1_value1", encoded_keyword, is_encoded=True
        ),
    )


def _replace_form_value(
    body: str,
    name: str,
    value: str,
    *,
    is_encoded: bool = False,
) -> str:
    """
    Replace an application/x-www-form-urlencoded field value.

    Args:
        body: Encoded form body.
        name: Field name.
        value: Replacement value.
        is_encoded: Whether the replacement is already URL-encoded.

    Returns:
        Form body with the field replaced or appended.
    """
    encoded_name = re.escape(name)
    encoded_value = value if is_encoded else quote(value, safe="")
    pattern = re.compile(rf"(^|&)({encoded_name}=)([^&]*)")
    if pattern.search(body):
        return pattern.sub(
            lambda match: f"{match.group(1)}{match.group(2)}{encoded_value}", body
        )
    return body + f"&{name}={encoded_value}"


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


def _meta_content(text: str, name: str) -> str | None:
    """
    Read the first matching meta content value.

    Args:
        text: HTML text.
        name: Meta name.

    Returns:
        Meta content or None.
    """
    values = _meta_content_list(text, name)
    return values[0] if values else None


def _meta_content_list(text: str, name: str) -> list[str]:
    """
    Read all matching meta content values.

    Args:
        text: HTML text.
        name: Meta name.

    Returns:
        Meta content values.
    """
    values: list[str] = []
    for match in re.finditer(r"<meta\b[^>]*>", text, re.S | re.I):
        attrs = _attrs(match.group(0))
        if attrs.get("name", "").casefold() != name.casefold():
            continue
        content = _clean_text(attrs.get("content"))
        if content:
            values.append(content)
    return values


def _author_text(text: str) -> str | None:
    """
    Parse author names from a detail page author block.

    Args:
        text: Detail page HTML.

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
    Read a detail row value by label.

    Args:
        text: Detail page HTML.
        label: Row label.

    Returns:
        Clean row value or None.
    """
    patterns = [
        rf'<span\b[^>]*class="[^"]*\browtit\b[^"]*"[^>]*>\s*{re.escape(label)}\s*[:：]?\s*</span>\s*<p[^>]*>(.*?)</p>',
        rf"<td\b[^>]*>\s*{re.escape(label)}\s*[:：]?\s*</td>\s*<td[^>]*>(.*?)</td>",
        rf"<label\b[^>]*>\s*{re.escape(label)}\s*[:：]?\s*</label>\s*([^<]+)",
    ]
    for pattern in patterns:
        match = re.search(pattern, text, re.S)
        if match:
            return _strip_tags(match.group(1))
    return None


def _first_block_text(text: str, pattern: str) -> str | None:
    """
    Extract and clean the first regex capture block.

    Args:
        text: HTML text.
        pattern: Regex pattern with one capture group.

    Returns:
        Clean block text or None.
    """
    match = re.search(pattern, text, re.S | re.I)
    if not match:
        return None
    return _strip_tags(match.group(1))


def _title_text(text: str) -> str | None:
    """
    Extract and clean the document title.

    Args:
        text: HTML text.

    Returns:
        Clean document title or None.
    """
    title = _first_block_text(text, r"<title>(.*?)</title>")
    if not title:
        return None
    title = re.sub(r"\s*-\s*(?:中国知网|CNKI)\s*$", "", title).strip()
    return title or None


def _clean_href(value: str) -> str:
    """
    Clean an anchor href value.

    Args:
        value: Raw href.

    Returns:
        Clean href.
    """
    return re.sub(r"\s+", "", html.unescape(value)).strip()


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
    text = TAG_RE.sub(" ", html.unescape(value))
    return re.sub(r"\s+", " ", text).strip()


def _clean_text(value: str | None) -> str | None:
    """
    Normalize whitespace in plain text.

    Args:
        value: Raw text.

    Returns:
        Clean text or None.
    """
    if value is None:
        return None
    text = re.sub(r"\s+", " ", html.unescape(str(value))).strip()
    return text or None


def _query_dict(query: str) -> dict[str, str]:
    """
    Decode a query string into a first-value dictionary.

    Args:
        query: Raw query string.

    Returns:
        Decoded query mapping.
    """
    result: dict[str, str] = {}
    for part in query.split("&"):
        if not part:
            continue
        key, value = part.split("=", 1) if "=" in part else (part, "")
        result[unquote_plus(html.unescape(key))] = unquote_plus(html.unescape(value))
    return result


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


def _has_cookie(jar: Any, name: str) -> bool:
    """
    Check whether a cookie jar contains a cookie name.

    Args:
        jar: Cookie jar-like object.
        name: Cookie name.

    Returns:
        True when the cookie exists.
    """
    return any(getattr(cookie, "name", "") == name for cookie in jar)


def _cookie_to_json(cookie: Cookie) -> dict[str, Any]:
    """
    Convert a cookie to JSON-serializable data.

    Args:
        cookie: Cookie object.

    Returns:
        Cookie data.
    """
    return {
        "name": cookie.name,
        "value": cookie.value,
        "domain": cookie.domain,
        "path": cookie.path,
        "secure": cookie.secure,
        "expires": cookie.expires,
        "discard": cookie.discard,
        "rest": dict(getattr(cookie, "_rest", {})),
    }


def _cookie_from_json(data: Mapping[str, Any]) -> Cookie:
    """
    Convert JSON-like cookie data to a cookie.

    Args:
        data: Cookie data.

    Returns:
        Cookie object.
    """
    domain = str(data.get("domain") or "")
    path = str(data.get("path") or "/")
    return Cookie(
        version=0,
        name=str(data["name"]),
        value=str(data["value"]),
        port=None,
        port_specified=False,
        domain=domain,
        domain_specified=bool(domain),
        domain_initial_dot=domain.startswith("."),
        path=path,
        path_specified=bool(path),
        secure=bool(data.get("secure")),
        expires=data.get("expires"),
        discard=bool(data.get("discard", False)),
        comment=None,
        comment_url=None,
        rest=dict(data.get("rest") or {}),
        rfc2109=False,
    )


def _jwt_exp(token: str | None) -> int | None:
    """
    Extract a JWT exp value without validating the token.

    Args:
        token: JWT string.

    Returns:
        Expiration timestamp or None.
    """
    if not token:
        return None
    parts = token.split(".")
    if len(parts) < 2:
        return None
    payload = parts[1]
    payload += "=" * ((4 - len(payload) % 4) % 4)
    try:
        data = json.loads(base64.urlsafe_b64decode(payload.encode("ascii")))
    except (ValueError, TypeError):
        return None
    exp = data.get("exp")
    try:
        return int(exp)
    except (TypeError, ValueError):
        return None


def _redact_url(url: str) -> str:
    """
    Redact sensitive query parameter values from a URL.

    Args:
        url: Raw URL.

    Returns:
        URL with sensitive query values redacted.
    """
    parsed = urlparse(url)
    if not parsed.query:
        return url
    sensitive = {
        "token",
        "bff-user-token",
        "userid",
        "username",
        "md5",
        "sign",
        "mhEnc",
        "enc",
        "sid",
        "uid",
        "filename",
        "dk",
    }
    pairs: list[tuple[str, str]] = []
    for part in parsed.query.split("&"):
        key, value = part.split("=", 1) if "=" in part else (part, "")
        pairs.append((key, "<redacted>" if key in sensitive else value))
    return urlunparse(parsed._replace(query=urlencode(pairs)))


def _safe_filename(value: str) -> str:
    """
    Build a safe PDF filename stem.

    Args:
        value: Raw title or filename.

    Returns:
        Safe filename stem.
    """
    text = _strip_tags(value)
    text = re.sub(r"[\\/:*?\"<>|]+", "_", text)
    text = re.sub(r"\s+", " ", text).strip(" .")
    return (text[:120] or "cnki").strip()


def _title_from_pdf_url(url: str) -> str | None:
    """
    Extract a title-like value from a PDF download URL.

    Args:
        url: PDF download URL.

    Returns:
        Title value or None.
    """
    query = _query_dict(urlparse(url).query)
    title = query.get("filetitle")
    return title or query.get("filename")
