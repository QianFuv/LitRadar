"""Shadow contracts for Rust authentication endpoints."""

from __future__ import annotations

import json
import unittest
import urllib.error
import urllib.request
from email.message import Message
from typing import Any

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.routes import register_routes

from .test_public_api_contracts import (
    available_port,
    start_rust_api,
    stop_process,
    temporary_auth_database,
)


def request_json(
    method: str,
    url: str,
    payload: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
) -> tuple[int, Any, Message]:
    """
    Send a JSON HTTP request to the Rust API test server.

    Args:
        method: HTTP method.
        url: Target URL.
        payload: Optional JSON request payload.
        headers: Optional request headers.

    Returns:
        Status code, decoded JSON payload, and response headers.
    """
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(url, data=body, method=method)
    request.add_header("Accept", "application/json")
    if body is not None:
        request.add_header("Content-Type", "application/json")
    for key, value in (headers or {}).items():
        request.add_header(key, value)
    try:
        with urllib.request.urlopen(request, timeout=20.0) as response:
            raw = response.read()
            return int(response.status), decode_json(raw), response.headers
    except urllib.error.HTTPError as error:
        raw = error.read()
        return int(error.code), decode_json(raw), error.headers


def decode_json(raw: bytes) -> Any:
    """
    Decode a JSON response body.

    Args:
        raw: Raw response body bytes.

    Returns:
        Decoded JSON payload, or None for an empty body.
    """
    if not raw:
        return None
    return json.loads(raw.decode("utf-8"))


def first_set_cookie(headers: Message) -> str:
    """
    Return the first Set-Cookie header.

    Args:
        headers: Response headers.

    Returns:
        First Set-Cookie header value.
    """
    cookies = headers.get_all("Set-Cookie") or []
    if not cookies:
        raise AssertionError("Expected a Set-Cookie header")
    return cookies[0]


def cookie_pair(set_cookie: str) -> str:
    """
    Return the name=value cookie pair from a Set-Cookie header.

    Args:
        set_cookie: Set-Cookie header value.

    Returns:
        Cookie pair suitable for a Cookie request header.
    """
    return set_cookie.split(";", maxsplit=1)[0]


