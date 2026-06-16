"""Tests for global article access capability behavior."""

from __future__ import annotations

import base64
import json
import sqlite3
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import aiosqlite
from fastapi import HTTPException

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.queries.articles import (
    get_article_access,
    redirect_article_fulltext,
)
from paper_scanner.sources.zjlib_cnki import DownloadedPdf, ZjlibCnkiError


def build_unsigned_jwt(exp: int) -> str:
    """
    Build an unsigned JWT-like token for session status tests.

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


def build_cookie(name: str, value: str) -> dict[str, object]:
    """
    Build JSON cookie data for stored CNKI session tests.

    Args:
        name: Cookie name.
        value: Cookie value.

    Returns:
        JSON-serializable cookie data.
    """
    return {
        "name": name,
        "value": value,
        "domain": "www.zjlib.cn",
        "path": "/",
        "secure": True,
        "expires": None,
        "discard": False,
        "rest": {},
    }


async def create_access_db(
    library_id: str,
    full_text_file: str | None,
    permalink: str | None,
    doi: str | None = None,
) -> aiosqlite.Connection:
    """
    Build an in-memory article database for access tests.

    Args:
        library_id: Journal source identifier.
        full_text_file: Article full-text URL value.
        permalink: Article permalink value.
        doi: Article DOI value.

    Returns:
        Open database connection with one article row.
    """
    db = await aiosqlite.connect(":memory:")
    db.row_factory = sqlite3.Row
    await db.executescript(
        """
        CREATE TABLE journals (
            journal_id INTEGER PRIMARY KEY,
            library_id TEXT,
            title TEXT,
            issn TEXT
        );
        CREATE TABLE issues (
            issue_id INTEGER PRIMARY KEY,
            publication_year INTEGER,
            number TEXT
        );
        CREATE TABLE articles (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            title TEXT,
            authors TEXT,
            doi TEXT,
            platform_id TEXT,
            full_text_file TEXT,
            permalink TEXT
        );
        """
    )
    await db.execute(
        """
        INSERT INTO journals (journal_id, library_id, title, issn)
        VALUES (?, ?, ?, ?)
        """,
        (1, library_id, "Test Journal", "1000-0000"),
    )
    await db.execute(
        """
        INSERT INTO issues (issue_id, publication_year, number)
        VALUES (?, ?, ?)
        """,
        (2, 2026, "01"),
    )
    await db.execute(
        """
        INSERT INTO articles (
            article_id,
            journal_id,
            issue_id,
            title,
            authors,
            doi,
            platform_id,
            full_text_file,
            permalink
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            10,
            1,
            2,
            "Test article",
            "Author One; Author Two",
            doi,
            "TEST202601001",
            full_text_file,
            permalink,
        ),
    )
    await db.commit()
    return db


