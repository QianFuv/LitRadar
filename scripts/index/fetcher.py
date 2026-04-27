"""Journal fetch and processing workflows."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

from tqdm import tqdm

from scripts.browzine import BrowZineAPIClient
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
    build_article_record,
    build_issue_record,
    build_journal_record,
    build_meta_record,
    build_weipu_article_record,
    build_weipu_issue_record,
    build_weipu_journal_record,
)
from scripts.shared.constants import DEFAULT_LIBRARY_ID, WEIPU_LIBRARY_ID
from scripts.shared.converters import chunked, is_weipu_library, to_int, to_int_stable
from scripts.weipu import WeipuAPISelectolax


async def fetch_issue_articles(
    semaphore: asyncio.Semaphore,
    client: BrowZineAPIClient,
    issue_id: int,
    library_id: str,
) -> tuple[int, list[dict[str, Any]] | None]:
    """
    Fetch articles for a single issue with concurrency control.

    Args:
        semaphore: Semaphore for limiting concurrent requests.
        client: BrowZine API client.
        issue_id: BrowZine issue ID.
        library_id: Library ID for the request.

    Returns:
        Tuple of issue ID and article list or None.
    """
    async with semaphore:
        articles = await client.get_articles_from_issue(issue_id, library_id)
    return issue_id, articles


async def fetch_weipu_issue_articles(
    semaphore: asyncio.Semaphore,
    client: WeipuAPISelectolax,
    journal_id: str,
    db_issue_id: int,
    weipu_issue_id: str,
) -> tuple[int, list[dict[str, Any]] | None]:
    """
    Fetch WeiPu articles for a single issue with concurrency control.

    Args:
        semaphore: Semaphore for limiting concurrent requests.
        client: WeiPu API client.
        journal_id: WeiPu journal ID.
        db_issue_id: Internal issue ID for the database.
        weipu_issue_id: WeiPu issue identifier.

    Returns:
        Tuple of database issue ID and article list or None.
    """
    async with semaphore:
        payload = await client.get_issue_articles(journal_id, weipu_issue_id)
    articles = payload.get("articles") if payload else None
    return db_issue_id, articles


async def process_weipu_journal(
    db: DatabaseClient,
    client: WeipuAPISelectolax,
    csv_path: Path,
    row: dict[str, str],
    issue_batch_size: int,
    request_workers: int,
    show_year_progress: bool,
    resume: bool,
    update: bool,
) -> None:
    """
    Export a single WeiPu journal to the database.

    Args:
        db: Database client.
        client: WeiPu API client.
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
    raw_journal_id = row.get("id")
    if not raw_journal_id:
        print(f"  - Skipping WeiPu journal with missing id: {row.get('title')}")
        return

    journal_id = to_int_stable(raw_journal_id, "weipu-journal")
    if not journal_id:
        print(f"  - Skipping WeiPu journal with invalid id: {row.get('title')}")
        return

    weipu_journal_id = str(raw_journal_id)
    details = await client.get_journal_details(weipu_journal_id)
    if not details:
        journal_match = None
        if row.get("issn"):
            journal_match = await client.search_journal_by_issn(row["issn"])
        if not journal_match and row.get("title"):
            journal_match = await client.search_journal_by_title(row["title"])
        if journal_match and journal_match.get("journalId"):
            weipu_journal_id = str(journal_match["journalId"])
            details = await client.get_journal_details(weipu_journal_id)

    if not details:
        print(f"  - No WeiPu details for journal {raw_journal_id}")
        return

    library_id = row.get("library") or WEIPU_LIBRARY_ID
    platform_journal_id = str(details.get("journalId") or weipu_journal_id)
    journal_record = build_weipu_journal_record(
        journal_id,
        library_id,
        platform_journal_id,
        row,
        details,
        details.get("totalIssues", 0) > 0,
    )
    meta_record = build_meta_record(journal_id, csv_path, row)
    journal_title = journal_record.get("title") or row.get("title") or ""

    await upsert_journal(db, journal_record)
    await upsert_meta(db, meta_record)
    await db.commit()

    years = details.get("years") or []
    if not years:
        print(f"  - No publication years for WeiPu journal {journal_id}")
        return

    if resume and not update and await is_journal_complete(db, journal_id):
        return

    completed_years: set[int] = set()
    if resume and not update:
        completed_years = await get_completed_years(db, journal_id)

    if update:
        years_to_process = years
    else:
        years_to_process = [
            year for year in years if year.get("year") not in completed_years
        ]
    total_years = len(years_to_process)
    progress = None
    if show_year_progress:
        progress = tqdm(
            total=total_years,
            desc=f"Journal {journal_id} years",
            unit="year",
        )

    semaphore = asyncio.Semaphore(max(1, request_workers))
    for index, year_entry in enumerate(years_to_process, start=1):
        year_value = year_entry.get("year")
        if not isinstance(year_value, int):
            if progress:
                progress.update(1)
            continue
        if progress:
            progress.set_postfix_str(f"{year_value} ({index}/{total_years})")
        issues = year_entry.get("issues") or []
        if not issues:
            if progress:
                progress.update(1)
            continue

        issue_records: list[dict[str, Any]] = []
        issue_pairs: list[tuple[int, str]] = []
        for issue in issues:
            record = build_weipu_issue_record(issue, journal_id, year_value)
            issue_id_value = issue.get("id")
            if record and issue_id_value is not None:
                issue_records.append(record)
                issue_pairs.append((record["issue_id"], str(issue_id_value)))

        if issue_records:
            await upsert_issues(db, issue_records)
        if update and issue_pairs:
            await refresh_article_listing_for_issues(
                db, [pair[0] for pair in issue_pairs]
            )

        issue_pairs_to_fetch = issue_pairs
        if update and issue_pairs:
            existing_issue_ids = await get_issue_ids_with_articles(
                db, journal_id, year_value
            )
            issue_pairs_to_fetch = [
                pair for pair in issue_pairs if pair[0] not in existing_issue_ids
            ]

        if issue_pairs_to_fetch:
            for batch in chunked(issue_pairs_to_fetch, issue_batch_size):
                tasks = [
                    asyncio.create_task(
                        fetch_weipu_issue_articles(
                            semaphore,
                            client,
                            weipu_journal_id,
                            db_issue_id,
                            weipu_issue_id,
                        )
                    )
                    for db_issue_id, weipu_issue_id in batch
                ]
                batch_records: list[dict[str, Any]] = []
                for completed in asyncio.as_completed(tasks):
                    try:
                        issue_id, articles = await completed
                    except Exception:
                        print("  - Failed to fetch WeiPu articles for an issue batch")
                        continue
                    if not articles:
                        continue
                    for article in articles:
                        record = build_weipu_article_record(
                            article, journal_id, issue_id
                        )
                        if record:
                            batch_records.append(record)
                if batch_records:
                    await upsert_articles(db, batch_records)
                    await upsert_article_search(db, batch_records, journal_title)
                    batch_article_ids = list(
                        {record["article_id"] for record in batch_records}
                    )
                    await refresh_article_listing_for_articles(db, batch_article_ids)

        if progress:
            progress.update(1)
        await mark_year_done(db, journal_id, year_value)
        await db.commit()

    if progress:
        progress.close()

    await mark_journal_done(db, journal_id)
    await db.commit()


