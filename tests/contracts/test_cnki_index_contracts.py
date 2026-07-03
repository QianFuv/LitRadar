"""Contracts for Rust CNKI index migration."""

from __future__ import annotations

import asyncio
import csv
import gc
import json
import shutil
import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

import aiosqlite

from paper_scanner.index.changes import (
    collect_article_snapshot,
    compute_changed_group_keys,
    write_change_manifest,
)
from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import (
    persist_index_run_stats,
    upsert_articles,
    upsert_issues,
    upsert_journal,
    upsert_meta,
)
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.fetcher import process_cnki_journal
from paper_scanner.index.stats import ApiStatsKey, IndexStatsRecorder
from paper_scanner.index.transforms import (
    build_cnki_article_record,
    build_cnki_issue_record,
    build_cnki_journal_record,
    build_journal_id,
    build_meta_record,
)
from paper_scanner.shared.cnki_urls import with_cnki_chinese_language
from paper_scanner.sources.cnki import client as cnki_source

from .contract_support import FIXTURE_ROOT
from .test_scholarly_index_contracts import (
    dump_core_rows,
    run_ps_cli,
)

CNKI_FIXTURE_ROOT = FIXTURE_ROOT / "cnki"
RUN_ID = "run-cnki-contract"
TIMESTAMP = "2026-07-03T00:00:00Z"


class FixtureCnkiClient:
    """Python CNKI client backed by the same HTML fixture as Rust."""

    def __init__(
        self,
        fixture: dict[str, Any],
        stats_recorder: IndexStatsRecorder,
    ) -> None:
        """
        Initialize the fixture CNKI client.

        Args:
            fixture: Fixture source payloads.
            stats_recorder: Stats recorder used by the Python indexer.

        Returns:
            None.
        """
        self.fixture = fixture
        self.stats_recorder = stats_recorder
        self.issue_article_requests: list[str] = []

    async def resolve_journal(self, row: dict[str, str]) -> dict[str, Any] | None:
        """
        Return parsed CNKI journal details when the fixture matches the row.

        Args:
            row: Source CSV row.

        Returns:
            CNKI journal detail payload or None.
        """
        text = self._recorded_text(
            "journal_detail",
            None,
            "GET",
            f"{cnki_source.BASE_URL}/knavi/detail?pykm=TEST",
        )
        details = parse_journal_detail(text)
        title = (row.get("title") or "").strip()
        issn = (row.get("issn") or "").strip()
        if cnki_source._journal_detail_matches(details, title, issn):
            return details
        return None

    async def get_year_issues(self, journal: dict[str, Any]) -> list[dict[str, Any]]:
        """
        Return parsed CNKI issue fixtures.

        Args:
            journal: CNKI journal detail payload.

        Returns:
            CNKI issue payloads.
        """
        text = self._recorded_text(
            "year_issues",
            None,
            "POST",
            f"{cnki_source.BASE_URL}/knavi/journals/{journal['pykm']}/yearList",
        )
        return cast(list[dict[str, Any]], cnki_source._parse_year_issues(text))

    async def get_issue_articles(
        self,
        journal: dict[str, Any],
        issue: dict[str, Any],
    ) -> list[dict[str, Any]]:
        """
        Return parsed CNKI article summaries for one issue.

        Args:
            journal: CNKI journal detail payload.
            issue: CNKI issue payload.

        Returns:
            Article summary payloads.
        """
        year_issue = str(issue["year_issue"])
        self.issue_article_requests.append(f"{issue['year']}:{issue['number']}")
        text = self._recorded_text(
            "issue_articles",
            year_issue,
            "POST",
            f"{cnki_source.BASE_URL}/knavi/journals/{journal['pykm']}/papers"
            f"?yearIssue={year_issue}",
        )
        return cast(
            list[dict[str, Any]],
            cnki_source._parse_issue_articles(text, issue),
        )

    async def get_article_detail(self, article_url: str) -> dict[str, Any]:
        """
        Return parsed CNKI article detail for one summary URL.

        Args:
            article_url: Article URL from the summary.

        Returns:
            Article detail payload.
        """
        platform_id = platform_id_from_url(article_url)
        text = self._recorded_text(
            "article_detail",
            platform_id,
            "GET",
            f"{cnki_source.BASE_URL}/kcms2/article/abstract?filename={platform_id}",
        )
        return parse_article_detail(text, article_url)

    def _recorded_text(
        self,
        endpoint: str,
        key: str | None,
        method: str,
        url: str,
    ) -> str:
        """
        Return fixture text and record one successful API attempt.

        Args:
            endpoint: Logical endpoint name.
            key: Optional fixture key.
            method: HTTP method.
            url: Request URL.

        Returns:
            Fixture response text.
        """
        stats_key = self.stats_recorder.record_api_call(
            "cnki",
            endpoint,
            method,
            url,
        )
        payload = fixture_text(self.fixture, endpoint, key)
        self._record_attempt(stats_key, 200, True, None)
        return payload

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


