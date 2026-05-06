"""Journal fetch and processing workflows."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

from tqdm import tqdm

from scripts.cnki import CnkiClient
from scripts.index.db.client import DatabaseClient
from scripts.index.db.operations import (
    get_completed_years,
    get_issue_ids_with_articles,
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
from scripts.index.transforms import (
    build_cnki_article_record,
    build_cnki_issue_record,
    build_cnki_journal_record,
    build_journal_id,
    build_meta_record,
    build_scholarly_article_record,
    build_scholarly_issue_record,
    build_scholarly_journal_record,
    normalize_doi,
    source_from_row,
)
from scripts.scholarly import ScholarlyClient
from scripts.shared.constants import CNKI_SOURCE, SCHOLARLY_SOURCE
from scripts.shared.converters import chunked

CNKI_DETAIL_ATTEMPTS = 3
CNKI_DETAIL_RETRY_SECONDS = 1.0
CNKI_DETAIL_WORKER_LIMIT = 32


def select_update_issue_ids(
    issue_ids: list[int],
    existing_issue_ids: set[int],
    has_refreshed_latest_existing_issue: bool,
) -> tuple[list[int], bool]:
    """
    Select issues to fetch during an update run.

    Args:
        issue_ids: Issue IDs in upstream order.
        existing_issue_ids: Issue IDs that already have articles.
        has_refreshed_latest_existing_issue: Whether an existing issue was refreshed.

    Returns:
        Tuple of issue IDs to fetch and updated refresh state.
    """
    issue_ids_to_fetch = [
        issue_id for issue_id in issue_ids if issue_id not in existing_issue_ids
    ]
    if has_refreshed_latest_existing_issue:
        return issue_ids_to_fetch, has_refreshed_latest_existing_issue

    for issue_id in issue_ids:
        if issue_id in existing_issue_ids:
            issue_ids_to_fetch.append(issue_id)
            return issue_ids_to_fetch, True
    return issue_ids_to_fetch, False


async def process_scholarly_journal(
    db: DatabaseClient,
    client: ScholarlyClient,
    csv_path: Path,
    row: dict[str, str],
    request_workers: int,
    show_year_progress: bool,
    resume: bool,
    update: bool,
) -> None:
    """
    Export one Crossref/OpenAlex/Unpaywall journal to the database.

    Args:
        db: Database client.
        client: Scholarly metadata client.
        csv_path: Source CSV path.
        row: CSV row for the journal.
        request_workers: Maximum concurrent HTTP requests.
        show_year_progress: Whether to display year progress with tqdm.
        resume: Whether to resume from completed journals.
        update: Whether to refresh existing journal data.

    Returns:
        None.
    """
    journal_id = build_journal_id(row)
    if journal_id is None:
        print(f"  - Skipping scholarly journal with missing id: {row.get('title')}")
        return
    if resume and not update and await is_journal_complete(db, journal_id):
        return

    issn = (row.get("issn") or row.get("id") or "").strip()
    if not issn:
        print(f"  - Skipping scholarly journal with missing ISSN: {row.get('title')}")
        return

    works = await client.fetch_journal_works(issn)
    journal_record = build_scholarly_journal_record(journal_id, row, works)
    meta_record = build_meta_record(journal_id, csv_path, row)
    journal_title = journal_record.get("title") or row.get("title") or ""

    await upsert_journal(db, journal_record)
    await upsert_meta(db, meta_record)
    await db.commit()

    dois = [doi for doi in [normalize_doi(work.get("DOI")) for work in works] if doi]
    openalex_by_doi = await client.fetch_openalex_by_dois(dois)
    unpaywall_by_doi = await client.fetch_unpaywall_by_dois(
        dois, request_workers=request_workers
    )

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
        article_record = build_scholarly_article_record(
            work,
            openalex_by_doi.get(doi or ""),
            unpaywall_by_doi.get(doi or ""),
            journal_id,
            issue_id,
        )
        if article_record:
            article_records.append(article_record)

    issue_records = list(issue_records_by_id.values())
    if issue_records:
        await upsert_issues(db, issue_records)
        if update:
            await refresh_article_listing_for_issues(
                db, [record["issue_id"] for record in issue_records]
            )

    if article_records:
        for batch in chunked(article_records, 500):
            await upsert_articles(db, batch)
            await upsert_article_search(db, batch, journal_title)
            await refresh_article_listing_for_articles(
                db, list({record["article_id"] for record in batch})
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

    Returns:
        None.
    """
    journal_id = build_journal_id(row)
    if journal_id is None:
        print(f"  - Skipping CNKI journal with missing id: {row.get('title')}")
        return

    if resume and not update and await is_journal_complete(db, journal_id):
        return

    details = await client.resolve_journal(row)
    if not details:
        print(f"  - No CNKI details for journal {row.get('title')}")
        return

    journal_record = build_cnki_journal_record(journal_id, row, details)
    meta_record = build_meta_record(journal_id, csv_path, row)
    journal_title = journal_record.get("title") or row.get("title") or ""
    journal_code = str(details["pykm"])

    await upsert_journal(db, journal_record)
    await upsert_meta(db, meta_record)
    await db.commit()

    if resume and not update and await is_journal_complete(db, journal_id):
        return

    issues = await client.get_year_issues(details)
    if not issues:
        print(f"  - No CNKI publication years for journal {journal_code}")
        return

    completed_years: set[int] = set()
    if resume and not update:
        completed_years = await get_completed_years(db, journal_id)

    issues_by_year: dict[int, list[dict[str, Any]]] = {}
    for issue in issues:
        year = issue.get("year")
        if isinstance(year, int):
            issues_by_year.setdefault(year, []).append(issue)

    if update:
        years_to_process = sorted(issues_by_year, reverse=True)
    else:
        years_to_process = [
            year
            for year in sorted(issues_by_year, reverse=True)
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
    has_refreshed_latest_existing_issue = False
    for index, year in enumerate(years_to_process, start=1):
        if progress:
            progress.set_postfix_str(f"{year} ({index}/{len(years_to_process)})")

        issue_records: list[dict[str, Any]] = []
        issue_pairs: list[tuple[int, dict[str, Any]]] = []
        for issue in issues_by_year.get(year, []):
            record = build_cnki_issue_record(journal_id, journal_code, issue)
            if record:
                issue_records.append(record)
                issue_pairs.append((record["issue_id"], issue))

        if issue_records:
            await upsert_issues(db, issue_records)
        if update and issue_pairs:
            await refresh_article_listing_for_issues(
                db, [pair[0] for pair in issue_pairs]
            )

        issue_pairs_to_fetch = issue_pairs
        if update and issue_pairs:
            existing_issue_ids = await get_issue_ids_with_articles(db, journal_id, year)
            selected_issue_ids, has_refreshed_latest_existing_issue = (
                select_update_issue_ids(
                    [pair[0] for pair in issue_pairs],
                    existing_issue_ids,
                    has_refreshed_latest_existing_issue,
                )
            )
            issue_pair_map = {pair[0]: pair for pair in issue_pairs}
            issue_pairs_to_fetch = [
                issue_pair_map[issue_id] for issue_id in selected_issue_ids
            ]

        for batch in chunked(issue_pairs_to_fetch, issue_batch_size):
            batch_records = await fetch_cnki_issue_batch(
                client, semaphore, details, journal_id, batch
            )
            if batch_records:
                await upsert_articles(db, batch_records)
                await upsert_article_search(db, batch_records, journal_title)
                await refresh_article_listing_for_articles(
                    db, list({record["article_id"] for record in batch_records})
                )

        await mark_year_done(db, journal_id, year)
        await db.commit()
        if progress:
            progress.update(1)

    if progress:
        progress.close()

    await mark_journal_done(db, journal_id)
    await db.commit()


async def fetch_cnki_issue_batch(
    client: CnkiClient,
    semaphore: asyncio.Semaphore,
    journal: dict[str, Any],
    journal_id: int,
    issue_pairs: list[tuple[int, dict[str, Any]]],
) -> list[dict[str, Any]]:
    """
    Fetch CNKI article details for an issue batch.

    Args:
        client: CNKI client.
        semaphore: Request concurrency limiter.
        journal: CNKI journal detail payload.
        journal_id: Internal journal ID.
        issue_pairs: Database issue ID and upstream issue payload pairs.

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
        summary_pairs.extend(await summary_completed)

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
        )
        return
    print(f"  - Skipping journal with unknown source {source}: {row.get('title')}")