async def process_journal(
    db: DatabaseClient,
    client: BrowZineAPIClient,
    weipu_client: WeipuAPISelectolax,
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
        client: BrowZine API client.
        weipu_client: WeiPu API client.
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
    if is_weipu_library(row.get("library")):
        await process_weipu_journal(
            db,
            weipu_client,
            csv_path,
            row,
            issue_batch_size,
            request_workers,
            show_year_progress,
            resume,
            update,
        )
        return

    journal_id = to_int(row.get("id"))
    if not journal_id:
        print(f"  - Skipping journal with missing id: {row.get('title')}")
        return

    library_id = row.get("library") or DEFAULT_LIBRARY_ID

    journal_info = await client.get_journal_info(journal_id, library_id)

    journal_record = build_journal_record(journal_id, library_id, row, journal_info)
    meta_record = build_meta_record(journal_id, csv_path, row)
    journal_title = journal_record.get("title") or row.get("title") or ""

    await upsert_journal(db, journal_record)
    await upsert_meta(db, meta_record)
    await db.commit()

    years = await client.get_publication_years(journal_id, library_id)
    if not years:
        print(f"  - No publication years for journal {journal_id}")
        return

    if resume and not update and await is_journal_complete(db, journal_id):
        return

    completed_years: set[int] = set()
    if resume and not update:
        completed_years = await get_completed_years(db, journal_id)

    seen_issue_ids: set[int] = set()
    if update:
        years_to_process = years
    else:
        years_to_process = [year for year in years if year not in completed_years]
    total_years = len(years_to_process)
    progress = None
    if show_year_progress:
        progress = tqdm(
            total=total_years,
            desc=f"Journal {journal_id} years",
            unit="year",
        )

    semaphore = asyncio.Semaphore(max(1, request_workers))
    for index, year in enumerate(years_to_process, start=1):
        if progress:
            progress.set_postfix_str(f"{year} ({index}/{total_years})")
        issues = await client.get_issues_by_year(journal_id, library_id, year)
        if not issues:
            if progress:
                progress.update(1)
            continue

        issue_records: list[dict[str, Any]] = []
        issue_ids: list[int] = []
        for issue in issues:
            record = build_issue_record(issue, journal_id, year)
            if record:
                issue_id = record["issue_id"]
                if issue_id in seen_issue_ids:
                    continue
                seen_issue_ids.add(issue_id)
                issue_records.append(record)
                issue_ids.append(issue_id)

        if issue_records:
            await upsert_issues(db, issue_records)
        if update and issue_ids:
            await refresh_article_listing_for_issues(db, issue_ids)

        issue_ids_to_fetch = issue_ids
        if update and issue_ids:
            existing_issue_ids = await get_issue_ids_with_articles(db, journal_id, year)
            issue_ids_to_fetch = [
                issue_id for issue_id in issue_ids if issue_id not in existing_issue_ids
            ]

        if issue_ids_to_fetch:
            for batch in chunked(issue_ids_to_fetch, issue_batch_size):
                tasks = [
                    asyncio.create_task(
                        fetch_issue_articles(semaphore, client, issue_id, library_id)
                    )
                    for issue_id in batch
                ]
                batch_records: list[dict[str, Any]] = []
                for completed in asyncio.as_completed(tasks):
                    try:
                        issue_id, articles = await completed
                    except Exception:
                        print("  - Failed to fetch articles for an issue batch")
                        continue
                    if not articles:
                        continue
                    for article in articles:
                        record = build_article_record(article, journal_id, issue_id)
                        if record:
                            batch_records.append(record)
                if batch_records:
                    await upsert_articles(db, batch_records)
                    await upsert_article_search(db, batch_records, journal_title)
                    batch_article_ids = list(
                        {record["article_id"] for record in batch_records}
                    )
                    await refresh_article_listing_for_articles(db, batch_article_ids)

        if progress:
            progress.update(1)
        await mark_year_done(db, journal_id, year)
        await db.commit()

    if progress:
        progress.close()

    in_press = await client.get_articles_in_press(journal_id, library_id)
    if in_press:
        in_press_records = []
        for article in in_press:
            record = build_article_record(article, journal_id, None)
            if record:
                in_press_records.append(record)
        await upsert_articles(db, in_press_records)
        await upsert_article_search(db, in_press_records, journal_title)
        in_press_article_ids = list(
            {record["article_id"] for record in in_press_records}
        )
        await refresh_article_listing_for_articles(db, in_press_article_ids)
        await db.commit()

    await mark_journal_done(db, journal_id)
    await db.commit()
