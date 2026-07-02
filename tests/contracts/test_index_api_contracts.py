"""Shadow contracts for Rust index read endpoints."""

from __future__ import annotations

import json
import os
import shutil
import sqlite3
import unittest
import urllib.error
import urllib.request
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from email.message import Message
from pathlib import Path
from unittest.mock import patch

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
import paper_scanner.api.queries.weekly as weekly_queries
import paper_scanner.shared.db_path as db_path_module
import paper_scanner.shared.sqlite_ext as sqlite_ext
from paper_scanner.api.routes import register_routes

from .contract_support import (
    CONTRACT_ARTICLE_ID,
    CONTRACT_DB_NAME,
    CONTRACT_JOURNAL_ID,
    assert_json_matches_fixture,
    build_contract_index_database,
    load_json_fixture,
    normalize_dynamic_values,
)
from .test_auth_api_contracts import request_json
from .test_auth_business_api_contracts import bearer_headers, login_cookie
from .test_public_api_contracts import (
    available_port,
    start_rust_api,
    stop_process,
    temporary_auth_database,
)

SIMPLE_DB_NAME = "simple.sqlite"


@dataclass
class IndexContractEnvironment:
    """Fixture environment shared by Python and Rust index API tests."""

    client: TestClient
    project_root: Path
    index_db_path: Path
    bearer_token: dict[str, object]


@contextmanager
def index_contract_environment() -> Iterator[IndexContractEnvironment]:
    """
    Build a temporary auth database, index database, and Python TestClient.

    Yields:
        Shared index contract environment.
    """
    with temporary_auth_database() as project_root:
        index_dir = project_root / "data" / "index"
        push_state_dir = project_root / "data" / "push_state"
        index_db_path = build_contract_index_database(index_dir)
        simple_tokenizer_path = sqlite_ext.resolve_simple_tokenizer_path()
        if simple_tokenizer_path is None:
            raise RuntimeError(
                "simple tokenizer extension is required for T6 contracts"
            )
        build_simple_contract_index_database(
            index_dir,
            index_db_path,
            Path(simple_tokenizer_path),
        )
        seed_weekly_manifest(push_state_dir, index_db_path)
        user = auth_db.create_user("alice", "secret123")
        bearer_token = auth_db.create_access_token(int(user["id"]), name="api")
        app = FastAPI()
        with (
            patch.object(db_path_module, "INDEX_DIR", index_dir),
            patch.object(weekly_queries, "INDEX_DIR", index_dir),
            patch.object(weekly_queries, "PUSH_STATE_DIR", push_state_dir),
            patch.dict(
                os.environ,
                {"SIMPLE_TOKENIZER_PATH": str(simple_tokenizer_path)},
            ),
        ):
            register_routes(app)
            client = TestClient(app)
            try:
                yield IndexContractEnvironment(
                    client=client,
                    project_root=project_root,
                    index_db_path=index_db_path,
                    bearer_token=bearer_token,
                )
            finally:
                client.close()


def seed_weekly_manifest(push_state_dir: Path, index_db_path: Path) -> None:
    """
    Write one deterministic weekly changes manifest.

    Args:
        push_state_dir: Directory that stores weekly manifest files.
        index_db_path: Contract index database path.

    Returns:
        None.
    """
    push_state_dir.mkdir(parents=True, exist_ok=True)
    payload = {
        "run_id": "2026-07-02T12:00:00Z",
        "generated_at": "2026-07-02T12:00:00Z",
        "db_name": CONTRACT_DB_NAME,
        "db_path": str(index_db_path),
        "notifiable_article_ids": [CONTRACT_ARTICLE_ID, CONTRACT_ARTICLE_ID],
        "backfill_article_ids": [9007199254740994],
    }
    (push_state_dir / "contract.changes.json").write_text(
        json.dumps(payload),
        encoding="utf-8",
    )


