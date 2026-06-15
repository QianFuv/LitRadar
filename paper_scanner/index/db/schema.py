"""Database schema definitions and initialization."""

from __future__ import annotations

import sqlite3

import aiosqlite

from paper_scanner.index.db.fts import ensure_article_search
from paper_scanner.index.db.retry import commit_with_retry, execute_with_retry
from paper_scanner.shared.constants import DB_TIMEOUT_SECONDS
from paper_scanner.shared.sqlite_ext import load_simple_tokenizer

JOURNAL_COLUMNS = [
    "journal_id",
    "library_id",
    "platform_journal_id",
    "title",
    "issn",
    "eissn",
    "scimago_rank",
    "cover_url",
    "available",
    "toc_data_approved_and_live",
    "has_articles",
]

JOURNAL_UPSERT = f"""
INSERT INTO journals ({", ".join(JOURNAL_COLUMNS)})
VALUES ({", ".join(["?"] * len(JOURNAL_COLUMNS))})
ON CONFLICT(journal_id) DO UPDATE SET
{", ".join(f"{col}=excluded.{col}" for col in JOURNAL_COLUMNS[1:])}
"""

META_COLUMNS = [
    "journal_id",
    "source_csv",
    "area",
    "csv_title",
    "csv_issn",
    "csv_library",
    "resolved_source",
    "resolved_source_id",
    "resolved_title",
    "resolved_issn",
    "resolved_eissn",
]

META_UPSERT = f"""
INSERT INTO journal_meta ({", ".join(META_COLUMNS)})
VALUES ({", ".join(["?"] * len(META_COLUMNS))})
ON CONFLICT(journal_id) DO UPDATE SET
{", ".join(f"{col}=excluded.{col}" for col in META_COLUMNS[1:])}
"""

ISSUE_COLUMNS = [
    "issue_id",
    "journal_id",
    "publication_year",
    "title",
    "volume",
    "number",
    "date",
    "is_valid_issue",
    "suppressed",
    "embargoed",
    "within_subscription",
]

ISSUE_UPSERT = f"""
INSERT INTO issues ({", ".join(ISSUE_COLUMNS)})
VALUES ({", ".join(["?"] * len(ISSUE_COLUMNS))})
ON CONFLICT(issue_id) DO UPDATE SET
{", ".join(f"{col}=excluded.{col}" for col in ISSUE_COLUMNS[1:])}
"""

ARTICLE_COLUMNS = [
    "article_id",
    "journal_id",
    "issue_id",
    "title",
    "date",
    "authors",
    "start_page",
    "end_page",
    "abstract",
    "doi",
    "pmid",
    "permalink",
    "suppressed",
    "in_press",
    "open_access",
    "platform_id",
    "retraction_doi",
    "within_library_holdings",
    "content_location",
    "full_text_file",
]

ARTICLE_UPSERT = f"""
INSERT INTO articles ({", ".join(ARTICLE_COLUMNS)})
VALUES ({", ".join(["?"] * len(ARTICLE_COLUMNS))})
ON CONFLICT(article_id) DO UPDATE SET
{", ".join(f"{col}=excluded.{col}" for col in ARTICLE_COLUMNS[1:])}
"""

ARTICLE_LISTING_COLUMNS = [
    "article_id",
    "journal_id",
    "issue_id",
    "publication_year",
    "date",
    "open_access",
    "in_press",
    "suppressed",
    "within_library_holdings",
    "doi",
    "pmid",
    "area",
]

ARTICLE_LISTING_BATCH_SIZE = 500


async def ensure_journal_platform_id_column(db: aiosqlite.Connection) -> None:
    """
    Ensure journals stores the upstream platform journal identifier.

    Args:
        db: Open aiosqlite connection.

    Returns:
        None.
    """
    cursor = await db.execute("PRAGMA table_info(journals);")
    journal_column_names = {str(row[1]) for row in await cursor.fetchall()}
    await cursor.close()
    if "platform_journal_id" in journal_column_names:
        return

    try:
        await execute_with_retry(
            db,
            "ALTER TABLE journals ADD COLUMN platform_journal_id TEXT;",
        )
    except sqlite3.OperationalError as exc:
        if "duplicate column name" not in str(exc).lower():
            raise
    await execute_with_retry(
        db,
        """
        UPDATE journals
        SET platform_journal_id = CAST(journal_id AS TEXT)
        WHERE platform_journal_id IS NULL
        """,
    )


