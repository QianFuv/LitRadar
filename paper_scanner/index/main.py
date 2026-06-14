"""Main entrypoint for index command."""

from __future__ import annotations

import argparse
import asyncio
import csv
import multiprocessing as mp
from pathlib import Path

import aiosqlite

from paper_scanner.index.changes import (
    collect_article_snapshot,
    compute_changed_group_keys,
    run_notify_for_manifest,
    write_change_manifest,
)
from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import mark_listing_ready
from paper_scanner.index.db.schema import init_db, optimize_db
from paper_scanner.index.fetcher import process_journal
from paper_scanner.index.workers import run_worker_batch, writer_process
from paper_scanner.shared.constants import (
    CNKI_SOURCE,
    DB_TIMEOUT_SECONDS,
    PROJECT_ROOT,
    SCHOLARLY_SOURCE,
)
from paper_scanner.shared.runtime_config import apply_runtime_config
from paper_scanner.sources.cnki import CnkiClient
from paper_scanner.sources.scholarly import ScholarlyClient


def load_csv_rows(csv_path: Path) -> list[dict[str, str]]:
    """
    Load CSV rows and ensure the source column exists.

    Args:
        csv_path: Path to the CSV file.

    Returns:
        List of CSV row dictionaries.
    """
    with open(csv_path, encoding="utf-8") as handle:
        reader = csv.DictReader(handle)
        rows = list(reader)
    if not rows:
        return []
    if "source" not in rows[0]:
        for row in rows:
            row["source"] = SCHOLARLY_SOURCE
    for row in rows:
        if not row.get("source"):
            row["source"] = SCHOLARLY_SOURCE
        row["source"] = row["source"].strip().lower()
    return rows


def validate_sources(rows: list[dict[str, str]]) -> list[dict[str, str]]:
    """
    Validate CSV source values.

    Args:
        rows: CSV rows.

    Returns:
        Validated CSV rows.
    """
    allowed = {SCHOLARLY_SOURCE, CNKI_SOURCE}
    for row in rows:
        source = (row.get("source") or SCHOLARLY_SOURCE).strip().lower()
        if source not in allowed:
            title = row.get("title") or row.get("id") or "Unknown"
            raise ValueError(f"Unsupported source for {title}: {source}")
        row["source"] = source
    return rows


async def export_csv(
    csv_path: Path,
    db_path: Path,
    issue_batch_size: int,
    thread_workers: int,
    processes: int,
    timeout: int,
    resume: bool,
    update: bool,
) -> None:
    """
    Export a CSV file to a SQLite database.

    Args:
        csv_path: Path to the CSV file.
        db_path: Output SQLite database path.
        issue_batch_size: Number of issues per fetch batch.
        thread_workers: Maximum concurrent HTTP requests.
        processes: Process workers for journal-level parallelism.
        timeout: HTTP request timeout in seconds.
        resume: Whether to resume from completed years and journals.
        update: Whether to perform incremental updates for existing years.

    Returns:
        None.
    """
    rows = load_csv_rows(csv_path)
    if not rows:
        print(f"Skipping empty CSV: {csv_path.name}")
        return

    print(f"\nProcessing {csv_path.name} -> {db_path.name}")
    rows = validate_sources(rows)

    if processes <= 1:
        scholarly_client = ScholarlyClient(
            timeout=timeout,
            worker_id=0,
            process_count=1,
        )
        cnki_client = CnkiClient(timeout=timeout)
        async with aiosqlite.connect(db_path, timeout=DB_TIMEOUT_SECONDS) as db:
            await init_db(db)
            local_db = LocalDatabaseClient(db)
            await local_db.start()
            try:
                for index, row in enumerate(rows, start=1):
                    title = row.get("title", "Unknown")
                    print(f"  [{index}/{len(rows)}] Exporting {title}")
                    await process_journal(
                        local_db,
                        scholarly_client,
                        cnki_client,
                        csv_path,
                        row,
                        issue_batch_size,
                        thread_workers,
                        True,
                        resume,
                        update,
                    )
            finally:
                await local_db.close()
                await optimize_db(db)
                if not update:
                    await mark_listing_ready(db)
                    await db.commit()
                await scholarly_client.aclose()
                await cnki_client.aclose()
        return

    ctx = mp.get_context()
    request_queue = ctx.Queue()
    response_queues = [ctx.Queue() for _ in range(processes)]
    status_queue = ctx.Queue()
    writer = ctx.Process(
        target=writer_process, args=(str(db_path), request_queue, response_queues)
    )
    writer.start()

    workers: list[mp.Process] = []
    for worker_id in range(processes):
        worker_rows = rows[worker_id::processes]
        if not worker_rows:
            continue
        worker = ctx.Process(
            target=run_worker_batch,
            args=(
                worker_id,
                processes,
                request_queue,
                response_queues[worker_id],
                status_queue,
                str(csv_path),
                worker_rows,
                issue_batch_size,
                thread_workers,
                timeout,
                resume,
                update,
            ),
        )
        worker.start()
        workers.append(worker)

    completed = 0
    total = len(rows)
    try:
        while completed < total:
            message = await asyncio.to_thread(status_queue.get)
            if message is None:
                continue
            completed += 1
            if message.get("ok"):
                title = message.get("title") or message.get("journal_id") or "Unknown"
                print(f"  Finished {title}")
            else:
                title = message.get("title") or message.get("journal_id") or "Unknown"
                error = message.get("error") or "Unknown error"
                print(f"  - Journal worker failed: {title} ({error})")
    finally:
        request_queue.put({"type": "stop"})
        writer.join()
        for worker in workers:
            worker.join()

    async with aiosqlite.connect(db_path, timeout=DB_TIMEOUT_SECONDS) as db:
        await optimize_db(db)
        if not update:
            await mark_listing_ready(db)
            await db.commit()