class ArticleAccessTest(unittest.IsolatedAsyncioTestCase):
    """Verify article detail and full-text capability decisions."""

    def setUp(self) -> None:
        """
        Create an isolated auth database.

        Returns:
            None.
        """
        self.temp_dir = tempfile.TemporaryDirectory()
        self.previous_auth_db_path = auth_db.AUTH_DB_PATH
        auth_db.AUTH_DB_PATH = Path(self.temp_dir.name) / "auth.sqlite"
        auth_db.init_auth_db()
        self.user = auth_db.create_user("alice", "secret123")

    def tearDown(self) -> None:
        """
        Restore the auth database path.

        Returns:
            None.
        """
        auth_db.AUTH_DB_PATH = self.previous_auth_db_path
        self.temp_dir.cleanup()

    def store_active_cnki_session(self) -> None:
        """
        Store an active fake CNKI session for the test user.

        Returns:
            None.
        """
        auth_db.upsert_cnki_session(
            self.user["id"],
            {
                "bff_user_token": build_unsigned_jwt(int(time.time()) + 3600),
                "qr_uuid": "qr-active",
                "cookies": [build_cookie("userToken", "SECRET")],
            },
            status="active",
        )

    async def test_stored_fulltext_access_is_available(self) -> None:
        """
        Ensure stored full-text URLs expose both detail and full-text actions.

        Returns:
            None.
        """
        db = await create_access_db(
            "scholarly",
            "https://example.test/fulltext.pdf",
            "https://doi.org/10.1000/test",
        )
        try:
            access = await get_article_access(10, db, self.user)
        finally:
            await db.close()

        self.assertTrue(access.detail.available)
        self.assertTrue(access.fulltext.available)
        self.assertEqual(access.fulltext.provider, "stored_url")

    async def test_detail_only_article_has_no_fulltext_action(self) -> None:
        """
        Ensure detail-only records do not pretend full text is available.

        Returns:
            None.
        """
        db = await create_access_db(
            "scholarly",
            None,
            "https://doi.org/10.1000/test",
        )
        try:
            access = await get_article_access(10, db, self.user)
        finally:
            await db.close()

        self.assertTrue(access.detail.available)
        self.assertFalse(access.fulltext.available)
        self.assertFalse(access.fulltext.requires_login)

    async def test_cnki_article_requires_login_without_session(self) -> None:
        """
        Ensure CNKI records report login-required full-text state.

        Returns:
            None.
        """
        db = await create_access_db(
            "cnki",
            None,
            "https://oversea.cnki.net/openlink/detail?filename=TEST",
        )
        try:
            access = await get_article_access(10, db, self.user)
        finally:
            await db.close()

        self.assertTrue(access.detail.available)
        self.assertFalse(access.fulltext.available)
        self.assertTrue(access.fulltext.requires_login)
        self.assertEqual(access.fulltext.provider, "zjlib_cnki")

    async def test_cnki_article_fulltext_available_with_active_session(self) -> None:
        """
        Ensure CNKI records expose full text when the user session is active.

        Returns:
            None.
        """
        self.store_active_cnki_session()
        db = await create_access_db(
            "cnki",
            None,
            "https://oversea.cnki.net/openlink/detail?filename=TEST",
        )
        try:
            access = await get_article_access(10, db, self.user)
        finally:
            await db.close()

        self.assertTrue(access.fulltext.available)
        self.assertFalse(access.fulltext.requires_login)
        self.assertEqual(access.fulltext.provider, "zjlib_cnki")

    async def test_cnki_fulltext_endpoint_returns_pdf_for_active_session(self) -> None:
        """
        Ensure active CNKI sessions can return service-side PDF bytes.

        Returns:
            None.
        """
        self.store_active_cnki_session()
        db = await create_access_db(
            "cnki",
            None,
            "https://oversea.cnki.net/openlink/detail?filename=TEST",
        )
        downloaded = DownloadedPdf(
            filename="Test article.pdf",
            final_url="https://example.test/pdf",
            content_type="application/pdf",
            byte_count=8,
            content=b"%PDF-1.7",
        )
        try:
            with patch(
                "paper_scanner.api.queries.articles._download_cnki_fulltext_pdf",
                return_value=(
                    downloaded,
                    {
                        "bff_user_token": build_unsigned_jwt(int(time.time()) + 3600),
                        "qr_uuid": "qr-active",
                        "cookies": [build_cookie("userToken", "SECRET")],
                    },
                ),
            ):
                response = await redirect_article_fulltext(10, db, self.user)
        finally:
            await db.close()

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.media_type, "application/pdf")
        self.assertEqual(response.body, b"%PDF-1.7")
        status = auth_db.get_cnki_session_status(self.user["id"])
        self.assertEqual(status["status"], "active")
        self.assertIsNotNone(status["last_used_at"])

    async def test_cnki_fulltext_endpoint_reports_exact_match_failure(self) -> None:
        """
        Ensure CNKI exact-match failures do not fall back to wrong PDFs.

        Returns:
            None.
        """
        self.store_active_cnki_session()
        db = await create_access_db(
            "cnki",
            None,
            "https://oversea.cnki.net/openlink/detail?filename=TEST",
        )
        try:
            with (
                patch(
                    "paper_scanner.api.queries.articles._download_cnki_fulltext_pdf",
                    side_effect=ZjlibCnkiError("No exact CNKI full-text match found"),
                ),
                self.assertRaises(HTTPException) as context,
            ):
                await redirect_article_fulltext(10, db, self.user)
        finally:
            await db.close()

        self.assertEqual(context.exception.status_code, 404)


if __name__ == "__main__":
    unittest.main()
