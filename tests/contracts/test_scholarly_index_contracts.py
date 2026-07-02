"""Contracts for Rust Scholarly index migration."""

from __future__ import annotations

import asyncio
import csv
import gc
import json
import os
import shutil
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any, cast

import aiosqlite
import httpx

from paper_scanner.index.changes import (
    collect_article_snapshot,
    compute_changed_group_keys,
    write_change_manifest,
)
from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import persist_index_run_stats
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.fetcher import process_scholarly_journal
from paper_scanner.index.stats import ApiStatsKey, IndexStatsRecorder
from paper_scanner.index.transforms import normalize_doi
from paper_scanner.shared.sqlite_ext import resolve_simple_tokenizer_path

from .contract_support import FIXTURE_ROOT, normalize_dynamic_values
from .test_public_api_contracts import PROJECT_ROOT

SCHOLARLY_FIXTURE_ROOT = FIXTURE_ROOT / "scholarly"
RUN_ID = "run-scholarly-contract"
TIMESTAMP = "2026-07-03T00:00:00Z"


class FixtureScholarlyClient:
    """Python Scholarly client backed by the same JSON fixture as Rust."""

    def __init__(
        self,
        fixture: dict[str, Any],
        stats_recorder: IndexStatsRecorder,
    ) -> None:
        """
        Initialize the fixture Scholarly client.

        Args:
            fixture: Fixture source payloads.
            stats_recorder: Stats recorder used by the Python indexer.

        Returns:
            None.
        """
        self.fixture = fixture
        self.stats_recorder = stats_recorder
        self.openalex_source_lookups = 0

    async def fetch_journal_works(
        self,
        issn: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Return Crossref works or raise a fixture HTTP error.

        Args:
            issn: ISSN lookup candidate.
            from_pub_date: Optional lower publication-date filter.
            until_pub_date: Optional upper publication-date filter.

        Returns:
            Crossref work payloads.
        """
        del from_pub_date, until_pub_date
        status_code = int(self.fixture.get("crossref_status") or 200)
        key = self._record_call(
            "crossref",
            "journal_works",
            "GET",
            f"https://api.crossref.org/journals/{issn}/works",
        )
        if status_code != 200:
            request = httpx.Request(
                "GET", f"https://api.crossref.org/journals/{issn}/works"
            )
            response = httpx.Response(status_code, request=request)
            error = httpx.HTTPStatusError(
                "fixture crossref failure", request=request, response=response
            )
            self._record_attempt(key, status_code, False, error)
            raise error
        self._record_attempt(key, 200, True, None)
        return list(self.fixture.get("crossref_works") or [])

    async def fetch_openalex_source_by_issns(
        self, issns: list[str]
    ) -> dict[str, Any] | None:
        """
        Return an OpenAlex source fixture by ISSN.

        Args:
            issns: ISSN lookup candidates.

        Returns:
            OpenAlex source payload or None.
        """
        self.openalex_source_lookups += 1
        key = self._record_call(
            "openalex",
            "sources",
            "GET",
            "https://api.openalex.org/sources?api_key=SECRET&mailto=a@example.test",
        )
        self._record_attempt(key, 200, True, None)
        return self.fixture.get("openalex_source_by_issns")

    async def fetch_openalex_source_by_title(self, title: str) -> dict[str, Any] | None:
        """
        Return an OpenAlex source fixture by title.

        Args:
            title: Title lookup value.

        Returns:
            OpenAlex source payload or None.
        """
        del title
        key = self._record_call(
            "openalex",
            "source_search",
            "GET",
            "https://api.openalex.org/sources?api_key=SECRET&mailto=a@example.test",
        )
        self._record_attempt(key, 200, True, None)
        return self.fixture.get("openalex_source_by_title")

    async def fetch_openalex_works_by_source(
        self,
        source_id: str,
        from_pub_date: str | None = None,
        until_pub_date: str | None = None,
    ) -> list[dict[str, Any]]:
        """
        Return OpenAlex works for a source.

        Args:
            source_id: OpenAlex source id.
            from_pub_date: Optional lower publication-date filter.
            until_pub_date: Optional upper publication-date filter.

        Returns:
            OpenAlex works.
        """
        del source_id, from_pub_date, until_pub_date
        key = self._record_call(
            "openalex",
            "source_works",
            "GET",
            "https://api.openalex.org/works?api_key=SECRET&mailto=a@example.test",
        )
        self._record_attempt(key, 200, True, None)
        return list(self.fixture.get("openalex_source_works") or [])

    async def fetch_openalex_by_dois(
        self, dois: list[str], batch_size: int = 100
    ) -> dict[str, dict[str, Any]]:
        """
        Return OpenAlex DOI enrichment fixtures.

        Args:
            dois: DOI values.
            batch_size: Requested batch size.

        Returns:
            OpenAlex works keyed by DOI.
        """
        del batch_size
        key = self._record_call(
            "openalex",
            "works",
            "GET",
            "https://api.openalex.org/works?api_key=SECRET&mailto=a@example.test",
        )
        self._record_attempt(key, 200, True, None)
        by_doi = dict(self.fixture.get("openalex_by_doi") or {})
        return {doi: by_doi[doi] for doi in dois if doi in by_doi}

    async def fetch_semantic_scholar_by_dois(
        self, dois: list[str], batch_size: int = 500
    ) -> dict[str, dict[str, Any]]:
        """
        Return Semantic Scholar DOI enrichment fixtures.

        Args:
            dois: DOI values.
            batch_size: Requested batch size.

        Returns:
            Semantic Scholar records keyed by DOI.
        """
        del batch_size
        key = self._record_call(
            "semantic_scholar",
            "paper_batch",
            "POST",
            "https://api.semanticscholar.org/graph/v1/paper/batch"
            "?fields=externalIds,url,isOpenAccess,openAccessPdf,abstract"
            "&x-api-key=SECRET",
        )
        status_code = int(self.fixture.get("semantic_scholar_status") or 200)
        if status_code != 200:
            request = httpx.Request(
                "POST", "https://api.semanticscholar.org/graph/v1/paper/batch"
            )
            response = httpx.Response(
                status_code,
                request=request,
                json={"error": self.fixture.get("semantic_scholar_error") or "error"},
            )
            error = httpx.HTTPStatusError(
                "fixture semantic scholar failure",
                request=request,
                response=response,
            )
            self._record_attempt(key, status_code, False, error)
            raise error
        self._record_attempt(key, 200, True, None)
        by_doi = dict(self.fixture.get("semantic_scholar_by_doi") or {})
        normalized = {normalize_doi(raw): payload for raw, payload in by_doi.items()}
        return {doi: normalized[doi] for doi in dois if doi in normalized}

    def _record_call(
        self, service: str, endpoint: str, method: str, url: str
    ) -> ApiStatsKey:
        """
        Record one logical API call.

        Args:
            service: Service name.
            endpoint: Endpoint name.
            method: HTTP method.
            url: Request URL.

        Returns:
            API stats key.
        """
        return self.stats_recorder.record_api_call(service, endpoint, method, url)

    def _record_attempt(
        self,
        key: ApiStatsKey,
        status_code: int | None,
        did_succeed: bool,
        error: BaseException | str | None,
    ) -> None:
        """
        Record one API attempt.

        Args:
            key: API stats key.
            status_code: HTTP status code.
            did_succeed: Whether the attempt succeeded.
            error: Optional error sample.

        Returns:
            None.
        """
        self.stats_recorder.record_api_attempt(
            key,
            status_code=status_code,
            did_succeed=did_succeed,
            elapsed_ms=0,
            error=error,
        )


def run_ps_cli(
    project_root: Path, args: list[str], check: bool = True
) -> subprocess.CompletedProcess[str]:
    """
    Run the Rust CLI.

    Args:
        project_root: Temporary project root.
        args: CLI arguments after the binary name.
        check: Whether non-zero exit codes should raise.

    Returns:
        Completed process.
    """
    env = os.environ.copy()
    env["PAPER_SCANNER_PROJECT_ROOT"] = str(project_root)
    result = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "ps-cli", "--", *args],
        cwd=PROJECT_ROOT,
        env=env,
        check=check,
        capture_output=True,
        text=True,
    )
    return result


async def build_python_index(
    db_path: Path,
    csv_path: Path,
    fixture_path: Path,
) -> Path:
    """
    Build a Python Scholarly index database and manifest.

    Args:
        db_path: Output database path.
        csv_path: Source CSV path.
        fixture_path: Source fixture path.

    Returns:
        Python manifest path.
    """
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    with open(csv_path, newline="", encoding="utf-8") as handle:
        row = next(csv.DictReader(handle))
    stats_recorder = IndexStatsRecorder(RUN_ID, csv_path.name, started_at=TIMESTAMP)

    async with aiosqlite.connect(db_path) as raw_db:
        await init_db(raw_db)
        db = LocalDatabaseClient(raw_db)
        await db.start()
        try:
            await process_scholarly_journal(
                db,
                cast(Any, FixtureScholarlyClient(fixture, stats_recorder)),
                csv_path,
                row,
                request_workers=4,
                show_year_progress=False,
                resume=False,
                update=False,
                stats_recorder=stats_recorder,
            )
            stats_recorder.stats.finish(
                "succeeded",
                error_summary=None,
                finished_at=TIMESTAMP,
            )
            await persist_index_run_stats(db, stats_recorder.stats)
        finally:
            await db.close()

    before_issue_map: dict[str, set[int]] = {}
    before_inpress_map: dict[int, set[int]] = {}
    after_issue_map, after_inpress_map = collect_article_snapshot(db_path)
    changed_issue_keys, changed_inpress_ids, summary = compute_changed_group_keys(
        before_issue_map,
        after_issue_map,
        before_inpress_map,
        after_inpress_map,
    )
    return write_change_manifest(
        db_path, changed_issue_keys, changed_inpress_ids, summary
    )


def dump_core_rows(db_path: Path) -> dict[str, Any]:
    """
    Dump comparable index rows.

    Args:
        db_path: SQLite database path.

    Returns:
        Comparable table rows.
    """
    queries = {
        "journals": (
            "SELECT journal_id, library_id, platform_journal_id, title, issn, eissn, "
            "available, has_articles FROM journals ORDER BY journal_id"
        ),
        "journal_meta": (
            "SELECT journal_id, source_csv, area, csv_title, csv_issn, csv_library, "
            "resolved_source, resolved_source_id, resolved_title, resolved_issn, "
            "resolved_eissn FROM journal_meta ORDER BY journal_id"
        ),
        "issues": (
            "SELECT issue_id, journal_id, publication_year, title, volume, number, "
            "date, is_valid_issue FROM issues ORDER BY issue_id"
        ),
        "articles": (
            "SELECT article_id, journal_id, issue_id, title, date, authors, "
            "start_page, end_page, abstract, doi, pmid, permalink, in_press, "
            "open_access, platform_id, content_location, full_text_file "
            "FROM articles ORDER BY article_id"
        ),
        "article_listing": (
            "SELECT article_id, journal_id, issue_id, publication_year, date, "
            "open_access, in_press, doi, area FROM article_listing ORDER BY article_id"
        ),
        "article_search": (
            "SELECT rowid, article_id, title, abstract, doi, authors, journal_title "
            "FROM article_search ORDER BY article_id"
        ),
        "index_runs": (
            "SELECT run_id, csv_file, status, total_journals, succeeded_journals, "
            "failed_journals, resumed_journals, error_summary FROM index_runs"
        ),
        "index_path_stats": (
            "SELECT source, path, journal_id, journal_title, status, works_count, "
            "issues_count, articles_written_count, articles_deleted_no_authors_count "
            "FROM index_path_stats ORDER BY source, path, journal_id"
        ),
        "index_api_call_stats": (
            "SELECT service, endpoint, method, url_path, logical_calls, attempts, "
            "successes, failures, retry_count, status_codes_json, transport_errors, "
            "rate_limit_failures FROM index_api_call_stats "
            "ORDER BY service, endpoint, method, url_path"
        ),
    }
    connection = sqlite3.connect(db_path)
    simple_path = resolve_simple_tokenizer_path()
    if simple_path:
        try:
            connection.enable_load_extension(True)
            connection.load_extension(simple_path)
            connection.enable_load_extension(False)
        except sqlite3.OperationalError:
            pass
    try:
        return {
            name: [tuple(row) for row in connection.execute(query).fetchall()]
            for name, query in queries.items()
        }
    finally:
        connection.close()


def normalized_manifest(path: Path) -> Any:
    """
    Load and normalize a change manifest.

    Args:
        path: Manifest path.

    Returns:
        Normalized manifest payload.
    """
    return normalize_dynamic_values(json.loads(path.read_text(encoding="utf-8")))


class ScholarlyIndexContractTest(unittest.TestCase):
    """Compare Rust Scholarly indexing against Python fixture behavior."""

    def test_rust_scholarly_fixture_index_matches_python_database_and_manifest(
        self,
    ) -> None:
        """
        Verify Rust Scholarly fixture indexing against Python output.

        Returns:
            None.
        """
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            temp_path = Path(temp_dir)
            csv_path = temp_path / "journals.csv"
            fixture_path = temp_path / "openalex_fallback_fixture.json"
            shutil.copy(SCHOLARLY_FIXTURE_ROOT / "journals.csv", csv_path)
            shutil.copy(
                SCHOLARLY_FIXTURE_ROOT / "openalex_fallback_fixture.json",
                fixture_path,
            )
            python_db = temp_path / "python" / "contract.sqlite"
            rust_db = temp_path / "rust" / "contract.sqlite"
            rust_manifest = temp_path / "rust" / "contract.changes.json"
            python_db.parent.mkdir(parents=True)
            rust_db.parent.mkdir(parents=True)

            python_manifest = asyncio.run(
                build_python_index(python_db, csv_path, fixture_path)
            )
            result = run_ps_cli(
                temp_path,
                [
                    "index",
                    "fixture",
                    "--csv",
                    str(csv_path),
                    "--fixture",
                    str(fixture_path),
                    "--output-db",
                    str(rust_db),
                    "--manifest",
                    str(rust_manifest),
                    "--run-id",
                    RUN_ID,
                    "--timestamp",
                    TIMESTAMP,
                    "--semantic-scholar-key",
                ],
            )
            payload = json.loads(result.stdout)

            self.assertEqual(payload["status"], "succeeded")
            self.assertEqual(dump_core_rows(rust_db), dump_core_rows(python_db))
            self.assertEqual(
                normalized_manifest(rust_manifest),
                normalized_manifest(python_manifest),
            )
            serialized_stats = json.dumps(
                dump_core_rows(rust_db)["index_api_call_stats"]
            )
            self.assertNotIn("SECRET", serialized_stats)
            self.assertNotIn("x-api-key=", serialized_stats)
            gc.collect()

    def test_crossref_non_404_failure_does_not_use_openalex_fallback(self) -> None:
        """
        Verify non-404 Crossref errors fail without OpenAlex fallback.

        Returns:
            None.
        """
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            temp_path = Path(temp_dir)
            csv_path = temp_path / "journals.csv"
            fixture_path = temp_path / "crossref_500_fixture.json"
            db_path = temp_path / "contract.sqlite"
            shutil.copy(SCHOLARLY_FIXTURE_ROOT / "journals.csv", csv_path)
            shutil.copy(
                SCHOLARLY_FIXTURE_ROOT / "crossref_500_fixture.json",
                fixture_path,
            )

            result = run_ps_cli(
                temp_path,
                [
                    "index",
                    "fixture",
                    "--csv",
                    str(csv_path),
                    "--fixture",
                    str(fixture_path),
                    "--output-db",
                    str(db_path),
                    "--run-id",
                    RUN_ID,
                    "--timestamp",
                    TIMESTAMP,
                    "--semantic-scholar-key",
                ],
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("crossref journal_works failed with HTTP 500", result.stderr)
            connection = sqlite3.connect(db_path)
            try:
                services = [
                    row[0]
                    for row in connection.execute(
                        "SELECT service FROM index_api_call_stats ORDER BY service"
                    ).fetchall()
                ]
            finally:
                connection.close()
            self.assertEqual(services, ["crossref"])
            gc.collect()
