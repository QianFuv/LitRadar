"""Shared fixtures for backend migration contract tests."""

from __future__ import annotations

import base64
import json
import sqlite3
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from unittest.mock import patch

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
import paper_scanner.shared.db_path as db_path_module
import paper_scanner.shared.sqlite_ext as sqlite_ext
from paper_scanner.api.auth_deps import SESSION_COOKIE_NAME
from paper_scanner.api.routes import register_routes

FIXTURE_ROOT = Path(__file__).resolve().parents[1] / "fixtures" / "contracts"
CONTRACT_DB_NAME = "contract.sqlite"
CONTRACT_ARTICLE_ID = 9_007_199_254_740_995
CONTRACT_OLD_ARTICLE_ID = 9_007_199_254_740_994
CONTRACT_INPRESS_ARTICLE_ID = 9_007_199_254_740_996
CONTRACT_CNKI_ARTICLE_ID = 9_007_199_254_740_997
CONTRACT_JOURNAL_ID = 9_007_199_254_740_993
CONTRACT_CNKI_JOURNAL_ID = 9_007_199_254_740_998
DYNAMIC_KEYS = {
    "completed_at",
    "created_at",
    "expires_at",
    "generated_at",
    "last_completed_run_at",
    "last_run_at",
    "last_used_at",
    "run_id",
    "started_at",
    "updated_at",
}


@dataclass
class ContractApp:
    """Isolated API app and fixture paths for contract tests."""

    client: TestClient
    root_path: Path
    auth_db_path: Path
    index_dir: Path
    index_db_path: Path
    user: dict[str, Any]


@dataclass
class FastCnkiClientInfo:
    """Safe CNKI client metadata used by fixture-only session tests."""

    has_bff_user_token: bool
    bff_user_token_exp: float | None
    cookie_names: list[str]


class FastCnkiClient:
    """Fixture CNKI client that exposes local session metadata only."""

    def __init__(self, *args: object, state_data: dict | None = None) -> None:
        """
        Store serialized session data without opening network clients.

        Args:
            args: Ignored positional arguments.
            state_data: Serialized CNKI session data.
        """
        del args
        self.state_data = state_data or {}

    def client_info(self) -> FastCnkiClientInfo:
        """
        Return safe metadata derived from serialized session data.

        Returns:
            Safe CNKI client metadata.
        """
        token = str(self.state_data.get("bff_user_token") or "")
        cookies = self.state_data.get("cookies")
        cookie_names: list[str] = []
        if isinstance(cookies, list):
            for cookie in cookies:
                if not isinstance(cookie, dict):
                    continue
                name = str(cookie.get("name") or "").strip()
                if name:
                    cookie_names.append(name)
        return FastCnkiClientInfo(
            has_bff_user_token=bool(token),
            bff_user_token_exp=parse_jwt_expiration(token),
            cookie_names=cookie_names,
        )

    def close(self) -> None:
        """
        Close the fixture client.

        Returns:
            None.
        """


def parse_jwt_expiration(token: str) -> float | None:
    """
    Parse an unsigned JWT-like token expiration value.

    Args:
        token: JWT-like token string.

    Returns:
        Expiration timestamp, or None when absent.
    """
    parts = token.split(".")
    if len(parts) < 2 or not parts[1]:
        return None
    padded_payload = parts[1] + "=" * (-len(parts[1]) % 4)
    try:
        payload = json.loads(base64.urlsafe_b64decode(padded_payload))
    except (ValueError, TypeError):
        return None
    expires_at = payload.get("exp") if isinstance(payload, dict) else None
    if not isinstance(expires_at, int | float):
        return None
    return float(expires_at)


