"""Main entrypoint for index command."""

from __future__ import annotations

import argparse
import asyncio
import csv
import multiprocessing as mp
import os
import uuid
from pathlib import Path

import aiosqlite

from paper_scanner.index.changes import (
    collect_article_snapshot,
    compute_changed_group_keys,
    run_notify_for_manifest,
    write_change_manifest,
)
from paper_scanner.index.db.client import LocalDatabaseClient
from paper_scanner.index.db.operations import (
    is_article_listing_complete,
    mark_listing_ready,
    persist_index_run_stats,
)
from paper_scanner.index.db.schema import init_db, optimize_db
from paper_scanner.index.fetcher import process_journal
from paper_scanner.index.stats import (
    IndexRunStats,
    IndexStatsRecorder,
    sanitize_error_sample,
)
from paper_scanner.index.workers import run_worker_batch, writer_process
from paper_scanner.shared.constants import (
    CNKI_SOURCE,
    DB_TIMEOUT_SECONDS,
    PROJECT_ROOT,
    SCHOLARLY_SOURCE,
)
from paper_scanner.shared.request_pools import build_value_pool
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


def validate_required_source_config(rows: list[dict[str, str]]) -> None:
    """
    Validate required runtime settings for source rows.

    Args:
        rows: Validated CSV rows.

    Returns:
        None.

    Raises:
        SystemExit: If a required source setting is missing.
    """
    has_scholarly_rows = any(row.get("source") == SCHOLARLY_SOURCE for row in rows)
    openalex_keys = build_value_pool(os.getenv("OPENALEX_API_KEY_POOL"))
    semantic_scholar_keys = build_value_pool(os.getenv("SEMANTIC_SCHOLAR_API_KEY_POOL"))
    if has_scholarly_rows and not openalex_keys:
        raise SystemExit("OpenAlex API key is required for scholarly indexing.")
    if has_scholarly_rows and not semantic_scholar_keys:
        raise SystemExit("Semantic Scholar API key is required for scholarly indexing.")


def build_index_run_id(csv_path: Path) -> str:
    """
    Build a unique index run identifier for one CSV file.

    Args:
        csv_path: Source CSV path.

    Returns:
        Unique run identifier.
    """
    return f"{csv_path.stem}-{uuid.uuid4().hex}"


def index_error_summary(errors: list[str]) -> str | None:
    """
    Build a compact run error summary.

    Args:
        errors: Error messages.

    Returns:
        Compact error summary or None.
    """
    if not errors:
        return None
    sanitized_errors = [
        sanitized for error in errors[:3] if (sanitized := sanitize_error_sample(error))
    ]
    return "; ".join(sanitized_errors) or None


def print_index_run_summary(stats: IndexRunStats) -> None:
    """
    Print a compact index statistics summary.

    Args:
        stats: Index run statistics.

    Returns:
        None.
    """
    print(
        "  Index run stats:",
        f"status={stats.status}",
        f"journals={stats.total_journals()}",
        f"succeeded={stats.succeeded_journals()}",
        f"failed={stats.failed_journals()}",
        f"resumed={stats.resumed_journals()}",
    )
    service_counts: dict[str, int] = {}
    for api_stats in stats.api_stats.values():
        service_counts[api_stats.key.service] = (
            service_counts.get(api_stats.key.service, 0) + api_stats.logical_calls
        )
    if service_counts:
        service_text = ", ".join(
            f"{service}={count}" for service, count in sorted(service_counts.items())
        )
        print(f"  API calls: {service_text}")


async def persist_final_index_stats(db_path: Path, stats: IndexRunStats) -> None:
    """
    Persist final index run statistics to a database.

    Args:
        db_path: SQLite database path.
        stats: Final index run statistics.

    Returns:
        None.
    """
    async with aiosqlite.connect(db_path, timeout=DB_TIMEOUT_SECONDS) as db:
        await init_db(db)
        local_db = LocalDatabaseClient(db)
        await local_db.start()
        try:
            await persist_index_run_stats(local_db, stats)
        finally:
            await local_db.close()


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
    validate_required_source_config(rows)
    stats_recorder = IndexStatsRecorder(build_index_run_id(csv_path), csv_path.name)

    if processes <= 1:
        scholarly_client = ScholarlyClient(
            timeout=timeout,
            worker_id=0,
            process_count=1,
            stats_recorder=stats_recorder,
        )
        cnki_client = CnkiClient(timeout=timeout, stats_recorder=stats_recorder)
        run_error: Exception | None = None
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
                        stats_recorder,
                    )
            except Exception as exc:
                run_error = exc
            finally:
                if run_error is None:
                    stats_recorder.stats.finish("succeeded")
                else:
                    stats_recorder.stats.finish(
                        "failed",
                        sanitize_error_sample(run_error),
                    )
                await persist_index_run_stats(local_db, stats_recorder.stats)
                await local_db.close()
                if run_error is None:
                    await optimize_db(db)
                if run_error is None and (
                    not update or await is_article_listing_complete(db)
                ):
                    await mark_listing_ready(db)
                    await db.commit()
                await scholarly_client.aclose()
                await cnki_client.aclose()
        print_index_run_summary(stats_recorder.stats)
        if run_error is not None:
            error_text = sanitize_error_sample(run_error) or type(run_error).__name__
            raise RuntimeError(
                f"Index run failed for {csv_path.name}: {error_text}"
            ) from run_error
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
                stats_recorder.stats.run_id,
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
    failure_messages: list[str] = []
    try:
        while completed < total:
            message = await asyncio.to_thread(status_queue.get)
            if message is None:
                continue
            completed += 1
            stats_payload = message.get("stats")
            if isinstance(stats_payload, dict):
                stats_recorder.merge(stats_payload)
            if message.get("ok"):
                title = message.get("title") or message.get("journal_id") or "Unknown"
                print(f"  Finished {title}")
            else:
                title = message.get("title") or message.get("journal_id") or "Unknown"
                error = (
                    sanitize_error_sample(message.get("error") or "Unknown error")
                    or "Unknown error"
                )
                failure_messages.append(f"{title}: {error}")
                print(f"  - Journal worker failed: {title} ({error})")
    finally:
        request_queue.put({"type": "stop"})
        writer.join()
        for worker in workers:
            worker.join()

    for worker in workers:
        if worker.exitcode not in (0, None):
            failure_messages.append(
                f"worker {worker.pid or 'unknown'} exited with {worker.exitcode}"
            )

    error_summary = index_error_summary(failure_messages)
    stats_recorder.stats.finish(
        "failed" if failure_messages else "succeeded",
        error_summary,
    )
    await persist_final_index_stats(db_path, stats_recorder.stats)
    print_index_run_summary(stats_recorder.stats)

    async with aiosqlite.connect(db_path, timeout=DB_TIMEOUT_SECONDS) as db:
        if not failure_messages:
            await optimize_db(db)
        if not failure_messages and (
            not update or await is_article_listing_complete(db)
        ):
            await mark_listing_ready(db)
            await db.commit()

    if failure_messages:
        raise RuntimeError(f"Index run failed for {csv_path.name}: {error_summary}")


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
