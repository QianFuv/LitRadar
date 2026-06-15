"""Journal fetch and processing workflows."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import httpx
from tqdm import tqdm

from paper_scanner.index.db.client import DatabaseClient
from paper_scanner.index.db.operations import (
    delete_articles,
    get_completed_years,
    get_journal_issue_ids_with_articles,
    get_latest_issue_with_articles,
    is_journal_complete,
    mark_journal_done,
    mark_year_done,
    refresh_article_listing_for_articles,
    refresh_article_listing_for_issues,
    upsert_article_search,
    upsert_articles,
    upsert_issues,
    upsert_journal,
    upsert_meta,
)
from paper_scanner.index.stats import (
    IndexStatsRecorder,
    NoOpIndexStatsRecorder,
    PathStatsKey,
)
from paper_scanner.index.transforms import (
    build_cnki_article_record,
    build_cnki_issue_record,
    build_cnki_journal_record,
    build_journal_id,
    build_meta_record,
    build_openalex_crossref_work,
    build_openalex_journal_row,
    build_openalex_resolved_meta_fields,
    build_resolved_meta_fields,
    build_scholarly_article_record,
    build_scholarly_issue_record,
    build_scholarly_journal_record,
    normalize_doi,
    source_from_row,
)
from paper_scanner.shared.constants import CNKI_SOURCE, SCHOLARLY_SOURCE
from paper_scanner.shared.converters import chunked
from paper_scanner.sources.cnki import CnkiClient
from paper_scanner.sources.scholarly import ScholarlyClient

CNKI_DETAIL_ATTEMPTS = 3
CNKI_DETAIL_RETRY_SECONDS = 1.0
CNKI_DETAIL_WORKER_LIMIT = 32


class JournalPathError(RuntimeError):
    """Raised when a journal source path cannot be completed."""


def stats_recorder_or_noop(
    stats_recorder: IndexStatsRecorder | NoOpIndexStatsRecorder | None,
) -> IndexStatsRecorder | NoOpIndexStatsRecorder:
    """
    Return an active stats recorder or a no-op recorder.

    Args:
        stats_recorder: Optional stats recorder.

    Returns:
        Stats recorder object.
    """
    return stats_recorder or NoOpIndexStatsRecorder()


def journal_title_from_row(row: dict[str, str]) -> str:
    """
    Resolve a display title for a CSV journal row.

    Args:
        row: CSV row.

    Returns:
        Journal title fallback.
    """
    return row.get("title") or row.get("id") or "Unknown"


def candidate_issns_from_row(csv_row: dict[str, str]) -> list[str]:
    """
    Build ordered ISSN candidates from a scholarly CSV row.

    Args:
        csv_row: Source CSV row.

    Returns:
        Unique ISSN candidates in lookup order.
    """
    candidates: list[str] = []
    for value in (
        csv_row.get("issn") or "",
        csv_row.get("all_issns") or "",
        csv_row.get("id") or "",
    ):
        for part in value.split(";"):
            candidate = part.strip()
            if (
                candidate
                and _is_issn_candidate(candidate)
                and candidate not in candidates
            ):
                candidates.append(candidate)
    return candidates


def _is_issn_candidate(value: str) -> bool:
    """
    Return whether a value can be used as an ISSN lookup candidate.

    Args:
        value: Raw ISSN-like value.

    Returns:
        Whether the value has a valid ISSN shape.
    """
    text = value.strip().replace("-", "").upper()
    if len(text) != 8:
        return False
    return text[:7].isdigit() and (text[7].isdigit() or text[7] == "X")


def split_article_records_by_authors(
    records: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[int]]:
    """
    Split article records into writable records and no-author article IDs.

    Args:
        records: Article records to classify.

    Returns:
        Tuple of records with authors and article IDs to delete.
    """
    kept_records: list[dict[str, Any]] = []
    deleted_article_ids: list[int] = []
    for record in records:
        authors = str(record.get("authors") or "").strip()
        if authors:
            kept_records.append(record)
            continue
        article_id = record.get("article_id")
        if isinstance(article_id, int):
            deleted_article_ids.append(article_id)
    return kept_records, deleted_article_ids


def select_recent_update_issue_ids(
    issue_ids: list[int],
    existing_issue_ids: set[int],
) -> list[int]:
    """
    Select issues from the newest issue through the latest indexed issue.

    Args:
        issue_ids: Issue IDs in newest-to-oldest upstream order.
        existing_issue_ids: Issue IDs that already have indexed articles.

    Returns:
        Issue IDs to refresh during an update run.
    """
    if not existing_issue_ids:
        return issue_ids
    issue_ids_to_fetch: list[int] = []
    for issue_id in issue_ids:
        issue_ids_to_fetch.append(issue_id)
        if issue_id in existing_issue_ids:
            break
    return issue_ids_to_fetch


def scholarly_update_from_pub_date(
    latest_issue: tuple[int, int | None, str | None] | None,
) -> str | None:
    """
    Build a Crossref publication-date lower bound for scholarly updates.

    Args:
        latest_issue: Latest existing issue tuple from the database.

    Returns:
        Publication date filter value or None when no prior issue exists.
    """
    if latest_issue is None:
        return None
    publication_year = latest_issue[1]
    if isinstance(publication_year, int):
        return f"{publication_year:04d}-01-01"
    date = latest_issue[2]
    if date:
        return str(date)[:10]
    return None


def select_scholarly_update_works(
    journal_id: int,
    works: list[dict[str, Any]],
    existing_issue_ids: set[int],
    latest_existing_issue_id: int | None,
) -> list[dict[str, Any]]:
    """
    Select Crossref works that need processing during a scholarly update.

    Args:
        journal_id: Internal journal ID.
        works: Crossref works fetched for the update window.
        existing_issue_ids: Issue IDs that already have articles.
        latest_existing_issue_id: Newest issue ID that already has articles.

    Returns:
        Crossref works from the latest existing issue and newly seen issues.
    """
    if latest_existing_issue_id is None:
        return works

    selected: list[dict[str, Any]] = []
    for work in works:
        issue_record = build_scholarly_issue_record(journal_id, work)
        if issue_record is None:
            selected.append(work)
            continue
        issue_id = issue_record["issue_id"]
        if issue_id == latest_existing_issue_id or issue_id not in existing_issue_ids:
            selected.append(work)
    return selected


async def process_scholarly_journal(
    db: DatabaseClient,
    client: ScholarlyClient,
    csv_path: Path,
    row: dict[str, str],
    request_workers: int,
    show_year_progress: bool,
    resume: bool,
    update: bool,
    stats_recorder: IndexStatsRecorder | NoOpIndexStatsRecorder | None = None,
) -> None:
    """
    Export one Crossref/OpenAlex/Semantic Scholar journal to the database.

    Args:
        db: Database client.
        client: Scholarly metadata client.
        csv_path: Source CSV path.
        row: CSV row for the journal.
        request_workers: Maximum concurrent HTTP requests for compatible callers.
        show_year_progress: Whether to display year progress with tqdm.
        resume: Whether to resume from completed journals.
        update: Whether to refresh existing journal data.
        stats_recorder: Optional index statistics recorder.

    Returns:
        None.
    """
    stats = stats_recorder_or_noop(stats_recorder)
    journal_id = build_journal_id(row)
    path_key = stats.record_path_started(
        SCHOLARLY_SOURCE,
        "journal",
        journal_id,
        journal_title_from_row(row),
    )
    try:
        if journal_id is None:
            raise JournalPathError(
                f"Scholarly journal missing id: {journal_title_from_row(row)}"
            )

        if resume and not update and await is_journal_complete(db, journal_id):
            stats.record_path_finished("resumed", path_key)
            return

        issn_candidates = candidate_issns_from_row(row)
        if not issn_candidates:
            raise JournalPathError(
                f"Scholarly journal missing ISSN: {journal_title_from_row(row)}"
            )

        latest_existing_issue = None
        existing_issue_ids: set[int] = set()
        from_pub_date = None
        if update:
            latest_existing_issue = await get_latest_issue_with_articles(db, journal_id)
            from_pub_date = scholarly_update_from_pub_date(latest_existing_issue)
            if latest_existing_issue is not None:
                existing_issue_ids = await get_journal_issue_ids_with_articles(
                    db, journal_id
                )

        issn = issn_candidates[0]
        works: list[dict[str, Any]] = []
        openalex_source: dict[str, Any] | None = None
        fallback_openalex_by_doi: dict[str, dict[str, Any]] = {}
        last_error: Exception | None = None
        for candidate in issn_candidates:
            try:
                works = await client.fetch_journal_works(
                    candidate, from_pub_date=from_pub_date
                )
                issn = candidate
                break
            except httpx.HTTPStatusError as exc:
                last_error = exc
                if exc.response.status_code != 404:
                    raise
        else:
            openalex_source = await client.fetch_openalex_source_by_issns(
                issn_candidates
            )
            if openalex_source is None:
                openalex_source = await client.fetch_openalex_source_by_title(
                    journal_title_from_row(row)
                )
            if openalex_source is None:
                error = JournalPathError(
                    "Scholarly journal has no available ISSN candidate: "
                    f"{journal_title_from_row(row)}"
                )
                if last_error is not None:
                    raise error from last_error
                raise error
            openalex_source_id = str(openalex_source.get("id") or "")
            openalex_source_works = await client.fetch_openalex_works_by_source(
                openalex_source_id,
                from_pub_date=from_pub_date,
            )
            journal_row = build_openalex_journal_row(row, openalex_source)
            issn = journal_row.get("issn") or issn
            source_issns = candidate_issns_from_row(journal_row)
            works = [
                work
                for openalex_work in openalex_source_works
                if (
                    work := build_openalex_crossref_work(
                        openalex_work,
                        source_issns,
                    )
                )
            ]
            fallback_openalex_by_doi = {
                doi: openalex_work
                for openalex_work in openalex_source_works
                if (doi := normalize_doi(openalex_work.get("doi")))
            }
            if not works:
                raise JournalPathError(
                    "OpenAlex fallback returned no usable works: "
                    f"{journal_title_from_row(row)}"
                )

        if latest_existing_issue is not None:
            works = select_scholarly_update_works(
                journal_id,
                works,
                existing_issue_ids,
                latest_existing_issue[0],
            )

        stats.record_path_counts(path_key, works_count=len(works))

        if openalex_source is None:
            journal_row = dict(row)
            journal_row["issn"] = issn
        journal_record = build_scholarly_journal_record(journal_id, journal_row, works)
        if latest_existing_issue is not None:
            journal_record["has_articles"] = 1
        meta_record = build_meta_record(journal_id, csv_path, row)
        if openalex_source is None:
            meta_record.update(
                build_resolved_meta_fields(
                    "crossref",
                    issn,
                    journal_record.get("title"),
                    journal_record.get("issn"),
                    journal_record.get("eissn"),
                )
            )
        else:
            meta_record.update(build_openalex_resolved_meta_fields(openalex_source))
        journal_title = journal_record.get("title") or row.get("title") or ""

        await upsert_journal(db, journal_record)
        await upsert_meta(db, meta_record)
        await db.commit()

        dois = [
            doi for doi in [normalize_doi(work.get("DOI")) for work in works] if doi
        ]
        openalex_by_doi = (
            fallback_openalex_by_doi or await client.fetch_openalex_by_dois(dois)
        )
        semantic_scholar_by_doi = await client.fetch_semantic_scholar_by_dois(dois)

        issue_records_by_id: dict[int, dict[str, Any]] = {}
        article_records: list[dict[str, Any]] = []
        processed_years: set[int] = set()
        for work in works:
            issue_record = build_scholarly_issue_record(journal_id, work)
            issue_id = None
            if issue_record:
                issue_id = issue_record["issue_id"]
                issue_records_by_id[issue_id] = issue_record
                year = issue_record.get("publication_year")
            if isinstance(year, int):
                processed_years.add(year)
            doi = normalize_doi(work.get("DOI"))
            embedded_openalex_work = work.get("_openalex_work")
            if not isinstance(embedded_openalex_work, dict):
                embedded_openalex_work = None
            article_record = build_scholarly_article_record(
                work,
                openalex_by_doi.get(doi or "") or embedded_openalex_work,
                semantic_scholar_by_doi.get(doi or ""),
                journal_id,
                issue_id,
            )
            if article_record:
                article_records.append(article_record)

        issue_records = list(issue_records_by_id.values())
        if issue_records:
            stats.record_path_counts(path_key, issues_count=len(issue_records))
            await upsert_issues(db, issue_records)
            if update:
                await refresh_article_listing_for_issues(
                    db, [record["issue_id"] for record in issue_records]
                )

        article_records, deleted_article_ids = split_article_records_by_authors(
            article_records
        )
        if deleted_article_ids:
            stats.record_path_counts(
                path_key,
                articles_deleted_no_authors_count=len(deleted_article_ids),
            )
            await delete_articles(db, deleted_article_ids)

        if article_records:
            for batch in chunked(article_records, 500):
                await upsert_articles(db, batch)
                await upsert_article_search(db, batch, journal_title)
                await refresh_article_listing_for_articles(
                    db, list({record["article_id"] for record in batch})
                )
                stats.record_path_counts(
                    path_key,
                    articles_written_count=len(batch),
                )

        years = sorted(processed_years, reverse=True)
        progress = None
        if show_year_progress:
            progress = tqdm(total=len(years), desc=f"Journal {journal_id} years")
        for year in years:
            await mark_year_done(db, journal_id, year)
            if progress:
                progress.update(1)
        if progress:
            progress.close()

        await mark_journal_done(db, journal_id)
        await db.commit()
        stats.record_path_finished("succeeded", path_key)
    except Exception as exc:
        stats.record_path_finished("failed", path_key, exc)
        raise
    finally:
        stats.clear_current_path()


async def process_cnki_journal(
    db: DatabaseClient,
    client: CnkiClient,
    csv_path: Path,
    row: dict[str, str],
    issue_batch_size: int,
    request_workers: int,
    show_year_progress: bool,
    resume: bool,
    update: bool,
    stats_recorder: IndexStatsRecorder | NoOpIndexStatsRecorder | None = None,
) -> None:
    """
    Export one CNKI journal to the database.

    Args:
        db: Database client.
        client: CNKI client.
        csv_path: Source CSV path.
        row: CSV row for the journal.
        issue_batch_size: Number of issues per fetch batch.
        request_workers: Maximum concurrent HTTP requests.
        show_year_progress: Whether to display year progress with tqdm.
        resume: Whether to resume from completed years and journals.
        update: Whether to perform incremental updates for existing years.
        stats_recorder: Optional index statistics recorder.

    Returns:
        None.
    """
    stats = stats_recorder_or_noop(stats_recorder)
    journal_id = build_journal_id(row)
    path_key = stats.record_path_started(
        CNKI_SOURCE,
        "journal",
        journal_id,
        journal_title_from_row(row),
    )
    try:
        if journal_id is None:
            raise JournalPathError(
                f"CNKI journal missing id: {journal_title_from_row(row)}"
            )

        if resume and not update and await is_journal_complete(db, journal_id):
            stats.record_path_finished("resumed", path_key)
            return

        details = await client.resolve_journal(row)
        if not details:
            raise JournalPathError(
                f"No CNKI details for journal: {journal_title_from_row(row)}"
            )

        journal_record = build_cnki_journal_record(journal_id, row, details)
        meta_record = build_meta_record(journal_id, csv_path, row)
        journal_title = journal_record.get("title") or row.get("title") or ""
        journal_code = str(details["pykm"])

        await upsert_journal(db, journal_record)
        await upsert_meta(db, meta_record)
        await db.commit()

        if resume and not update and await is_journal_complete(db, journal_id):
            stats.record_path_finished("resumed", path_key)
            return

        issues = await client.get_year_issues(details)
        if not issues:
            raise JournalPathError(
                f"No CNKI publication years for journal {journal_code}"
            )
        stats.record_path_counts(path_key, issues_count=len(issues))

        issues_by_year: dict[int, list[dict[str, Any]]] = {}
        for issue in issues:
            year = issue.get("year")
            if isinstance(year, int):
                issues_by_year.setdefault(year, []).append(issue)

        issue_records_by_year: dict[int, list[dict[str, Any]]] = {}
        issue_pairs_by_year: dict[int, list[tuple[int, dict[str, Any]]]] = {}
        for year in sorted(issues_by_year, reverse=True):
            for issue in issues_by_year.get(year, []):
                record = build_cnki_issue_record(journal_id, journal_code, issue)
                if record:
                    issue_records_by_year.setdefault(year, []).append(record)
                    issue_pairs_by_year.setdefault(year, []).append(
                        (record["issue_id"], issue)
                    )

        completed_years: set[int] = set()
        if resume and not update:
            completed_years = await get_completed_years(db, journal_id)

        selected_update_issue_ids: set[int] | None = None
        if update:
            ordered_issue_ids = [
                issue_id
                for year in sorted(issue_pairs_by_year, reverse=True)
                for issue_id, _issue in issue_pairs_by_year.get(year, [])
            ]
            existing_issue_ids = await get_journal_issue_ids_with_articles(
                db, journal_id
            )
            selected_update_issue_ids = set(
                select_recent_update_issue_ids(ordered_issue_ids, existing_issue_ids)
            )
            years_to_process = [
                year
                for year in sorted(issue_pairs_by_year, reverse=True)
                if any(
                    issue_id in selected_update_issue_ids
                    for issue_id, _issue in issue_pairs_by_year.get(year, [])
                )
            ]
        else:
            years_to_process = [
                year
                for year in sorted(issue_pairs_by_year, reverse=True)
                if year not in completed_years
            ]

        progress = None
        if show_year_progress:
            progress = tqdm(
                total=len(years_to_process),
                desc=f"Journal {journal_id} years",
                unit="year",
            )

        detail_workers = max(1, min(request_workers, CNKI_DETAIL_WORKER_LIMIT))
        semaphore = asyncio.Semaphore(detail_workers)
        for index, year in enumerate(years_to_process, start=1):
            if progress:
                progress.set_postfix_str(f"{year} ({index}/{len(years_to_process)})")

            issue_records = issue_records_by_year.get(year, [])
            issue_pairs = issue_pairs_by_year.get(year, [])
            if selected_update_issue_ids is not None:
                issue_records = [
                    record
                    for record in issue_records
                    if record["issue_id"] in selected_update_issue_ids
                ]
                issue_pairs = [
                    pair for pair in issue_pairs if pair[0] in selected_update_issue_ids
                ]

            if issue_records:
                await upsert_issues(db, issue_records)

            issue_pairs_to_fetch = issue_pairs
            for batch in chunked(issue_pairs_to_fetch, issue_batch_size):
                batch_records = await fetch_cnki_issue_batch(
                    client,
                    semaphore,
                    details,
                    journal_id,
                    batch,
                    stats,
                    path_key,
                )
                batch_records, deleted_article_ids = split_article_records_by_authors(
                    batch_records
                )
                if deleted_article_ids:
                    stats.record_path_counts(
                        path_key,
                        articles_deleted_no_authors_count=len(deleted_article_ids),
                    )
                    await delete_articles(db, deleted_article_ids)
                if batch_records:
                    await upsert_articles(db, batch_records)
                    await upsert_article_search(db, batch_records, journal_title)
                    await refresh_article_listing_for_articles(
                        db, list({record["article_id"] for record in batch_records})
                    )
                    stats.record_path_counts(
                        path_key,
                        articles_written_count=len(batch_records),
                    )

            await mark_year_done(db, journal_id, year)
            await db.commit()
            if progress:
                progress.update(1)

        if progress:
            progress.close()

        await mark_journal_done(db, journal_id)
        await db.commit()
        stats.record_path_finished("succeeded", path_key)
    except Exception as exc:
        stats.record_path_finished("failed", path_key, exc)
        raise
    finally:
        stats.clear_current_path()


async def fetch_cnki_issue_batch(
    client: CnkiClient,
    semaphore: asyncio.Semaphore,
    journal: dict[str, Any],
    journal_id: int,
    issue_pairs: list[tuple[int, dict[str, Any]]],
    stats_recorder: IndexStatsRecorder | NoOpIndexStatsRecorder,
    path_key: PathStatsKey,
) -> list[dict[str, Any]]:
    """
    Fetch CNKI article details for an issue batch.

    Args:
        client: CNKI client.
        semaphore: Request concurrency limiter.
        journal: CNKI journal detail payload.
        journal_id: Internal journal ID.
        issue_pairs: Database issue ID and upstream issue payload pairs.
        stats_recorder: Index statistics recorder.
        path_key: Path statistics key.

    Returns:
        Article records.
    """
    summary_tasks = [
        asyncio.create_task(
            fetch_cnki_issue_summaries(client, semaphore, journal, issue_id, issue)
        )
        for issue_id, issue in issue_pairs
    ]
    summary_pairs: list[tuple[int, dict[str, Any]]] = []
    for summary_completed in asyncio.as_completed(summary_tasks):
        completed_pairs = await summary_completed
        summary_pairs.extend(completed_pairs)
        stats_recorder.record_path_counts(
            path_key,
            article_summaries_count=len(completed_pairs),
        )

    detail_tasks = [
        asyncio.create_task(
            fetch_cnki_article_detail(client, semaphore, issue_id, summary)
        )
        for issue_id, summary in summary_pairs
        if summary.get("article_url")
    ]
    records: list[dict[str, Any]] = []
    for detail_completed in asyncio.as_completed(detail_tasks):
        issue_id, summary, detail = await detail_completed
        stats_recorder.record_path_counts(path_key, article_details_count=1)
        record = build_cnki_article_record(detail, summary, journal_id, issue_id)
        if record:
            records.append(record)
    return records


async def fetch_cnki_issue_summaries(
    client: CnkiClient,
    semaphore: asyncio.Semaphore,
    journal: dict[str, Any],
    issue_id: int,
    issue: dict[str, Any],
) -> list[tuple[int, dict[str, Any]]]:
    """
    Fetch CNKI article summaries for one issue.

    Args:
        client: CNKI client.
        semaphore: Request concurrency limiter.
        journal: CNKI journal detail payload.
        issue_id: Internal issue ID.
        issue: CNKI issue payload.

    Returns:
        Issue ID and article summary pairs.
    """
    async with semaphore:
        summaries = await client.get_issue_articles(journal, issue)
    return [(issue_id, summary) for summary in summaries]


async def fetch_cnki_article_detail(
    client: CnkiClient,
    semaphore: asyncio.Semaphore,
    issue_id: int,
    summary: dict[str, Any],
) -> tuple[int, dict[str, Any], dict[str, Any]]:
    """
    Fetch one CNKI article detail payload.

    Args:
        client: CNKI client.
        semaphore: Request concurrency limiter.
        issue_id: Internal issue ID.
        summary: Article summary.

    Returns:
        Tuple of issue ID, summary payload, and detail payload.
    """
    async with semaphore:
        article_url = str(summary["article_url"])
        last_error: Exception | None = None
        for attempt in range(CNKI_DETAIL_ATTEMPTS):
            try:
                detail = await client.get_article_detail(article_url)
                if detail.get("title") or detail.get("platform_id"):
                    return issue_id, summary, detail
            except Exception as exc:
                last_error = exc
            if attempt < CNKI_DETAIL_ATTEMPTS - 1:
                await asyncio.sleep(CNKI_DETAIL_RETRY_SECONDS * (attempt + 1))
        if last_error is not None:
            raise RuntimeError(f"CNKI detail failed for {article_url}") from last_error
        raise RuntimeError(f"CNKI detail missing for {article_url}")


async def process_journal(
    db: DatabaseClient,
    scholarly_client: ScholarlyClient,
    cnki_client: CnkiClient,
    csv_path: Path,
    row: dict[str, str],
    issue_batch_size: int,
    request_workers: int,
    show_year_progress: bool,
    resume: bool,
    update: bool,
    stats_recorder: IndexStatsRecorder | NoOpIndexStatsRecorder | None = None,
) -> None:
    """
    Export a single journal to the database.

    Args:
        db: Database client.
        scholarly_client: Scholarly metadata client.
        cnki_client: CNKI client.
        csv_path: Source CSV path.
        row: CSV row for the journal.
        issue_batch_size: Number of issues per fetch batch.
        request_workers: Maximum concurrent HTTP requests.
        show_year_progress: Whether to display year progress with tqdm.
        resume: Whether to resume from completed years and journals.
        update: Whether to perform incremental updates for existing years.
        stats_recorder: Optional index statistics recorder.

    Returns:
        None.
    """
    source = source_from_row(row)
    if source == CNKI_SOURCE:
        await process_cnki_journal(
            db,
            cnki_client,
            csv_path,
            row,
            issue_batch_size,
            request_workers,
            show_year_progress,
            resume,
            update,
            stats_recorder,
        )
        return
    if source == SCHOLARLY_SOURCE:
        await process_scholarly_journal(
            db,
            scholarly_client,
            csv_path,
            row,
            request_workers,
            show_year_progress,
            resume,
            update,
            stats_recorder,
        )
        return
    stats = stats_recorder_or_noop(stats_recorder)
    journal_id = build_journal_id(row)
    path_key = stats.record_path_started(
        source,
        "journal",
        journal_id,
        journal_title_from_row(row),
    )
    error = JournalPathError(
        f"Unsupported journal source {source}: {journal_title_from_row(row)}"
    )
    stats.record_path_finished("failed", path_key, error)
    stats.clear_current_path()
    raise error