async def ensure_journal_meta_resolution_columns(db: aiosqlite.Connection) -> None:
    """
    Ensure journal metadata can store resolved source details.

    Args:
        db: Open aiosqlite connection.

    Returns:
        None.
    """
    cursor = await db.execute("PRAGMA table_info(journal_meta)")
    meta_column_names = {str(row[1]) for row in await cursor.fetchall()}
    await cursor.close()
    required_columns = {
        "resolved_source": "TEXT",
        "resolved_source_id": "TEXT",
        "resolved_title": "TEXT",
        "resolved_issn": "TEXT",
        "resolved_eissn": "TEXT",
    }
    for column_name, column_type in required_columns.items():
        if column_name in meta_column_names:
            continue
        try:
            await execute_with_retry(
                db,
                f"ALTER TABLE journal_meta ADD COLUMN {column_name} {column_type};",
            )
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise


async def init_db(db: aiosqlite.Connection) -> None:
    """
    Initialize database schema and indexes.

    Args:
        db: Open aiosqlite connection.

    Returns:
        None.
    """
    await execute_with_retry(db, "PRAGMA journal_mode=WAL;")
    await execute_with_retry(db, "PRAGMA foreign_keys=ON;")
    await execute_with_retry(db, "PRAGMA synchronous=NORMAL;")
    await execute_with_retry(db, f"PRAGMA busy_timeout={DB_TIMEOUT_SECONDS * 1000};")
    use_simple = await load_simple_tokenizer(db)

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS journals (
            journal_id INTEGER PRIMARY KEY,
            library_id TEXT NOT NULL,
            platform_journal_id TEXT,
            title TEXT,
            issn TEXT,
            eissn TEXT,
            scimago_rank REAL,
            cover_url TEXT,
            available INTEGER,
            toc_data_approved_and_live INTEGER,
            has_articles INTEGER
        );
        """,
    )

    await ensure_journal_platform_id_column(db)

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS journal_meta (
            journal_id INTEGER PRIMARY KEY,
            source_csv TEXT NOT NULL,
            area TEXT,
            csv_title TEXT,
            csv_issn TEXT,
            csv_library TEXT,
            resolved_source TEXT,
            resolved_source_id TEXT,
            resolved_title TEXT,
            resolved_issn TEXT,
            resolved_eissn TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE
        );
        """,
    )

    await ensure_journal_meta_resolution_columns(db)

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS issues (
            issue_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            publication_year INTEGER,
            title TEXT,
            volume TEXT,
            number TEXT,
            date TEXT,
            is_valid_issue INTEGER,
            suppressed INTEGER,
            embargoed INTEGER,
            within_subscription INTEGER,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS articles (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            title TEXT,
            date TEXT,
            authors TEXT,
            start_page TEXT,
            end_page TEXT,
            abstract TEXT,
            doi TEXT,
            pmid TEXT,
            permalink TEXT,
            suppressed INTEGER,
            in_press INTEGER,
            open_access INTEGER,
            platform_id TEXT,
            retraction_doi TEXT,
            within_library_holdings INTEGER,
            content_location TEXT,
            full_text_file TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE,
            FOREIGN KEY (issue_id) REFERENCES issues(issue_id)
                ON DELETE SET NULL
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS article_listing (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            publication_year INTEGER,
            date TEXT,
            open_access INTEGER,
            in_press INTEGER,
            suppressed INTEGER,
            within_library_holdings INTEGER,
            doi TEXT,
            pmid TEXT,
            area TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE,
            FOREIGN KEY (issue_id) REFERENCES issues(issue_id)
                ON DELETE SET NULL
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS listing_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            status TEXT,
            updated_at TEXT
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS journal_year_state (
            journal_id INTEGER NOT NULL,
            year INTEGER NOT NULL,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (journal_id, year)
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS journal_state (
            journal_id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS index_runs (
            run_id TEXT PRIMARY KEY,
            csv_file TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            status TEXT NOT NULL,
            total_journals INTEGER NOT NULL,
            succeeded_journals INTEGER NOT NULL,
            failed_journals INTEGER NOT NULL,
            resumed_journals INTEGER NOT NULL,
            error_summary TEXT
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS index_path_stats (
            run_id TEXT NOT NULL,
            source TEXT NOT NULL,
            path TEXT NOT NULL,
            journal_id INTEGER,
            journal_title TEXT,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            works_count INTEGER NOT NULL,
            issues_count INTEGER NOT NULL,
            article_summaries_count INTEGER NOT NULL,
            article_details_count INTEGER NOT NULL,
            articles_written_count INTEGER NOT NULL,
            articles_deleted_no_authors_count INTEGER NOT NULL,
            error_type TEXT,
            error_message TEXT,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );
        """,
    )

    await execute_with_retry(
        db,
        """
        CREATE TABLE IF NOT EXISTS index_api_call_stats (
            run_id TEXT NOT NULL,
            source TEXT NOT NULL,
            service TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            method TEXT NOT NULL,
            url_path TEXT NOT NULL,
            journal_id INTEGER,
            journal_title TEXT,
            logical_calls INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            successes INTEGER NOT NULL,
            failures INTEGER NOT NULL,
            retry_count INTEGER NOT NULL,
            status_codes_json TEXT NOT NULL,
            transport_errors INTEGER NOT NULL,
            rate_limit_failures INTEGER NOT NULL,
            total_latency_ms INTEGER NOT NULL,
            error_samples_json TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );
        """,
    )

    await ensure_article_search(db, use_simple)

    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_journals_issn ON journals(issn);"
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_journals_library_id ON journals(library_id);",
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_journals_available ON journals(available);"
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_journals_has_articles "
        "ON journals(has_articles);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_journals_scimago_rank "
        "ON journals(scimago_rank);",
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_journal_meta_area ON journal_meta(area);"
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_journal_meta_area_journal "
        "ON journal_meta(area, journal_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_issues_journal_year "
        "ON issues(journal_id, publication_year);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_issues_publication_year "
        "ON issues(publication_year);",
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_articles_journal ON articles(journal_id);"
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_articles_issue ON articles(issue_id);"
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_articles_date ON articles(date);"
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_date_id "
        "ON articles(date, article_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_journal_date_id "
        "ON articles(journal_id, date, article_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_issue_date_id "
        "ON articles(issue_id, date, article_id);",
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_articles_doi ON articles(doi);"
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_articles_pmid ON articles(pmid);"
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_open_access ON articles(open_access);",
    )
    await execute_with_retry(
        db, "CREATE INDEX IF NOT EXISTS idx_articles_in_press ON articles(in_press);"
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_suppressed ON articles(suppressed);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_within_holdings "
        "ON articles(within_library_holdings);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_open_access_date_id "
        "ON articles(open_access, date, article_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_in_press_date_id "
        "ON articles(in_press, date, article_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_suppressed_date_id "
        "ON articles(suppressed, date, article_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_articles_within_holdings_date_id "
        "ON articles(within_library_holdings, date, article_id);",
    )

    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_article_listing_date_id "
        "ON article_listing(date, article_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_article_listing_area ON article_listing(area);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_article_listing_publication_year "
        "ON article_listing(publication_year);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_article_listing_journal "
        "ON article_listing(journal_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_article_listing_issue "
        "ON article_listing(issue_id);",
    )

    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_index_path_stats_run "
        "ON index_path_stats(run_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_index_path_stats_status "
        "ON index_path_stats(run_id, status);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_index_api_call_stats_run "
        "ON index_api_call_stats(run_id);",
    )
    await execute_with_retry(
        db,
        "CREATE INDEX IF NOT EXISTS idx_index_api_call_stats_service "
        "ON index_api_call_stats(run_id, service, endpoint);",
    )

    await commit_with_retry(db)


async def optimize_db(db: aiosqlite.Connection) -> None:
    """
    Run SQLite optimizations after data load.

    Args:
        db: Open aiosqlite connection.

    Returns:
        None.
    """
    await execute_with_retry(db, "ANALYZE;")
    await execute_with_retry(db, "PRAGMA optimize;")
    await commit_with_retry(db)
