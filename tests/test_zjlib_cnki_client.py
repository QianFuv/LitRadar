"""Tests for Zhejiang Library mediated CNKI full-text client utilities."""

from __future__ import annotations

import base64
import json
import time
import unittest

import httpx

from paper_scanner.sources.zjlib_cnki import (
    ArticleIdentity,
    CnkiArticleCandidate,
    DownloadedPdf,
    SearchResult,
    ZhejiangLibraryCnkiClient,
    ZjlibCnkiError,
    does_article_metadata_match,
)
from paper_scanner.sources.zjlib_cnki.client import _parse_search_results


def build_unsigned_jwt(exp: int) -> str:
    """
    Build an unsigned JWT-like string for expiry parsing tests.

    Args:
        exp: Expiration timestamp.

    Returns:
        JWT-like token string.
    """

    def encode(payload: dict[str, object]) -> str:
        """
        Base64-url encode one JWT segment.

        Args:
            payload: JSON payload.

        Returns:
            Encoded segment without padding.
        """
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        return base64.urlsafe_b64encode(body).decode("ascii").rstrip("=")

    return f"{encode({'alg': 'none'})}.{encode({'exp': exp})}."


class FakeMatchingClient(ZhejiangLibraryCnkiClient):
    """Fake client that records whether PDF download was attempted."""

    def __init__(self, candidates: list[CnkiArticleCandidate]) -> None:
        """
        Initialize the fake matching client.

        Args:
            candidates: Candidate metadata returned by inspection.
        """
        super().__init__(
            client=httpx.Client(transport=httpx.MockTransport(self.handle))
        )
        self.candidates = candidates
        self.downloaded_urls: list[str] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        """
        Return a placeholder response for unused HTTP calls.

        Args:
            request: HTTP request.

        Returns:
            Placeholder response.
        """
        return httpx.Response(404, request=request)

    def search(self, keyword: str, *, limit: int = 10) -> list[SearchResult]:
        """
        Return fake search results.

        Args:
            keyword: Search keyword.
            limit: Maximum result count.

        Returns:
            Fake search result rows.
        """
        return [candidate.result for candidate in self.candidates[:limit]]

    def inspect_result_metadata(self, result: SearchResult) -> CnkiArticleCandidate:
        """
        Return candidate metadata by result index.

        Args:
            result: Search result.

        Returns:
            Candidate metadata.
        """
        return self.candidates[result.index - 1]

    def download_pdf(
        self,
        pdf_url: str,
        *,
        title: str | None = None,
        referer: str | None = None,
    ) -> DownloadedPdf:
        """
        Record and return a fake PDF download.

        Args:
            pdf_url: PDF URL.
            title: Optional title.
            referer: Optional referer.

        Returns:
            Fake PDF metadata.
        """
        self.downloaded_urls.append(pdf_url)
        return DownloadedPdf(
            filename=f"{title or 'cnki'}.pdf",
            final_url=pdf_url,
            content_type="application/pdf",
            byte_count=8,
            content=b"%PDF-1.7",
        )


