"""Shadow contracts for Rust CNKI session and full-text replay routes."""

from __future__ import annotations

import json
import unittest
import urllib.error
import urllib.request
from typing import Any

import paper_scanner.api.auth_db as auth_db

from .contract_support import (
    CONTRACT_CNKI_ARTICLE_ID,
    CONTRACT_DB_NAME,
    build_contract_index_database,
    build_cookie,
    build_unsigned_jwt,
)
from .test_auth_api_contracts import request_json
from .test_auth_business_api_contracts import login_cookie
from .test_public_api_contracts import (
    available_port,
    start_rust_api,
    stop_process,
    temporary_auth_database,
)


def request_bytes(url: str, headers: dict[str, str]) -> tuple[int, bytes, Any]:
    """
    Request bytes from a Rust API URL.

    Args:
        url: Target URL.
        headers: Request headers.

    Returns:
        Status code, response body, and headers.
    """
    request = urllib.request.Request(url, method="GET")
    for key, value in headers.items():
        request.add_header(key, value)
    try:
        with urllib.request.urlopen(request, timeout=20.0) as response:
            return int(response.status), response.read(), response.headers
    except urllib.error.HTTPError as error:
        return int(error.code), error.read(), error.headers


class CnkiApiContractTest(unittest.TestCase):
    """Verify migrated Rust CNKI session and full-text replay behavior."""

    def test_cnki_session_replay_routes_keep_safe_payloads(self) -> None:
        """
        Verify start, poll, status, and delete session behavior.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            auth_db.create_user("alice", "secret123")
            port = available_port()
            process = start_rust_api(
                project_root,
                port,
                {"PAPER_SCANNER_CNKI_REPLAY_MODE": "poll_success"},
            )
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "alice", "secret123")
                headers = {"Cookie": cookie}
                empty_status, empty_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/cnki/session",
                    headers=headers,
                )
                start_status, start_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/cnki/login/start",
                    headers=headers,
                )
                poll_status, poll_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/cnki/login/poll",
                    {},
                    headers=headers,
                )
                delete_status, delete_payload, _headers = request_json(
                    "DELETE",
                    f"{base_url}/api/cnki/session",
                    headers=headers,
                )

                self.assertEqual(empty_status, 200)
                self.assertEqual(empty_payload["status"], "empty")
                self.assertFalse(empty_payload["configured"])
                self.assertEqual(start_status, 200)
                self.assertEqual(start_payload["session"]["status"], "waiting_scan")
                self.assertNotIn("SECRET", json.dumps(start_payload))
                self.assertEqual(poll_status, 200)
                self.assertEqual(poll_payload["status"], "COMPLETE")
                self.assertEqual(poll_payload["session"]["status"], "active")
                self.assertIn("userToken", poll_payload["session"]["cookie_names"])
                self.assertIn("vpn358_sid", poll_payload["session"]["cookie_names"])
                self.assertNotIn("SECRET_COOKIE_VALUE", json.dumps(poll_payload))
                self.assertNotIn("SECRET_VPN_VALUE", json.dumps(poll_payload))
                self.assertEqual(delete_status, 200)
                self.assertEqual(delete_payload["status"], "empty")
            finally:
                stop_process(process)

    def test_cnki_poll_replay_errors_match_python_codes(self) -> None:
        """
        Verify replayed timeout and warm-up error payloads.

        Returns:
            None.
        """
        for mode, expected_status, expected_code, expected_phase in [
            ("timeout", 408, "cnki_login_timeout", "login"),
            ("warmup_failure", 502, "cnki_warmup_failed", "warmup"),
        ]:
            with self.subTest(mode=mode), temporary_auth_database() as project_root:
                auth_db.create_user("alice", "secret123")
                port = available_port()
                process = start_rust_api(
                    project_root,
                    port,
                    {"PAPER_SCANNER_CNKI_REPLAY_MODE": mode},
                )
                base_url = f"http://127.0.0.1:{port}"
                try:
                    cookie = login_cookie(base_url, "alice", "secret123")
                    headers = {"Cookie": cookie}
                    request_json(
                        "POST",
                        f"{base_url}/api/cnki/login/start",
                        headers=headers,
                    )
                    status, payload, _headers = request_json(
                        "POST",
                        f"{base_url}/api/cnki/login/poll",
                        {},
                        headers=headers,
                    )

                    self.assertEqual(status, expected_status)
                    self.assertEqual(payload["detail"]["code"], expected_code)
                    self.assertEqual(payload["detail"]["phase"], expected_phase)
                finally:
                    stop_process(process)

    def test_cnki_start_without_replay_fails_loudly(self) -> None:
        """
        Verify the Rust API does not synthesize QR login success by default.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            auth_db.create_user("alice", "secret123")
            port = available_port()
            process = start_rust_api(project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "alice", "secret123")
                status, payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/cnki/login/start",
                    headers={"Cookie": cookie},
                )

                self.assertEqual(status, 502)
                self.assertEqual(payload["detail"]["code"], "cnki_login_start_failed")
            finally:
                stop_process(process)

    def test_cnki_fulltext_replay_returns_pdf_and_mismatch_error(self) -> None:
        """
        Verify offline PDF replay and exact-match failure behavior.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            user = auth_db.create_user("alice", "secret123")
            index_dir = project_root / "data" / "index"
            build_contract_index_database(index_dir)
            pdf_file = project_root / "data" / "cnki-replay.pdf"
            pdf_file.write_bytes(b"%PDF-1.7")
            auth_db.upsert_cnki_session(
                user["id"],
                {
                    "bff_user_token": build_unsigned_jwt(4_102_444_800),
                    "qr_uuid": "qr-contract",
                    "cookies": [build_cookie("userToken", "SECRET_COOKIE_VALUE")],
                },
                status="active",
            )
            port = available_port()
            process = start_rust_api(
                project_root,
                port,
                {
                    "PAPER_SCANNER_CNKI_PDF_REPLAY_PATH": str(pdf_file),
                    "PAPER_SCANNER_CNKI_PDF_REPLAY_FILENAME": "Golden CNKI.pdf",
                },
            )
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "alice", "secret123")
                status, body, headers = request_bytes(
                    (
                        f"{base_url}/api/articles/{CONTRACT_CNKI_ARTICLE_ID}/fulltext"
                        f"?db={CONTRACT_DB_NAME}"
                    ),
                    {"Cookie": cookie},
                )

                self.assertEqual(status, 200)
                self.assertEqual(body, b"%PDF-1.7")
                self.assertEqual(headers["Content-Type"], "application/pdf")
                self.assertEqual(
                    headers["Content-Disposition"],
                    "attachment; filename*=UTF-8''Golden%20CNKI.pdf",
                )
            finally:
                stop_process(process)

        with temporary_auth_database() as project_root:
            user = auth_db.create_user("alice", "secret123")
            build_contract_index_database(project_root / "data" / "index")
            auth_db.upsert_cnki_session(
                user["id"],
                {
                    "bff_user_token": build_unsigned_jwt(4_102_444_800),
                    "qr_uuid": "qr-contract",
                    "cookies": [build_cookie("userToken", "SECRET_COOKIE_VALUE")],
                },
                status="active",
            )
            port = available_port()
            process = start_rust_api(
                project_root,
                port,
                {"PAPER_SCANNER_CNKI_PDF_REPLAY_MODE": "mismatch"},
            )
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "alice", "secret123")
                status, payload, _headers = request_json(
                    "GET",
                    (
                        f"{base_url}/api/articles/{CONTRACT_CNKI_ARTICLE_ID}/fulltext"
                        f"?db={CONTRACT_DB_NAME}"
                    ),
                    headers={"Cookie": cookie},
                )

                self.assertEqual(status, 404)
                self.assertEqual(
                    payload,
                    {"detail": "No exact CNKI full-text match found"},
                )
            finally:
                stop_process(process)

    def test_expired_cnki_session_does_not_enable_fulltext(self) -> None:
        """
        Verify expired CNKI credentials do not enable service-side full text.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            user = auth_db.create_user("alice", "secret123")
            build_contract_index_database(project_root / "data" / "index")
            auth_db.upsert_cnki_session(
                user["id"],
                {
                    "bff_user_token": build_unsigned_jwt(1),
                    "qr_uuid": "qr-contract",
                    "cookies": [build_cookie("userToken", "SECRET_COOKIE_VALUE")],
                },
                status="active",
            )
            port = available_port()
            process = start_rust_api(project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "alice", "secret123")
                status, payload, _headers = request_json(
                    "GET",
                    (
                        f"{base_url}/api/articles/{CONTRACT_CNKI_ARTICLE_ID}/access"
                        f"?db={CONTRACT_DB_NAME}"
                    ),
                    headers={"Cookie": cookie},
                )

                self.assertEqual(status, 200)
                self.assertFalse(payload["fulltext"]["available"])
                self.assertTrue(payload["fulltext"]["requires_login"])
                self.assertNotIn("SECRET_COOKIE_VALUE", json.dumps(payload))
            finally:
                stop_process(process)


if __name__ == "__main__":
    unittest.main()