def load_json_fixture(relative_path: str) -> dict[str, Any]:
    """
    Load one JSON contract fixture.

    Args:
        relative_path: Fixture path relative to the contract fixture root.

    Returns:
        Parsed JSON object.
    """
    path = FIXTURE_ROOT / relative_path
    with open(path, encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise TypeError(f"Fixture must be a JSON object: {path}")
    return payload


def build_contract_index_database(index_dir: Path) -> Path:
    """
    Create a deterministic index SQLite database from the SQL fixture.

    Args:
        index_dir: Directory that should contain fixture index databases.

    Returns:
        Created SQLite database path.
    """
    index_dir.mkdir(parents=True, exist_ok=True)
    db_path = index_dir / CONTRACT_DB_NAME
    sql_path = FIXTURE_ROOT / "index_fixture.sql"
    with open(sql_path, encoding="utf-8") as handle:
        script = handle.read()
    with sqlite3.connect(db_path) as connection:
        connection.executescript(script)
    return db_path


def build_unsigned_jwt(expires_at: int) -> str:
    """
    Build an unsigned JWT-like token for safe CNKI status contracts.

    Args:
        expires_at: Expiration timestamp.

    Returns:
        JWT-like token string.
    """

    def encode(payload: dict[str, object]) -> str:
        """
        Encode one JWT segment without padding.

        Args:
            payload: Segment payload.

        Returns:
            Base64-url encoded segment.
        """
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        return base64.urlsafe_b64encode(body).decode("ascii").rstrip("=")

    return f"{encode({'alg': 'none'})}.{encode({'exp': expires_at})}."


def build_cookie(name: str, value: str) -> dict[str, Any]:
    """
    Build serialized cookie data for CNKI session fixtures.

    Args:
        name: Cookie name.
        value: Cookie value.

    Returns:
        JSON-serializable cookie payload.
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


def normalize_dynamic_values(value: Any) -> Any:
    """
    Replace environment-specific values with stable placeholders.

    Args:
        value: Payload to normalize.

    Returns:
        Payload with timestamps, counters, and temp paths normalized.
    """
    if isinstance(value, dict):
        normalized: dict[str, Any] = {}
        for key, item in value.items():
            if key in DYNAMIC_KEYS and item is not None:
                normalized[key] = "<timestamp>"
            elif key == "seconds_remaining" and item is not None:
                normalized[key] = "<seconds>"
            elif key == "command" and item:
                normalized[key] = "<command>"
            elif key == "db_path" and item:
                normalized[key] = "<db_path>"
            else:
                normalized[key] = normalize_dynamic_values(item)
        return normalized
    if isinstance(value, list):
        return [normalize_dynamic_values(item) for item in value]
    return value


def assert_json_matches_fixture(
    test_case: Any,
    actual: Any,
    expected: Any,
) -> None:
    """
    Assert that a normalized payload matches its golden fixture.

    Args:
        test_case: Active unittest test case.
        actual: Actual payload.
        expected: Expected payload.

    Returns:
        None.
    """
    test_case.maxDiff = None
    test_case.assertEqual(normalize_dynamic_values(actual), expected)


@contextmanager
def isolated_contract_app() -> Iterator[ContractApp]:
    """
    Create an isolated API app, auth database, and index fixture database.

    Yields:
        Isolated contract app context.
    """
    temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
    root_path = Path(temp_dir.name)
    auth_db_path = root_path / "auth.sqlite"
    index_dir = root_path / "index"
    previous_auth_db_path = auth_db.AUTH_DB_PATH
    previous_index_dir = db_path_module.INDEX_DIR
    simple_tokenizer_patch = patch.object(
        sqlite_ext,
        "resolve_simple_tokenizer_path",
        return_value=None,
    )
    cnki_client_patch = patch.object(
        auth_db,
        "ZhejiangLibraryCnkiClient",
        FastCnkiClient,
    )

    try:
        simple_tokenizer_patch.start()
        cnki_client_patch.start()
        auth_db.AUTH_DB_PATH = auth_db_path
        db_path_module.INDEX_DIR = index_dir
        auth_db.init_auth_db()
        user = auth_db.create_user("alice", "secret123")
        index_db_path = build_contract_index_database(index_dir)
        app = FastAPI()
        register_routes(app)
        client = TestClient(app)
        try:
            login_response = client.post(
                "/api/auth/login",
                json={"username": "alice", "password": "secret123"},
            )
            if login_response.status_code != 200:
                raise RuntimeError("Contract app login failed")
            if client.cookies.get(SESSION_COOKIE_NAME) is None:
                raise RuntimeError("Contract app login did not set session cookie")
            yield ContractApp(
                client=client,
                root_path=root_path,
                auth_db_path=auth_db_path,
                index_dir=index_dir,
                index_db_path=index_db_path,
                user=user,
            )
        finally:
            client.close()
    finally:
        cnki_client_patch.stop()
        simple_tokenizer_patch.stop()
        auth_db.AUTH_DB_PATH = previous_auth_db_path
        db_path_module.INDEX_DIR = previous_index_dir
        temp_dir.cleanup()