def fixture_text(fixture: dict[str, Any], endpoint: str, key: str | None) -> str:
    """
    Resolve one fixture response body.

    Args:
        fixture: Fixture source payload.
        endpoint: Logical endpoint name.
        key: Optional fixture key.

    Returns:
        Fixture response text.
    """
    if endpoint == "journal_detail":
        return str(fixture["journal_detail_html"])
    if endpoint == "year_issues":
        return str(fixture["year_issues_html"])
    if endpoint == "issue_articles":
        return str(fixture["issue_articles_html"][str(key)])
    if endpoint == "article_detail":
        return str(fixture["article_detail_html"][str(key)])
    raise KeyError(endpoint)


def parse_journal_detail(text: str) -> dict[str, Any]:
    """
    Parse a CNKI journal detail page with the production parser helpers.

    Args:
        text: Journal detail HTML.

    Returns:
        CNKI journal detail payload.
    """
    pykm = cnki_source._input_value(text, "pykm")
    if not pykm:
        raise ValueError("CNKI journal detail missing pykm")
    visible_text = cnki_source._strip_tags(text)
    return {
        "detail_url": with_cnki_chinese_language(
            f"{cnki_source.BASE_URL}/knavi/detail?pykm={pykm}"
        ),
        "pykm": pykm,
        "pcode": cnki_source._input_value(text, "pCode") or cnki_source.DEFAULT_PCODE,
        "time": cnki_source._input_value(text, "time"),
        "title": cnki_source._input_value(text, "shareChName")
        or cnki_source._title_text(text),
        "issn": cnki_source._regex_group(r"ISSN\s*[:：]\s*([0-9Xx-]+)", visible_text),
        "cn": cnki_source._regex_group(r"CN\s*[:：]\s*([0-9A-Za-z/-]+)", visible_text),
        "impact_factor": cnki_source._regex_group(
            r"(?:复合影响因子|Combined IF)\s*[:：]\s*([0-9.]+)",
            visible_text,
        ),
        "cover_url": cnki_source._image_url(text),
        "raw_text": visible_text,
    }