async def async_main(args: argparse.Namespace) -> None:
    """
    Run export process for all target CSV files.

    Args:
        args: Parsed CLI arguments.

    Returns:
        None.
    """
    project_root = PROJECT_ROOT
    apply_runtime_config()
    meta_dir = project_root / "data" / "meta"
    index_dir = project_root / "data" / "index"
    index_dir.mkdir(parents=True, exist_ok=True)

    if not meta_dir.exists():
        print(f"Directory not found: {meta_dir}")
        return

    if args.file:
        csv_paths = [meta_dir / args.file]
        if not csv_paths[0].exists():
            print(f"CSV not found: {csv_paths[0]}")
            return
    else:
        csv_paths = sorted(meta_dir.glob("*.csv"))

    if not csv_paths:
        print(f"No CSV files found in {meta_dir}")
        return

    issue_batch_size = max(1, args.issue_batch or args.workers)

    print("=" * 60)
    print("Paper Scanner Article Indexer")
    print("=" * 60)
    print(f"Found {len(csv_paths)} CSV file(s)")
    print(f"Request workers: {args.workers}")
    print(f"Process workers: {args.processes}")
    print(f"Issue batch size: {issue_batch_size}")
    if args.update:
        print("Change tracking: enabled (article-level diff)")

    manifest_records: list[tuple[Path, Path]] = []

    for csv_path in csv_paths:
        db_path = index_dir / f"{csv_path.stem}.sqlite"
        before_issue_map: dict[str, set[int]] = {}
        before_inpress_map: dict[int, set[int]] = {}
        if args.update and db_path.exists():
            before_issue_map, before_inpress_map = collect_article_snapshot(db_path)

        await export_csv(
            csv_path,
            db_path,
            issue_batch_size,
            args.workers,
            args.processes,
            args.timeout,
            args.resume,
            args.update,
        )

        if args.update and db_path.exists():
            after_issue_map, after_inpress_map = collect_article_snapshot(db_path)
            changed_issue_keys, changed_inpress_ids, summary = (
                compute_changed_group_keys(
                    before_issue_map,
                    after_issue_map,
                    before_inpress_map,
                    after_inpress_map,
                )
            )
            manifest_path = write_change_manifest(
                db_path,
                changed_issue_keys,
                changed_inpress_ids,
                summary,
            )
            manifest_records.append((db_path, manifest_path))
            print(
                "  Change manifest:",
                manifest_path,
                f"(issues={len(changed_issue_keys)}, "
                f"inpress={len(changed_inpress_ids)}, "
                f"added={summary['added_article_count']}, "
                f"removed={summary['removed_article_count']})",
            )

    if args.notify and args.update:
        for db_path, manifest_path in manifest_records:
            print(f"Running notify for {db_path.name}")
            return_code = run_notify_for_manifest(
                db_path,
                manifest_path,
                args.notify_dry_run,
            )
            if return_code != 0:
                print(
                    f"  - notify failed for {db_path.name} with exit code {return_code}"
                )

    print("\nDone.")


def main() -> None:
    """
    Parse CLI arguments and run the exporter.

    Args:
        None.

    Returns:
        None.
    """
    parser = argparse.ArgumentParser(
        description="Export journal articles to SQLite databases"
    )
    parser.add_argument(
        "--file",
        "-f",
        type=str,
        help="Specific CSV filename under data/meta (e.g., utd24.csv)",
    )
    parser.add_argument(
        "--workers",
        "-w",
        type=int,
        default=32,
        help="Maximum concurrent HTTP requests",
    )
    parser.add_argument(
        "--issue-batch",
        type=int,
        default=0,
        help="Issues per async batch (default: workers)",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=20,
        help="HTTP request timeout in seconds",
    )
    parser.add_argument(
        "--processes",
        type=int,
        default=2,
        help="Process workers for journal-level parallelism",
    )
    parser.add_argument(
        "--resume",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Resume from completed years and journals",
    )
    parser.add_argument(
        "--update",
        action=argparse.BooleanOptionalAction,
        default=False,
        help="Incrementally update existing years and journals",
    )
    parser.add_argument(
        "--notify",
        action=argparse.BooleanOptionalAction,
        default=False,
        help="Run notify after update using the generated change manifest",
    )
    parser.add_argument(
        "--notify-dry-run",
        action=argparse.BooleanOptionalAction,
        default=False,
        help="Run notify with --dry-run when --notify is enabled",
    )
    args = parser.parse_args()

    if args.notify and not args.update:
        parser.error("--notify requires --update")

    asyncio.run(async_main(args))


if __name__ == "__main__":
    main()
