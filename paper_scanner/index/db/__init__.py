"""Database subpackage exports."""

from paper_scanner.index.db.client import (
    DatabaseClient,
    IPCDatabaseClient,
    LocalDatabaseClient,
)
from paper_scanner.index.db.fts import ensure_article_search
from paper_scanner.index.db.operations import (
    get_completed_years,
    get_issue_ids_with_articles,
    get_journal_issue_ids_with_articles,
    get_latest_issue_with_articles,
    is_article_listing_complete,
    is_journal_complete,
    mark_journal_done,
    mark_listing_ready,
    mark_year_done,
    persist_index_run_stats,
    refresh_article_listing_for_articles,
    refresh_article_listing_for_issues,
    upsert_article_search,
    upsert_articles,
    upsert_issues,
    upsert_journal,
    upsert_meta,
)
from paper_scanner.index.db.retry import (
    commit_with_retry,
    execute_with_retry,
    executemany_with_retry,
)
from paper_scanner.index.db.schema import init_db, optimize_db
from paper_scanner.index.db.writer import DatabaseWriter

__all__ = [
    "DatabaseClient",
    "LocalDatabaseClient",
    "IPCDatabaseClient",
    "DatabaseWriter",
    "execute_with_retry",
    "executemany_with_retry",
    "commit_with_retry",
    "ensure_article_search",
    "init_db",
    "optimize_db",
    "upsert_journal",
    "upsert_meta",
    "upsert_issues",
    "upsert_articles",
    "upsert_article_search",
    "refresh_article_listing_for_articles",
    "refresh_article_listing_for_issues",
    "get_issue_ids_with_articles",
    "get_journal_issue_ids_with_articles",
    "get_latest_issue_with_articles",
    "get_completed_years",
    "is_article_listing_complete",
    "is_journal_complete",
    "mark_year_done",
    "mark_journal_done",
    "mark_listing_ready",
    "persist_index_run_stats",
]