def parse_article_detail(text: str, article_url: str) -> dict[str, Any]:
    """
    Parse a CNKI article detail page with the production parser helpers.

    Args:
        text: Article detail HTML.
        article_url: Original article URL.

    Returns:
        CNKI article detail payload.
    """
    resolved_url = with_cnki_chinese_language(article_url)
    filename = cnki_source._input_value(
        text, "paramfilename"
    ) or cnki_source._input_value(text, "param-filename")
    dbcode = cnki_source._input_value(text, "paramdbcode") or cnki_source._input_value(
        text, "param-dbcode"
    )
    dbname = cnki_source._input_value(text, "paramdbname") or cnki_source._input_value(
        text, "param-dbname"
    )
    visible_text = cnki_source._strip_tags(text)
    online_time = cnki_source._row_value(
        text, "在线公开时间"
    ) or cnki_source._row_value(
        text,
        "Online Release Time",
    )
    permalink = (
        cnki_source._article_detail_url(dbcode, dbname, filename) or resolved_url
    )
    return {
        "article_url": resolved_url,
        "platform_id": filename,
        "dbcode": dbcode,
        "dbname": dbname,
        "title": cnki_source._first_block_text(
            text,
            r'<p\s+class="title-one"[^>]*>(.*?)</p>',
        )
        or cnki_source._title_text(text),
        "authors": cnki_source._author_text(text) or None,
        "abstract": cnki_source._input_value(text, "abstract_text"),
        "doi": cnki_source._row_value(text, "DOI"),
        "online_release_date": cnki_source._date_part(online_time),
        "pages": cnki_source._regex_group(
            r"页码\s*[:：]\s*([0-9A-Za-z\-–—]+)",
            visible_text,
        ),
        "html_read_url": cnki_source._link_with_text(text, "HTML阅读"),
        "permalink": permalink,
        "content_location": permalink,
    }


def platform_id_from_url(article_url: str) -> str:
    """
    Extract a CNKI platform id from an article URL.

    Args:
        article_url: Article URL.

    Returns:
        Platform id value.
    """
    query = parse_qs(urlsplit(article_url).query)
    return query.get("filename", [article_url.rsplit("/", maxsplit=1)[-1]])[0]


def run_without_simple_tokenizer(awaitable: Any) -> Any:
    """
    Run an async test helper with the optional SQLite tokenizer disabled.

    Args:
        awaitable: Awaitable object to run.

    Returns:
        Awaitable result.
    """
    with patch(
        "paper_scanner.shared.sqlite_ext.resolve_simple_tokenizer_path",
        return_value=None,
    ):
        return asyncio.run(awaitable)


async def initialize_cnki_db(
    db_path: Path,
    csv_path: Path,
    fixture_path: Path,
    seed_existing: bool,
) -> None:
    """
    Initialize a Python index database and optionally seed old CNKI rows.

    Args:
        db_path: Output database path.
        csv_path: Source CSV path.
        fixture_path: Source fixture path.
        seed_existing: Whether to seed 2024 and 2025 issue rows.

    Returns:
        None.
    """
    db_path.parent.mkdir(parents=True, exist_ok=True)
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    with open(csv_path, newline="", encoding="utf-8") as handle:
        row = next(csv.DictReader(handle))

    async with aiosqlite.connect(db_path) as raw_db:
        await init_db(raw_db)
        db = LocalDatabaseClient(raw_db)
        await db.start()
        try:
            if seed_existing:
                await seed_cnki_database(db, csv_path, row, fixture)
            await db.commit()
        finally:
            await db.close()


async def seed_cnki_database(
    db: LocalDatabaseClient,
    csv_path: Path,
    row: dict[str, str],
    fixture: dict[str, Any],
) -> None:
    """
    Seed existing 2024 and 2025 CNKI issue/article rows.

    Args:
        db: Open database client.
        csv_path: Source CSV path.
        row: Source CSV row.
        fixture: Fixture source payloads.

    Returns:
        None.
    """
    journal_id = build_journal_id(row)
    if journal_id is None:
        raise ValueError("CNKI test journal id should build")
    details = parse_journal_detail(str(fixture["journal_detail_html"]))
    issues = cnki_source._parse_year_issues(str(fixture["year_issues_html"]))
    selected = {
        int(issue["year"]): cast(dict[str, Any], issue)
        for issue in issues
        if issue["year"] in {2024, 2025}
    }
    journal_code = str(details["pykm"])
    issue_records = [
        build_cnki_issue_record(journal_id, journal_code, selected[2024]),
        build_cnki_issue_record(journal_id, journal_code, selected[2025]),
    ]
    typed_issue_records = [record for record in issue_records if record is not None]
    old_article = build_cnki_article_record(
        seed_detail("seed-old"),
        seed_summary(selected[2024], "seed-old"),
        journal_id,
        typed_issue_records[0]["issue_id"],
    )
    latest_article = build_cnki_article_record(
        seed_detail("seed-latest"),
        seed_summary(selected[2025], "seed-latest"),
        journal_id,
        typed_issue_records[1]["issue_id"],
    )
    await upsert_journal(db, build_cnki_journal_record(journal_id, row, details))
    await upsert_meta(db, build_meta_record(journal_id, csv_path, row))
    await upsert_issues(db, typed_issue_records)
    await upsert_articles(
        db,
        [article for article in [old_article, latest_article] if article is not None],
    )


