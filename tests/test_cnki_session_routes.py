"""Tests for per-user Zhejiang Library CNKI session routes."""

from __future__ import annotations

import base64
import json
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.routes import register_routes
from paper_scanner.sources.zjlib_cnki import QrLogin, ZjlibCnkiError


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


def build_cookie(name: str, value: str) -> dict[str, Any]:
    """
    Build JSON cookie data for stored session tests.

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


class FakeZhejiangLibraryCnkiClient:
    """Fake sync client used by CNKI session route tests."""

    completed_token = build_unsigned_jwt(int(time.time()) + 3600)

    def __init__(
        self, *args: object, state_data: dict | None = None, **kwargs: object
    ) -> None:
        """
        Initialize the fake client.

        Args:
            args: Ignored positional arguments.
            state_data: Existing session state.
            kwargs: Ignored keyword arguments.
        """
        self.state_data = dict(state_data or {})
        self.did_complete = False
        self.did_warm_up = False

    def __enter__(self) -> FakeZhejiangLibraryCnkiClient:
        """
        Enter a context-managed fake client.

        Returns:
            This fake client.
        """
        return self

    def __exit__(self, *_exc: object) -> None:
        """
        Exit a context-managed fake client.

        Returns:
            None.
        """

    def start_qr_login(self) -> QrLogin:
        """
        Return a fake QR login challenge.

        Returns:
            Fake QR login challenge.
        """
        self.state_data["qr_uuid"] = "qr-user-1"
        return QrLogin(
            uuid="qr-user-1",
            status="WAITING_SCAN",
            qr_code="https://qr.test/qr-user-1.png",
        )

    def poll_qr_login(
        self,
        *,
        timeout_seconds: int = 180,
        interval_seconds: float = 2.0,
    ) -> str:
        """
        Complete fake QR login.

        Args:
            timeout_seconds: Ignored timeout.
            interval_seconds: Ignored interval.

        Returns:
            Completed fake token.
        """
        self.did_complete = True
        self.state_data["bff_user_token"] = self.completed_token
        return self.completed_token

    def build_share_sso_url(self) -> str:
        """
        Return a fake Share SSO URL.

        Returns:
            Fake Share SSO URL.
        """
        self.state_data["share_sso_url"] = "https://share.test/sso"
        return str(self.state_data["share_sso_url"])

    def enter_share(self, sso_url: str) -> None:
        """
        Mark fake Share entry as completed.

        Args:
            sso_url: Fake Share SSO URL.

        Returns:
            None.
        """
        self.state_data["entered_share_url"] = sso_url

    def get_zyproxy_login_url(self) -> str:
        """
        Return a fake zyproxy login URL.

        Returns:
            Fake zyproxy login URL.
        """
        self.state_data["zyproxy_login_url"] = "https://login.elib.test/index.php"
        return str(self.state_data["zyproxy_login_url"])

    def enter_zyproxy(self, login_url: str) -> str:
        """
        Mark fake zyproxy entry as completed.

        Args:
            login_url: Fake zyproxy login URL.

        Returns:
            Fake final zyproxy URL.
        """
        self.did_warm_up = True
        self.state_data["entered_zyproxy_url"] = login_url
        self.state_data["final_zyproxy_url"] = "https://cnki.elib.test/kns55/"
        return str(self.state_data["final_zyproxy_url"])

    def warm_up_fulltext_session(self) -> str:
        """
        Run the fake Share and zyproxy warm-up sequence.

        Returns:
            Fake final zyproxy URL.
        """
        sso_url = self.build_share_sso_url()
        self.enter_share(sso_url)
        login_url = self.get_zyproxy_login_url()
        return self.enter_zyproxy(login_url)

    def to_state_data(self) -> dict[str, Any]:
        """
        Return fake session state.

        Returns:
            Fake persisted session data.
        """
        state = dict(self.state_data)
        state.setdefault("qr_uuid", "qr-user-1")
        if self.did_complete:
            state["cookies"] = [build_cookie("userToken", "SECRET_COOKIE_VALUE")]
        if self.did_warm_up:
            state["cookies"] = [
                *state.get("cookies", []),
                build_cookie("vpn358_sid", "SECRET_VPN_VALUE"),
            ]
        else:
            state.setdefault("cookies", [])
        return state


class FailingWarmUpCnkiClient(FakeZhejiangLibraryCnkiClient):
    """Fake client that fails after QR polling succeeds."""

    def warm_up_fulltext_session(self) -> str:
        """
        Raise a fake warm-up error.

        Raises:
            ZjlibCnkiError: Always raised to simulate upstream failure.
        """
        raise ZjlibCnkiError("Share warm-up failed")


class TimeoutQrLoginCnkiClient(FakeZhejiangLibraryCnkiClient):
    """Fake client that times out while waiting for QR confirmation."""

    def poll_qr_login(
        self,
        *,
        timeout_seconds: int = 180,
        interval_seconds: float = 2.0,
    ) -> str:
        """
        Raise a fake QR polling timeout.

        Args:
            timeout_seconds: Ignored timeout.
            interval_seconds: Ignored interval.

        Raises:
            ZjlibCnkiError: Always raised to simulate polling timeout.
        """
        raise ZjlibCnkiError("Timed out waiting for QR scan after 15 seconds.")


class CnkiSessionRoutesTest(unittest.TestCase):
    """Verify user-scoped CNKI session persistence and route responses."""

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
        self.other_user = auth_db.create_user("bob", "secret123")
        self.token = auth_db.create_access_token(self.user["id"], name="test")["token"]
        self.other_token = auth_db.create_access_token(
            self.other_user["id"], name="test"
        )["token"]
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

    def auth_headers(self, token: str | None = None) -> dict[str, str]:
        """
        Build bearer auth headers.

        Args:
            token: Optional token override.

        Returns:
            Authorization headers.
        """
        return {"Authorization": f"Bearer {token or self.token}"}

    def test_session_status_defaults_to_empty(self) -> None:
        """
        Ensure users without CNKI sessions receive an empty status.

        Returns:
            None.
        """
        response = self.client.get("/api/cnki/session", headers=self.auth_headers())

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "empty")
        self.assertFalse(response.json()["configured"])

    def test_start_login_stores_waiting_session_without_secrets(self) -> None:
        """
        Ensure QR login start stores only the current user's session.

        Returns:
            None.
        """
        with patch(
            "paper_scanner.api.routes.cnki.ZhejiangLibraryCnkiClient",
            FakeZhejiangLibraryCnkiClient,
        ):
            response = self.client.post(
                "/api/cnki/login/start",
                headers=self.auth_headers(),
            )

        payload = response.json()
        other_response = self.client.get(
            "/api/cnki/session",
            headers=self.auth_headers(self.other_token),
        )

        self.assertEqual(response.status_code, 200)
        self.assertEqual(payload["uuid"], "qr-user-1")
        self.assertEqual(payload["session"]["status"], "waiting_scan")
        self.assertNotIn("SECRET", json.dumps(payload, ensure_ascii=False))
        self.assertEqual(other_response.json()["status"], "empty")

    def test_poll_login_marks_session_active_without_returning_values(self) -> None:
        """
        Ensure QR polling persists active session status without API secret leaks.

        Returns:
            None.
        """
        with patch(
            "paper_scanner.api.routes.cnki.ZhejiangLibraryCnkiClient",
            FakeZhejiangLibraryCnkiClient,
        ):
            self.client.post("/api/cnki/login/start", headers=self.auth_headers())
            response = self.client.post(
                "/api/cnki/login/poll",
                json={},
                headers=self.auth_headers(),
            )

        payload = response.json()
        raw_session = auth_db.get_cnki_session(self.user["id"])

        self.assertEqual(response.status_code, 200)
        self.assertEqual(payload["status"], "COMPLETE")
        self.assertEqual(payload["session"]["status"], "active")
        self.assertIn("userToken", payload["session"]["cookie_names"])
        self.assertIn("vpn358_sid", payload["session"]["cookie_names"])
        self.assertNotIn("SECRET_COOKIE_VALUE", json.dumps(payload, ensure_ascii=False))
        self.assertNotIn("SECRET_VPN_VALUE", json.dumps(payload, ensure_ascii=False))
        assert raw_session is not None
        self.assertIn("bff_user_token", raw_session["session_data"])
        self.assertIn("final_zyproxy_url", raw_session["session_data"])

    def test_poll_login_reports_qr_timeout_code(self) -> None:
        """
        Ensure QR polling timeout is distinguishable from warm-up failure.

        Returns:
            None.
        """
        with patch(
            "paper_scanner.api.routes.cnki.ZhejiangLibraryCnkiClient",
            TimeoutQrLoginCnkiClient,
        ):
            self.client.post("/api/cnki/login/start", headers=self.auth_headers())
            response = self.client.post(
                "/api/cnki/login/poll",
                json={},
                headers=self.auth_headers(),
            )

        detail = response.json()["detail"]

        self.assertEqual(response.status_code, 408)
        self.assertEqual(detail["code"], "cnki_login_timeout")
        self.assertEqual(detail["phase"], "login")

    def test_poll_login_reports_fulltext_warm_up_failure(self) -> None:
        """
        Ensure QR login does not silently return an unprepared active session.

        Returns:
            None.
        """
        with patch(
            "paper_scanner.api.routes.cnki.ZhejiangLibraryCnkiClient",
            FailingWarmUpCnkiClient,
        ):
            self.client.post("/api/cnki/login/start", headers=self.auth_headers())
            response = self.client.post(
                "/api/cnki/login/poll",
                json={},
                headers=self.auth_headers(),
            )

        detail = response.json()["detail"]

        self.assertEqual(response.status_code, 502)
        self.assertEqual(detail["code"], "cnki_warmup_failed")
        self.assertEqual(detail["phase"], "warmup")
        self.assertIn("Share warm-up failed", detail["message"])

    def test_expired_session_reports_expired(self) -> None:
        """
        Ensure expired stored tokens are reported as expired.

        Returns:
            None.
        """
        now = time.time()
        auth_db.upsert_cnki_session(
            self.user["id"],
            {
                "bff_user_token": build_unsigned_jwt(int(now) - 10),
                "qr_uuid": "qr-expired",
                "cookies": [build_cookie("userToken", "expired")],
            },
            status="active",
            now=now,
        )

        response = self.client.get("/api/cnki/session", headers=self.auth_headers())

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "expired")
        self.assertEqual(response.json()["seconds_remaining"], 0)

    def test_delete_session_clears_current_user_state(self) -> None:
        """
        Ensure clearing a session removes only the current user's row.

        Returns:
            None.
        """
        auth_db.upsert_cnki_session(
            self.user["id"],
            {"qr_uuid": "qr-user-1", "cookies": []},
            status="waiting_scan",
        )

        response = self.client.delete("/api/cnki/session", headers=self.auth_headers())

        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "empty")
        self.assertIsNone(auth_db.get_cnki_session(self.user["id"]))


if __name__ == "__main__":
    unittest.main()
