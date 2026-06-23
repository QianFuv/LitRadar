"""Tests for favorites export authentication."""

from __future__ import annotations

import inspect
import tempfile
import unittest
from pathlib import Path

from fastapi import HTTPException

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.routes.favorites import get_export_user


class FavoritesExportAuthTest(unittest.IsolatedAsyncioTestCase):
    """Verify export authentication accepts supported token transports."""

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
        self.token = auth_db.create_access_token(self.user["id"], name="export")[
            "token"
        ]

    def tearDown(self) -> None:
        """
        Restore the auth database path.

        Returns:
            None.
        """
        auth_db.AUTH_DB_PATH = self.previous_auth_db_path
        self.temp_dir.cleanup()

    async def test_export_user_accepts_bearer_header(self) -> None:
        """
        Ensure export accepts explicit Bearer API-token authentication.

        Returns:
            None.
        """
        user = await get_export_user(
            authorization=f"Bearer {self.token}",
            session_cookie=None,
        )

        self.assertEqual(user["id"], self.user["id"])

    async def test_export_user_accepts_session_cookie(self) -> None:
        """
        Ensure export accepts browser session cookie authentication.

        Returns:
            None.
        """
        user = await get_export_user(
            authorization=None,
            session_cookie=self.token,
        )

        self.assertEqual(user["id"], self.user["id"])

    async def test_export_user_rejects_missing_auth(self) -> None:
        """
        Ensure export requires either Bearer auth or the session cookie.

        Returns:
            None.
        """
        with self.assertRaises(HTTPException) as context:
            await get_export_user(authorization=None, session_cookie=None)

        self.assertEqual(context.exception.status_code, 401)

    def test_export_user_does_not_accept_query_access_token(self) -> None:
        """
        Ensure export auth cannot regress to query-string token transport.

        Returns:
            None.
        """
        parameters = inspect.signature(get_export_user).parameters

        self.assertNotIn("access_token", parameters)


if __name__ == "__main__":
    unittest.main()