def seed_summary(issue: dict[str, Any], platform_id: str) -> dict[str, Any]:
    """
    Build a seed CNKI article summary.

    Args:
        issue: CNKI issue payload.
        platform_id: Seed platform id.

    Returns:
        CNKI article summary payload.
    """
    return {
        "article_url": f"https://example.test/article/{platform_id}",
        "platform_id": platform_id,
        "title": f"CNKI article {platform_id}",
        "authors": "Test Author",
        "pages": "1-2",
        "section": "Articles",
        "is_free": 0,
        "date": f"{int(issue['year']):04d}-01-01",
    }


def seed_detail(platform_id: str) -> dict[str, Any]:
    """
    Build a seed CNKI article detail.

    Args:
        platform_id: Seed platform id.

    Returns:
        CNKI article detail payload.
    """
    return {
        "article_url": f"https://example.test/article/{platform_id}",
        "platform_id": platform_id,
        "title": f"CNKI article {platform_id}",
        "authors": "Test Author",
        "abstract": "Seed abstract.",
        "doi": None,
        "online_release_date": "2025-01-01",
        "pages": "1-2",
        "html_read_url": None,
        "permalink": f"https://example.test/article/{platform_id}",
        "content_location": f"https://example.test/article/{platform_id}",
    }


async def run_python_cnki_index(
    db_path: Path,
    csv_path: Path,
    fixture_path: Path,
    resume: bool,
    update: bool,
) -> Path:
    """
    Run Python CNKI indexing against the shared fixture.

    Args:
        db_path: Output database path.
        csv_path: Source CSV path.
        fixture_path: Source fixture path.
        resume: Whether resume mode is enabled.
        update: Whether update mode is enabled.

    Returns:
        Python manifest path.
    """
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    with open(csv_path, newline="", encoding="utf-8") as handle:
        row = next(csv.DictReader(handle))
    before_issue_map, before_inpress_map = collect_article_snapshot(db_path)
    stats_recorder = IndexStatsRecorder(RUN_ID, csv_path.name, started_at=TIMESTAMP)

    async with aiosqlite.connect(db_path) as raw_db:
        db = LocalDatabaseClient(raw_db)
        await db.start()
        try:
            await process_cnki_journal(
                db,
                cast(Any, FixtureCnkiClient(fixture, stats_recorder)),
                csv_path,
                row,
                issue_batch_size=10,
                request_workers=4,
                show_year_progress=False,
                resume=resume,
                update=update,
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


class CnkiIndexContractTest(unittest.TestCase):
    """Compare Rust CNKI indexing against Python fixture behavior."""

    def test_rust_cnki_fixture_index_matches_python_database_and_writes_manifest(
        self,
    ) -> None:
        """
        Verify Rust CNKI fixture indexing against Python database output.

        Returns:
            None.
        """
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            temp_path = Path(temp_dir)
            csv_path = temp_path / "journals.csv"
            fixture_path = temp_path / "fixture.json"
            shutil.copy(CNKI_FIXTURE_ROOT / "journals.csv", csv_path)
            shutil.copy(CNKI_FIXTURE_ROOT / "fixture.json", fixture_path)
            python_db = temp_path / "python" / "contract.sqlite"
            rust_db = temp_path / "rust" / "contract.sqlite"
            rust_manifest = temp_path / "rust" / "contract.changes.json"
            run_without_simple_tokenizer(
                initialize_cnki_db(python_db, csv_path, fixture_path, False)
            )

            run_without_simple_tokenizer(
                run_python_cnki_index(
                    python_db,
                    csv_path,
                    fixture_path,
                    resume=False,
                    update=False,
                )
            )
            result = run_ps_cli(
                temp_path,
                [
                    "index",
                    "fixture",
                    "--source",
                    "cnki",
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
                ],
            )
            payload = json.loads(result.stdout)

            self.assertEqual(payload["status"], "succeeded")
            self.assertEqual(dump_core_rows(rust_db), dump_core_rows(python_db))
            self.assertTrue(rust_manifest.exists())
            serialized_articles = json.dumps(dump_core_rows(rust_db)["articles"])
            self.assertIn("CNKI202601001", serialized_articles)
            self.assertNotIn("barnew/download", serialized_articles)
            gc.collect()

    def test_cnki_update_matches_python_recent_issue_scope(self) -> None:
        """
        Verify update mode refreshes through the latest existing issue only.

        Returns:
            None.
        """
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            temp_path = Path(temp_dir)
            csv_path = temp_path / "journals.csv"
            fixture_path = temp_path / "fixture.json"
            seed_db = temp_path / "seed.sqlite"
            python_db = temp_path / "python" / "contract.sqlite"
            rust_db = temp_path / "rust" / "contract.sqlite"
            shutil.copy(CNKI_FIXTURE_ROOT / "journals.csv", csv_path)
            shutil.copy(CNKI_FIXTURE_ROOT / "fixture.json", fixture_path)
            run_without_simple_tokenizer(
                initialize_cnki_db(seed_db, csv_path, fixture_path, True)
            )
            python_db.parent.mkdir(parents=True)
            rust_db.parent.mkdir(parents=True)
            shutil.copy(seed_db, python_db)
            shutil.copy(seed_db, rust_db)

            run_without_simple_tokenizer(
                run_python_cnki_index(
                    python_db,
                    csv_path,
                    fixture_path,
                    resume=True,
                    update=True,
                )
            )
            result = run_ps_cli(
                temp_path,
                [
                    "index",
                    "fixture",
                    "--source",
                    "cnki",
                    "--csv",
                    str(csv_path),
                    "--fixture",
                    str(fixture_path),
                    "--output-db",
                    str(rust_db),
                    "--run-id",
                    RUN_ID,
                    "--timestamp",
                    TIMESTAMP,
                    "--resume",
                    "--update",
                ],
            )
            payload = json.loads(result.stdout)
            serialized_articles = json.dumps(dump_core_rows(rust_db)["articles"])

            self.assertEqual(payload["status"], "succeeded")
            self.assertEqual(dump_core_rows(rust_db), dump_core_rows(python_db))
            self.assertIn("CNKI202601001", serialized_articles)
            self.assertIn("CNKI202501001", serialized_articles)
            self.assertNotIn("CNKI202401001", serialized_articles)
            gc.collect()

    def test_cnki_parser_failure_fails_loudly(self) -> None:
        """
        Verify parser failures return non-zero CLI status and path stats.

        Returns:
            None.
        """
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            temp_path = Path(temp_dir)
            csv_path = temp_path / "journals.csv"
            fixture_path = temp_path / "parser_failure_fixture.json"
            db_path = temp_path / "contract.sqlite"
            shutil.copy(CNKI_FIXTURE_ROOT / "journals.csv", csv_path)
            shutil.copy(
                CNKI_FIXTURE_ROOT / "parser_failure_fixture.json",
                fixture_path,
            )

            result = run_ps_cli(
                temp_path,
                [
                    "index",
                    "fixture",
                    "--source",
                    "cnki",
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
                ],
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "CNKI parser fixture failed for article_detail", result.stderr
            )
            connection = sqlite3.connect(db_path)
            try:
                rows = connection.execute(
                    "SELECT status, error_message FROM index_path_stats"
                ).fetchall()
            finally:
                connection.close()
            self.assertEqual(rows[0][0], "failed")
            self.assertIn("article_detail", rows[0][1])
            gc.collect()