def build_simple_contract_index_database(
    index_dir: Path,
    source_db_path: Path,
    simple_tokenizer_path: Path,
) -> Path:
    """
    Build a contract database whose article_search table uses simple tokenizer.

    Args:
        index_dir: Directory that stores index fixture databases.
        source_db_path: Source contract database path.
        simple_tokenizer_path: SQLite extension path for the simple tokenizer.

    Returns:
        Created simple-tokenizer database path.
    """
    simple_db_path = index_dir / SIMPLE_DB_NAME
    shutil.copyfile(source_db_path, simple_db_path)
    with sqlite3.connect(simple_db_path) as connection:
        connection.enable_load_extension(True)
        connection.load_extension(str(simple_tokenizer_path))
        rows = connection.execute(
            """
            SELECT rowid, article_id, title, abstract, doi, authors, journal_title
            FROM article_search
            """
        ).fetchall()
        connection.execute("DROP TABLE article_search")
        connection.execute(
            """
            CREATE VIRTUAL TABLE article_search USING fts5(
                article_id UNINDEXED,
                title,
                abstract,
                doi,
                authors,
                journal_title,
                tokenize = 'simple'
            )
            """
        )
        connection.executemany(
            """
            INSERT INTO article_search (
                rowid,
                article_id,
                title,
                abstract,
                doi,
                authors,
                journal_title
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            rows,
        )
        connection.commit()
    return simple_db_path


def request_redirect(url: str, headers: dict[str, str]) -> tuple[int, Message]:
    """
    Fetch a URL without following redirects.

    Args:
        url: Target URL.
        headers: Request headers.

    Returns:
        HTTP status code and response headers.
    """

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        """Redirect handler that exposes 3xx responses to the caller."""

        def redirect_request(
            self,
            req: urllib.request.Request,
            fp: object,
            code: int,
            msg: str,
            headers: Message,
            newurl: str,
        ) -> None:
            """
            Stop urllib from following redirect responses.

            Args:
                req: Original request.
                fp: Response file object.
                code: HTTP status code.
                msg: HTTP status message.
                headers: Response headers.
                newurl: Redirect target.

            Returns:
                None.
            """
            del req, fp, code, msg, headers, newurl
            return None

    request = urllib.request.Request(url, method="GET")
    for key, value in headers.items():
        request.add_header(key, value)
    opener = urllib.request.build_opener(NoRedirect)
    try:
        with opener.open(request, timeout=20.0) as response:
            return int(response.status), response.headers
    except urllib.error.HTTPError as error:
        return int(error.code), error.headers


def disable_article_listing(index_db_path: Path) -> None:
    """
    Force article queries down the direct table path.

    Args:
        index_db_path: Contract index database path.

    Returns:
        None.
    """
    with sqlite3.connect(index_db_path) as connection:
        connection.execute("UPDATE listing_state SET status = 'stale' WHERE id = 1")
        connection.commit()


class IndexApiContractTest(unittest.TestCase):
    """Compare migrated Rust index read endpoints with Python behavior."""

    def test_index_read_routes_match_python_shadow(self) -> None:
        """
        Verify meta, journal, issue, article, access, and weekly routes.

        Returns:
            None.
        """
        golden = load_json_fixture("backend_golden.json")
        with index_contract_environment() as environment:
            port = available_port()
            process = start_rust_api(environment.project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            cookie = login_cookie(base_url, "alice", "secret123")
            rust_headers = {"Cookie": cookie}
            python_headers = bearer_headers(environment.bearer_token)
            try:
                for path in [
                    f"/api/meta/areas?db={CONTRACT_DB_NAME}",
                    f"/api/meta/journals?db={CONTRACT_DB_NAME}",
                    f"/api/meta/sources?db={CONTRACT_DB_NAME}",
                    f"/api/years?db={CONTRACT_DB_NAME}",
                    f"/api/journals?db={CONTRACT_DB_NAME}&area=systems&limit=5",
                    f"/api/journals/{CONTRACT_JOURNAL_ID}?db={CONTRACT_DB_NAME}",
                    f"/api/issues?db={CONTRACT_DB_NAME}&journal_id={CONTRACT_JOURNAL_ID}",
                    f"/api/issues/101?db={CONTRACT_DB_NAME}",
                    f"/api/articles/{CONTRACT_ARTICLE_ID}?db={CONTRACT_DB_NAME}",
                    f"/api/articles/{CONTRACT_ARTICLE_ID}/access?db={CONTRACT_DB_NAME}",
                ]:
                    python_response = environment.client.get(
                        path, headers=python_headers
                    )
                    rust_status, rust_payload, _headers = request_json(
                        "GET",
                        f"{base_url}{path}",
                        headers=rust_headers,
                    )

                    self.assertEqual(rust_status, python_response.status_code, path)
                    self.assertEqual(rust_payload, python_response.json(), path)

                article_page_path = (
                    f"/api/articles?db={CONTRACT_DB_NAME}&limit=1&include_total=false"
                )
                page_status, page_payload, _headers = request_json(
                    "GET",
                    f"{base_url}{article_page_path}",
                    headers=rust_headers,
                )
                self.assertEqual(page_status, 200)
                assert_json_matches_fixture(
                    self,
                    page_payload,
                    golden["api"]["article_page"]["json"],
                )

                search_path = f"/api/articles?db={CONTRACT_DB_NAME}&q=rust&limit=5"
                python_search = environment.client.get(
                    search_path, headers=python_headers
                )
                search_status, search_payload, _headers = request_json(
                    "GET",
                    f"{base_url}{search_path}",
                    headers=rust_headers,
                )
                self.assertEqual(search_status, python_search.status_code)
                self.assertEqual(search_payload, python_search.json())
                self.assertEqual(
                    [item["article_id"] for item in search_payload["items"]],
                    golden["api"]["article_search"]["article_ids"],
                )

                simple_search_path = f"/api/articles?db={SIMPLE_DB_NAME}&q=rust&limit=5"
                python_simple_search = environment.client.get(
                    simple_search_path,
                    headers=python_headers,
                )
                simple_status, simple_payload, _headers = request_json(
                    "GET",
                    f"{base_url}{simple_search_path}",
                    headers=rust_headers,
                )
                self.assertEqual(simple_status, python_simple_search.status_code)
                self.assertEqual(simple_payload, python_simple_search.json())
                self.assertEqual(
                    [item["article_id"] for item in simple_payload["items"]],
                    golden["api"]["article_search"]["article_ids"],
                )

                disable_article_listing(environment.index_db_path)
                python_fallback = environment.client.get(
                    search_path, headers=python_headers
                )
                fallback_status, fallback_payload, _headers = request_json(
                    "GET",
                    f"{base_url}{search_path}",
                    headers=rust_headers,
                )
                self.assertEqual(fallback_status, python_fallback.status_code)
                self.assertEqual(fallback_payload, python_fallback.json())

                weekly_path = "/api/weekly-updates"
                python_weekly = environment.client.get(
                    weekly_path, headers=python_headers
                )
                weekly_status, weekly_payload, _headers = request_json(
                    "GET",
                    f"{base_url}{weekly_path}",
                    headers=rust_headers,
                )
                self.assertEqual(weekly_status, python_weekly.status_code)
                self.assertEqual(
                    normalize_dynamic_values(weekly_payload),
                    normalize_dynamic_values(python_weekly.json()),
                )

                redirect_status, redirect_headers = request_redirect(
                    (
                        f"{base_url}/api/articles/{CONTRACT_ARTICLE_ID}/fulltext"
                        f"?db={CONTRACT_DB_NAME}"
                    ),
                    rust_headers,
                )
                self.assertEqual(
                    redirect_status,
                    golden["api"]["fulltext_redirect"]["status_code"],
                )
                self.assertEqual(
                    redirect_headers["Location"],
                    golden["api"]["fulltext_redirect"]["location"],
                )

                missing_status, missing_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/articles?db=missing",
                    headers=rust_headers,
                )
                self.assertEqual(
                    missing_status,
                    golden["api"]["db_selection_errors"]["missing"]["status_code"],
                )
                self.assertEqual(
                    missing_payload,
                    golden["api"]["db_selection_errors"]["missing"]["json"],
                )

                (environment.project_root / "data" / "index" / "other.sqlite").touch()
                multiple_status, multiple_payload, _headers = request_json(
                    "GET",
                    f"{base_url}/api/articles",
                    headers=rust_headers,
                )
                self.assertEqual(
                    multiple_status,
                    golden["api"]["db_selection_errors"]["multiple"]["status_code"],
                )
                self.assertEqual(
                    multiple_payload,
                    golden["api"]["db_selection_errors"]["multiple"]["json"],
                )
            finally:
                stop_process(process)


if __name__ == "__main__":
    unittest.main()