class AuthApiContractTest(unittest.TestCase):
    """Compare migrated Rust auth behavior with Python-created auth state."""

    def test_existing_python_user_and_token_authenticate_through_rust(self) -> None:
        """
        Verify login, cookie auth, Bearer auth, and logout token revocation.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            user = auth_db.create_user("alice", "secret123")
            bearer_token = auth_db.create_access_token(int(user["id"]), name="api")
            app = FastAPI()
            register_routes(app)
            client = TestClient(app)
            port = available_port()
            process = start_rust_api(project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            try:
                missing_status, missing_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/me",
                )
                invalid_format_status, invalid_format_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/me",
                    headers={"Authorization": "Token invalid"},
                )
                bad_login_status, bad_login_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/login",
                    {"username": "alice", "password": "wrong"},
                )
                login_status, login_payload, login_headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/login",
                    {"username": "alice", "password": "secret123"},
                )
                set_cookie = first_set_cookie(login_headers)
                session_cookie = cookie_pair(set_cookie)

                self.assertEqual(login_status, 200)
                self.assertEqual(missing_status, 401)
                self.assertEqual(missing_payload, {"detail": "Authentication required"})
                self.assertEqual(invalid_format_status, 401)
                self.assertEqual(
                    invalid_format_payload,
                    {"detail": "Invalid authorization format"},
                )
                self.assertEqual(bad_login_status, 401)
                self.assertEqual(
                    bad_login_payload,
                    {"detail": "Invalid username or password"},
                )
                self.assertEqual(
                    login_payload["user"],
                    {"id": int(user["id"]), "username": "alice", "is_admin": False},
                )
                self.assertIn("expires_at", login_payload)
                self.assertNotIn("token", login_payload)
                self.assertIn("HttpOnly", set_cookie)
                self.assertIn("SameSite=lax", set_cookie)
                self.assertIn("Path=/", set_cookie)

                me_status, me_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/me",
                    headers={"Cookie": session_cookie},
                )
                python_me = client.get(
                    "/api/auth/me",
                    headers={"Authorization": f"Bearer {bearer_token['token']}"},
                )
                bearer_status, bearer_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/me",
                    headers={"Authorization": f"Bearer {bearer_token['token']}"},
                )

                self.assertEqual(me_status, 200)
                self.assertEqual(me_payload, login_payload["user"])
                self.assertEqual(bearer_status, python_me.status_code)
                self.assertEqual(bearer_payload, python_me.json())

                logout_status, logout_payload, logout_headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/logout",
                    headers={"Cookie": session_cookie},
                )
                cleared_cookie = first_set_cookie(logout_headers)
                revoked_status, revoked_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/me",
                    headers={"Cookie": session_cookie},
                )

                self.assertEqual(logout_status, 200)
                self.assertEqual(
                    logout_payload, {"ok": True, "user_id": int(user["id"])}
                )
                self.assertIn("Max-Age=0", cleared_cookie)
                self.assertEqual(revoked_status, 401)
                self.assertEqual(
                    revoked_payload, {"detail": "Invalid or expired token"}
                )
            finally:
                stop_process(process)
                client.close()

    def test_registration_invites_tokens_and_password_change_match_contracts(
        self,
    ) -> None:
        """
        Verify first-user admin, invite flow, token management, and password change.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            port = available_port()
            process = start_rust_api(project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            try:
                required_status, required_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/invite-required",
                )
                register_status, owner_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/register",
                    {
                        "username": "owner",
                        "password": "secret123",
                        "invite_code": "",
                    },
                )
                login_status, _login_payload, login_headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/login",
                    {"username": "owner", "password": "secret123"},
                )
                owner_cookie = cookie_pair(first_set_cookie(login_headers))
                invite_status, invite_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/invite-code",
                    headers={"Cookie": owner_cookie},
                )
                second_status, second_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/register",
                    {
                        "username": "second",
                        "password": "secret123",
                        "invite_code": invite_payload["code"],
                    },
                )
                used_invite_status, used_invite_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/invite-code",
                    headers={"Cookie": owner_cookie},
                )
                token_status, token_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/tokens",
                    {"name": "cli", "ttl": 10},
                    headers={"Cookie": owner_cookie},
                )
                tokens_status, tokens_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/tokens",
                    headers={"Cookie": owner_cookie},
                )
                delete_status, delete_payload, _headers = request_json(
                    "DELETE",
                    f"{base_url}/api/auth/tokens/{token_payload['id']}",
                    headers={"Cookie": owner_cookie},
                )
                change_status, change_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/change-password",
                    {"old_password": "secret123", "new_password": "newsecret"},
                    headers={"Cookie": owner_cookie},
                )
                old_cookie_status, old_cookie_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/auth/me",
                    headers={"Cookie": owner_cookie},
                )
                new_login_status, _new_login_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/auth/login",
                    {"username": "owner", "password": "newsecret"},
                )

                self.assertEqual(required_status, 200)
                self.assertEqual(required_payload, {"required": False})
                self.assertEqual(register_status, 200)
                self.assertTrue(owner_payload["is_admin"])
                self.assertEqual(login_status, 200)
                self.assertEqual(invite_status, 200)
                self.assertFalse(invite_payload["used"])
                self.assertEqual(second_status, 200)
                self.assertFalse(second_payload["is_admin"])
                self.assertEqual(used_invite_status, 200)
                self.assertTrue(used_invite_payload["used"])
                self.assertEqual(token_status, 200)
                self.assertEqual(token_payload["name"], "cli")
                self.assertIn("token", token_payload)
                self.assertIsInstance(token_payload["token"], str)
                self.assertNotEqual(token_payload["token"], "")
                self.assertEqual(tokens_status, 200)
                self.assertEqual([item["name"] for item in tokens_payload], ["cli"])
                self.assertEqual(delete_status, 200)
                self.assertEqual(delete_payload, {"ok": True})
                self.assertEqual(change_status, 200)
                self.assertEqual(change_payload, {"ok": True})
                self.assertEqual(old_cookie_status, 401)
                self.assertEqual(
                    old_cookie_payload, {"detail": "Invalid or expired token"}
                )
                self.assertEqual(new_login_status, 200)
            finally:
                stop_process(process)


if __name__ == "__main__":
    unittest.main()
