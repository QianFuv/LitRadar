"""Tests for browser cookie-backed authentication."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.auth_deps import SESSION_COOKIE_NAME
from paper_scanner.api.routes import register_routes


class AuthCookieSessionTest(unittest.TestCase):
    """Verify browser session cookies and bearer token compatibility."""

    def setUp(self) -> None:
        """
        Create an isolated auth database and API app.

        Returns:
            None.
        """
        self.temp_dir = tempfile.TemporaryDirectory()
        self.previous_auth_db_path = auth_db.AUTH_DB_PATH
        auth_db.AUTH_DB_PATH = Path(self.temp_dir.name) / "auth.sqlite"
        auth_db.init_auth_db()
        self.user = auth_db.create_user("alice", "secret123")
        app = FastAPI()
        register_routes(app)
        self.client = TestClient(app)

    def tearDown(self) -> None:
        """
        Restore the auth database path.

        Returns:
            None.
        """
        auth_db.AUTH_DB_PATH = self.previous_auth_db_path
        self.temp_dir.cleanup()

    def login(self) -> str:
        """
        Log in through the API and return the session cookie value.

        Returns:
            Raw session cookie value.
        """
        response = self.client.post(
            "/api/auth/login",
            json={"username": "alice", "password": "secret123"},
        )

        self.assertEqual(response.status_code, 200)
        cookie_value = response.cookies.get(SESSION_COOKIE_NAME)
        self.assertIsNotNone(cookie_value)
        return str(cookie_value)

    def test_login_sets_http_only_cookie_without_returning_token(self) -> None:
        """
        Ensure browser login stores the token only in an HttpOnly cookie.

        Returns:
            None.
        """
        response = self.client.post(
            "/api/auth/login",
            json={"username": "alice", "password": "secret123"},
        )

        self.assertEqual(response.status_code, 200)
        payload = response.json()
        set_cookie = response.headers["set-cookie"].lower()
        self.assertEqual(payload["user"]["id"], self.user["id"])
        self.assertNotIn("access_token", payload)
        self.assertIn(f"{SESSION_COOKIE_NAME}=", set_cookie)
        self.assertIn("httponly", set_cookie)
        self.assertIn("samesite=lax", set_cookie)
        self.assertIn("max-age=", set_cookie)

    def test_cookie_authenticates_current_user(self) -> None:
        """
        Ensure the session cookie authenticates protected browser requests.

        Returns:
            None.
        """
        self.login()

        response = self.client.get("/api/auth/me")

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["id"], self.user["id"])

    def test_current_user_rejects_missing_auth(self) -> None:
        """
        Ensure protected auth routes reject requests without credentials.

        Returns:
            None.
        """
        self.client.cookies.clear()

        response = self.client.get("/api/auth/me")

        self.assertEqual(response.status_code, 401)

    def test_bearer_token_still_authenticates_api_clients(self) -> None:
        """
        Ensure explicit bearer tokens remain valid for API clients.

        Returns:
            None.
        """
        token = auth_db.create_access_token(self.user["id"], name="api")["token"]

        response = self.client.get(
            "/api/auth/me",
            headers={"Authorization": f"Bearer {token}"},
        )

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["id"], self.user["id"])

    def test_logout_revokes_cookie_session_and_clears_cookie(self) -> None:
        """
        Ensure cookie logout revokes the active token and clears the cookie.

        Returns:
            None.
        """
        cookie_value = self.login()

        response = self.client.post("/api/auth/logout")

        self.assertEqual(response.status_code, 200)
        self.assertIn(f"{SESSION_COOKIE_NAME}=", response.headers["set-cookie"])
        self.assertIn("Max-Age=0", response.headers["set-cookie"])
        self.assertIsNone(auth_db.verify_access_token(cookie_value))
        self.assertEqual(self.client.get("/api/auth/me").status_code, 401)


if __name__ == "__main__":
    unittest.main()
