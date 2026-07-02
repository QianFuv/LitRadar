"""Golden backend contracts for the Rust migration."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
import unittest
from unittest.mock import patch

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api import scheduler
from paper_scanner.index.changes import write_change_manifest
from paper_scanner.index.main import index_error_summary
from paper_scanner.index.stats import IndexStatsRecorder
from paper_scanner.notify.workflow import run_notification
from paper_scanner.shared.converters import to_int_stable
from paper_scanner.sources.zjlib_cnki import DownloadedPdf

from .contract_support import (
    CONTRACT_ARTICLE_ID,
    CONTRACT_CNKI_ARTICLE_ID,
    CONTRACT_CNKI_JOURNAL_ID,
    CONTRACT_DB_NAME,
    CONTRACT_INPRESS_ARTICLE_ID,
    CONTRACT_JOURNAL_ID,
    CONTRACT_OLD_ARTICLE_ID,
    assert_json_matches_fixture,
    build_cookie,
    build_unsigned_jwt,
    isolated_contract_app,
    load_json_fixture,
    normalize_dynamic_values,
)


class BackendApiContractTest(unittest.TestCase):
    """Verify golden API contracts used by the Rust migration."""

    @classmethod
    def setUpClass(cls) -> None:
        """
        Create an isolated app for API contract tests.

        Returns:
            None.
        """
        cls.contract_context = isolated_contract_app()
        cls.contract = cls.contract_context.__enter__()

    @classmethod
    def tearDownClass(cls) -> None:
        """
        Dispose the isolated API app.

        Returns:
            None.
        """
        cls.contract_context.__exit__(None, None, None)

    def setUp(self) -> None:
        """
        Load the golden fixture.

        Returns:
            None.
        """
        self.golden = load_json_fixture("backend_golden.json")

    def test_article_page_search_and_file_responses_match_golden(self) -> None:
        """
        Pin article IDs, pagination, FTS search, redirects, and PDF metadata.

        Returns:
            None.
        """
        article_page_response = self.contract.client.get(
            "/api/articles",
            params={
                "db": CONTRACT_DB_NAME,
                "limit": "1",
                "include_total": "false",
            },
        )
        article_page = article_page_response.json()

        self.assertEqual(article_page_response.status_code, 200)
        assert_json_matches_fixture(
            self,
            article_page,
            self.golden["api"]["article_page"]["json"],
        )
        self.assertIsInstance(article_page["items"][0]["article_id"], str)
        self.assertIsInstance(article_page["items"][0]["journal_id"], str)
        self.assertNotIsInstance(article_page["items"][0]["article_id"], int)

        search_response = self.contract.client.get(
            "/api/articles",
            params={"db": CONTRACT_DB_NAME, "q": "rust", "limit": "5"},
        )
        self.assertEqual(search_response.status_code, 200)
        self.assertEqual(
            [item["article_id"] for item in search_response.json()["items"]],
            self.golden["api"]["article_search"]["article_ids"],
        )

        redirect_response = self.contract.client.get(
            f"/api/articles/{CONTRACT_ARTICLE_ID}/fulltext",
            params={"db": CONTRACT_DB_NAME},
            follow_redirects=False,
        )
        self.assertEqual(
            {
                "status_code": redirect_response.status_code,
                "location": redirect_response.headers.get("location"),
            },
            self.golden["api"]["fulltext_redirect"],
        )

        self._store_active_cnki_session()
        downloaded = DownloadedPdf(
            filename="Golden CNKI.pdf",
            final_url="https://example.test/cnki.pdf",
            content_type="application/pdf",
            byte_count=8,
            content=b"%PDF-1.7",
        )
        with patch(
            "paper_scanner.api.queries.articles._download_cnki_fulltext_pdf",
            return_value=(
                downloaded,
                {
                    "bff_user_token": build_unsigned_jwt(int(time.time()) + 3600),
                    "qr_uuid": "qr-contract",
                    "cookies": [build_cookie("userToken", "SECRET_COOKIE_VALUE")],
                },
            ),
        ):
            pdf_response = self.contract.client.get(
                f"/api/articles/{CONTRACT_CNKI_ARTICLE_ID}/fulltext",
                params={"db": CONTRACT_DB_NAME},
            )

        self.assertEqual(pdf_response.status_code, 200)
        self.assertEqual(
            {
                "status_code": pdf_response.status_code,
                "content_type": pdf_response.headers.get("content-type"),
                "content_disposition": pdf_response.headers.get("content-disposition"),
                "body_prefix": pdf_response.content.decode("ascii"),
            },
            self.golden["api"]["cnki_pdf_response"],
        )

    def test_db_selection_auth_favorites_and_cnki_status_match_golden(self) -> None:
        """
        Pin DB selection errors, cookie auth, favorite quirks, and safe CNKI status.

        Returns:
            None.
        """
        (self.contract.index_dir / "other.sqlite").touch()

        multiple_response = self.contract.client.get("/api/articles")
        missing_response = self.contract.client.get(
            "/api/articles",
            params={"db": "missing"},
        )
        self.assertEqual(
            {
                "multiple": {
                    "status_code": multiple_response.status_code,
                    "json": multiple_response.json(),
                },
                "missing": {
                    "status_code": missing_response.status_code,
                    "json": missing_response.json(),
                },
            },
            self.golden["api"]["db_selection_errors"],
        )

        me_response = self.contract.client.get("/api/auth/me")
        self.assertEqual(me_response.status_code, 200)
        self.assertEqual(me_response.json(), self.golden["api"]["auth_me"])

        folder_response = self.contract.client.post(
            "/api/favorites/folders",
            json={"name": "Contracts", "is_tracking": False},
        )
        self.assertEqual(folder_response.status_code, 200)
        folder_id = int(folder_response.json()["id"])

        bulk_add_response = self.contract.client.post(
            f"/api/favorites/folders/{folder_id}/articles/bulk",
            json={
                "articles": [
                    {
                        "article_id": str(CONTRACT_ARTICLE_ID),
                        "db_name": CONTRACT_DB_NAME,
                        "note": "golden",
                    }
                ]
            },
        )
        batch_response = self.contract.client.post(
            "/api/favorites/check/batch",
            json={
                "article_ids": [str(CONTRACT_ARTICLE_ID)],
                "db_name": CONTRACT_DB_NAME,
            },
        )
        bulk_remove_response = self.contract.client.post(
            f"/api/favorites/folders/{folder_id}/articles/bulk-remove",
            json={
                "articles": [
                    {
                        "article_id": str(CONTRACT_ARTICLE_ID),
                        "db_name": CONTRACT_DB_NAME,
                    }
                ]
            },
        )

        self.assertEqual(bulk_add_response.json(), self.golden["api"]["bulk_add"])
        self.assertEqual(
            batch_response.json(),
            self.golden["api"]["favorite_batch_check"],
        )
        self.assertEqual(
            bulk_remove_response.json(),
            self.golden["api"]["bulk_remove"],
        )

        self._store_active_cnki_session()
        cnki_response = self.contract.client.get("/api/cnki/session")
        cnki_payload = cnki_response.json()

        self.assertEqual(cnki_response.status_code, 200)
        self.assertNotIn("SECRET_COOKIE_VALUE", json.dumps(cnki_payload))
        assert_json_matches_fixture(
            self,
            cnki_payload,
            self.golden["api"]["cnki_session_status"],
        )

    def _store_active_cnki_session(self) -> None:
        """
        Store an active CNKI session for the fixture user.

        Returns:
            None.
        """
        auth_db.delete_cnki_session(int(self.contract.user["id"]))
        auth_db.upsert_cnki_session(
            int(self.contract.user["id"]),
            {
                "bff_user_token": build_unsigned_jwt(int(time.time()) + 3600),
                "qr_uuid": "qr-contract",
                "cookies": [build_cookie("userToken", "SECRET_COOKIE_VALUE")],
            },
            status="active",
        )


class BackendWorkerAndManifestContractTest(unittest.TestCase):
    """Verify worker, state-file, manifest, and helper contracts."""

    @classmethod
    def setUpClass(cls) -> None:
        """
        Create an isolated app for worker and helper contract tests.

        Returns:
            None.
        """
        cls.contract_context = isolated_contract_app()
        cls.contract = cls.contract_context.__enter__()

    @classmethod
    def tearDownClass(cls) -> None:
        """
        Dispose the isolated worker app.

        Returns:
            None.
        """
        cls.contract_context.__exit__(None, None, None)

    def setUp(self) -> None:
        """
        Load the golden fixture.

        Returns:
            None.
        """
        self.golden = load_json_fixture("backend_golden.json")

    def test_scheduler_status_writeback_matches_golden(self) -> None:
        """
        Pin scheduled command exit-code writeback semantics.

        Returns:
            None.
        """
        command = subprocess.list2cmdline(
            [sys.executable, "-c", "import sys; sys.exit(7)"]
        )
        task = auth_db.create_scheduled_task(
            "failing contract command",
            command,
            "* * * * *",
            enabled=True,
        )

        did_run = scheduler.run_task_now(int(task["id"]))
        updated = auth_db.get_scheduled_task(int(task["id"]))

        self.assertTrue(did_run)
        assert updated is not None
        assert_json_matches_fixture(
            self,
            normalize_dynamic_values(updated),
            self.golden["worker"]["scheduled_task_failure"],
        )

    def test_notification_cli_state_and_change_manifest_match_golden(self) -> None:
        """
        Pin notification state-file and index change-manifest shapes.

        Returns:
            None.
        """
        state_dir = self.contract.root_path / "push_state"
        args = argparse.Namespace(
            db=CONTRACT_DB_NAME,
            changes_file="",
            state_dir=str(state_dir),
            ai_model="",
            max_candidates=None,
            timeout=1.0,
            retries=1,
            dedupe_retention_days=30,
            dry_run=True,
        )

        exit_code = run_notification(args)
        state_path = state_dir / "contract.json"
        with open(state_path, encoding="utf-8") as handle:
            state_payload = json.load(handle)

        self.assertEqual(exit_code, 0)
        assert_json_matches_fixture(
            self,
            state_payload,
            self.golden["worker"]["notify_state_skipped"],
        )

        changed_issue_keys = [f"{CONTRACT_JOURNAL_ID}:101"]
        changed_inpress_ids = [CONTRACT_JOURNAL_ID]
        summary = {
            "changed_issue_count": 1,
            "changed_inpress_count": 1,
            "added_article_count": 2,
            "removed_article_count": 0,
            "added_article_ids": [
                CONTRACT_OLD_ARTICLE_ID,
                CONTRACT_INPRESS_ARTICLE_ID,
            ],
            "removed_article_ids": [],
            "issues": [
                {
                    "issue_key": f"{CONTRACT_JOURNAL_ID}:101",
                    "before_count": 1,
                    "after_count": 2,
                    "added_article_ids": [CONTRACT_OLD_ARTICLE_ID],
                    "removed_article_ids": [],
                }
            ],
            "inpress": [
                {
                    "journal_id": CONTRACT_JOURNAL_ID,
                    "before_count": 0,
                    "after_count": 1,
                    "added_article_ids": [CONTRACT_INPRESS_ARTICLE_ID],
                    "removed_article_ids": [],
                }
            ],
        }
        manifest_path = write_change_manifest(
            self.contract.index_db_path,
            changed_issue_keys,
            changed_inpress_ids,
            summary,
        )
        with open(manifest_path, encoding="utf-8") as handle:
            manifest_payload = json.load(handle)

        assert_json_matches_fixture(
            self,
            manifest_payload,
            self.golden["worker"]["change_manifest"],
        )

    def test_stats_stable_ids_and_recorded_payloads_are_contract_safe(self) -> None:
        """
        Pin stable IDs, stats redaction, and offline upstream fixtures.

        Returns:
            None.
        """
        stable_ids = {
            "article_doi": to_int_stable("10.1000/golden", "article"),
            "journal_title": to_int_stable("Golden Systems Journal", "journal"),
        }
        self.assertEqual(stable_ids, self.golden["helpers"]["stable_ids"])

        recorder = IndexStatsRecorder(
            run_id="run-contract",
            csv_file="contracts.csv",
            started_at="2026-07-02T00:00:00",
        )
        api_key = recorder.record_api_call(
            "openalex",
            "works",
            "GET",
            "https://api.openalex.org/works?api_key=SECRET&token=TOKEN",
        )
        recorder.record_api_attempt(
            api_key,
            status_code=429,
            did_succeed=False,
            elapsed_ms=12.0,
            error="https://api.openalex.org/works?api_key=SECRET&token=TOKEN",
            did_retry=True,
        )
        recorder.record_api_attempt(
            api_key,
            status_code=200,
            did_succeed=True,
            elapsed_ms=24.0,
        )

        stats_payload = recorder.to_dict()
        stats_text = json.dumps(stats_payload)
        summary = index_error_summary(
            [
                (
                    "Client error for url "
                    "'https://api.openalex.org/works?api_key=SECRET&token=TOKEN'"
                )
            ]
        )

        self.assertNotIn("SECRET", stats_text)
        self.assertNotIn("TOKEN", stats_text)
        self.assertEqual(summary, self.golden["helpers"]["redacted_error_summary"])

        api_stats = stats_payload["api_stats"][0]
        self.assertEqual(
            {
                "logical_calls": api_stats["logical_calls"],
                "attempts": api_stats["attempts"],
                "successes": api_stats["successes"],
                "failures": api_stats["failures"],
                "retry_count": api_stats["retry_count"],
                "status_codes": api_stats["status_codes"],
                "url_path": api_stats["key"]["url_path"],
            },
            self.golden["helpers"]["stats_summary"],
        )

        upstream_payloads = load_json_fixture("recorded_http/upstream_responses.json")
        self.assertEqual(
            sorted(upstream_payloads),
            [
                "cnki",
                "crossref",
                "openai",
                "openalex",
                "pushplus",
                "semantic_scholar",
                "zjlib",
            ],
        )
        self.assertNotIn("SECRET", json.dumps(upstream_payloads))
        self.assertIn(str(CONTRACT_CNKI_JOURNAL_ID), json.dumps(upstream_payloads))


if __name__ == "__main__":
    unittest.main()