class ZhejiangLibraryCnkiClientTest(unittest.TestCase):
    """Verify the Zhejiang Library CNKI client behavior used by the API layer."""

    def test_qr_login_poll_and_safe_info(self) -> None:
        """
        Ensure QR login state can be persisted while safe info omits secrets.

        Returns:
            None.
        """
        exp = int(time.time()) + 3600
        token = build_unsigned_jwt(exp)

        def handle(request: httpx.Request) -> httpx.Response:
            """
            Return mocked Zhejiang Library login responses.

            Args:
                request: HTTP request.

            Returns:
                Mocked HTTP response.
            """
            if request.url.path.endswith("/login/zfb-qr"):
                return httpx.Response(
                    200,
                    json={
                        "data": {
                            "uuid": "uuid-1",
                            "status": "WAITING_SCAN",
                            "qrCode": "https://qr.test/image.png",
                        }
                    },
                    request=request,
                )
            if request.url.path.endswith("/qr/status"):
                return httpx.Response(
                    200,
                    json={"data": {"status": "COMPLETE", "data": token}},
                    request=request,
                )
            return httpx.Response(404, request=request)

        with ZhejiangLibraryCnkiClient(
            client=httpx.Client(transport=httpx.MockTransport(handle))
        ) as client:
            qr_login = client.start_qr_login()
            self.assertEqual(qr_login.uuid, "uuid-1")
            self.assertEqual(qr_login.status, "WAITING_SCAN")

            completed_token = client.poll_qr_login(
                interval_seconds=0.0, timeout_seconds=1
            )
            self.assertEqual(completed_token, token)

            state = client.to_state_data()
            info = client.client_info()

        self.assertEqual(state["bff_user_token"], token)
        self.assertTrue(info.has_bff_user_token)
        self.assertEqual(info.bff_user_token_exp, exp)
        self.assertIn("userToken", info.cookie_names)
        self.assertNotIn(token, json.dumps(info.__dict__, ensure_ascii=False))

    def test_search_result_parser_extracts_detail_rows(self) -> None:
        """
        Ensure old KNS search result rows are parsed into candidates.

        Returns:
            None.
        """
        detail_href = (
            "/kns55/detail/detail.aspx?FileName=TEST001&DbName=CJFD&DbCode=CJFQ"
        )
        html_text = f"""
        <table>
          <tr>
            <td>
              <a href="{detail_href}">
                ReplaceJiankuohao('测试文章')
              </a>
              <a href="/kns55/download.aspx?filename=TEST001">下载</a>
            </td>
          </tr>
        </table>
        """
        results = _parse_search_results(
            html_text,
            "https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/brief/brief.aspx",
        )

        self.assertEqual(len(results), 1)
        self.assertEqual(results[0].title, "测试文章")
        self.assertEqual(results[0].file_name, "TEST001")
        self.assertIn("/kns55/download.aspx", results[0].download_url or "")

    def test_detail_metadata_parser_extracts_identity_and_pdf_url(self) -> None:
        """
        Ensure detail inspection reads candidate identity before download.

        Returns:
            None.
        """

        def handle(request: httpx.Request) -> httpx.Response:
            """
            Return a synthetic CNKI detail page.

            Args:
                request: HTTP request.

            Returns:
                Mocked detail response.
            """
            text = """
            <html>
              <head>
                <meta name="citation_title"
                      content="基于TSC—LSTM 的新密市地面沉降预测模型研究" />
                <meta name="citation_author" content="张三" />
                <meta name="citation_author" content="李四" />
                <meta name="citation_journal_title" content="测绘科学" />
              </head>
              <body>
                <a href="/kcms/download.aspx?filename=TEST&dflag=pdfdown">PDF下载</a>
              </body>
            </html>
            """
            return httpx.Response(200, text=text, request=request)

        result = SearchResult(
            index=1,
            title="fallback",
            detail_url="https://example.test/kns55/detail/detail.aspx?FileName=TEST",
            file_name="TEST",
            db_name="CJFD",
            db_code="CJFQ",
        )
        with ZhejiangLibraryCnkiClient(
            client=httpx.Client(transport=httpx.MockTransport(handle))
        ) as client:
            candidate = client.inspect_result_metadata(result)

        self.assertEqual(candidate.identity.authors, "张三; 李四")
        self.assertEqual(candidate.identity.journal_title, "测绘科学")
        self.assertIn("dflag=pdfdown", candidate.pdf_url or "")

    def test_metadata_match_requires_title_authors_and_journal(self) -> None:
        """
        Ensure exact matching requires all required metadata fields.

        Returns:
            None.
        """
        expected = ArticleIdentity(
            title="基于 TSC-LSTM 的新密市地面沉降预测模型研究",
            authors="张三; 李四",
            journal_title="测绘科学",
        )
        actual = ArticleIdentity(
            title="基于TSC—LSTM的新密市地面沉降预测模型研究",
            authors="张三；李四",
            journal_title="测绘 科学",
        )
        wrong_journal = ArticleIdentity(
            title=actual.title,
            authors=actual.authors,
            journal_title="遥感学报",
        )

        self.assertTrue(does_article_metadata_match(expected, actual))
        self.assertFalse(does_article_metadata_match(expected, wrong_journal))

    def test_metadata_match_rejects_any_required_field_mismatch(self) -> None:
        """
        Ensure each required metadata field can independently reject a candidate.

        Returns:
            None.
        """
        expected = ArticleIdentity(
            title="目标文章",
            authors="张三; 李四",
            journal_title="目标期刊",
        )
        cases = [
            (
                "title",
                ArticleIdentity(
                    title="另一篇文章",
                    authors=expected.authors,
                    journal_title=expected.journal_title,
                ),
            ),
            (
                "author_order",
                ArticleIdentity(
                    title=expected.title,
                    authors="李四; 张三",
                    journal_title=expected.journal_title,
                ),
            ),
            (
                "missing_authors",
                ArticleIdentity(
                    title=expected.title,
                    authors="",
                    journal_title=expected.journal_title,
                ),
            ),
            (
                "journal",
                ArticleIdentity(
                    title=expected.title,
                    authors=expected.authors,
                    journal_title="错误期刊",
                ),
            ),
            (
                "missing_journal",
                ArticleIdentity(
                    title=expected.title,
                    authors=expected.authors,
                    journal_title="",
                ),
            ),
        ]

        for name, actual in cases:
            with self.subTest(name=name):
                self.assertFalse(does_article_metadata_match(expected, actual))

    def test_download_matching_pdf_skips_mismatched_candidates(self) -> None:
        """
        Ensure mismatched CNKI candidates are not downloaded.

        Returns:
            None.
        """
        expected = ArticleIdentity(
            title="目标文章",
            authors="张三; 李四",
            journal_title="目标期刊",
        )
        result = SearchResult(
            index=1,
            title="目标文章",
            detail_url="https://example.test/detail",
            file_name="TEST",
            db_name="CJFD",
            db_code="CJFQ",
        )
        candidate = CnkiArticleCandidate(
            result=result,
            identity=ArticleIdentity(
                title="目标文章",
                authors="张三; 王五",
                journal_title="目标期刊",
            ),
            detail_url="https://example.test/detail",
            pdf_url="https://example.test/pdf",
        )
        client = FakeMatchingClient([candidate])

        with self.assertRaises(ZjlibCnkiError):
            client.download_matching_pdf(expected)

        self.assertEqual(client.downloaded_urls, [])

    def test_download_matching_pdf_downloads_later_exact_candidate(self) -> None:
        """
        Ensure search scanning skips wrong candidates before downloading a match.

        Returns:
            None.
        """
        expected = ArticleIdentity(
            title="目标文章",
            authors="张三; 李四",
            journal_title="目标期刊",
        )
        wrong_result = SearchResult(
            index=1,
            title="目标文章",
            detail_url="https://example.test/detail-1",
            file_name="TEST1",
            db_name="CJFD",
            db_code="CJFQ",
        )
        exact_result = SearchResult(
            index=2,
            title="目标文章",
            detail_url="https://example.test/detail-2",
            file_name="TEST2",
            db_name="CJFD",
            db_code="CJFQ",
        )
        client = FakeMatchingClient(
            [
                CnkiArticleCandidate(
                    result=wrong_result,
                    identity=ArticleIdentity(
                        title="目标文章",
                        authors="张三; 李四",
                        journal_title="错误期刊",
                    ),
                    detail_url="https://example.test/detail-1",
                    pdf_url="https://example.test/pdf-1",
                ),
                CnkiArticleCandidate(
                    result=exact_result,
                    identity=expected,
                    detail_url="https://example.test/detail-2",
                    pdf_url="https://example.test/pdf-2",
                ),
            ]
        )

        downloaded = client.download_matching_pdf(expected)

        self.assertEqual(downloaded.content, b"%PDF-1.7")
        self.assertEqual(client.downloaded_urls, ["https://example.test/pdf-2"])

    def test_download_matching_pdf_downloads_exact_candidate(self) -> None:
        """
        Ensure exact CNKI candidates can be downloaded.

        Returns:
            None.
        """
        expected = ArticleIdentity(
            title="目标文章",
            authors="张三; 李四",
            journal_title="目标期刊",
        )
        result = SearchResult(
            index=1,
            title="目标文章",
            detail_url="https://example.test/detail",
            file_name="TEST",
            db_name="CJFD",
            db_code="CJFQ",
        )
        candidate = CnkiArticleCandidate(
            result=result,
            identity=expected,
            detail_url="https://example.test/detail",
            pdf_url="https://example.test/pdf",
        )
        client = FakeMatchingClient([candidate])

        downloaded = client.download_matching_pdf(expected)

        self.assertEqual(downloaded.content, b"%PDF-1.7")
        self.assertEqual(client.downloaded_urls, ["https://example.test/pdf"])


if __name__ == "__main__":
    unittest.main()
