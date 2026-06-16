"""Tests for favorites export authentication."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

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
        Ensure direct dependency use does not pass FastAPI Query defaults onward.

        Returns:
            None.
        """
        user = await get_export_user(authorization=f"Bearer {self.token}")

        self.assertEqual(user["id"], self.user["id"])


if __name__ == "__main__":
    unittest.main()
