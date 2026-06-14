"""Multiprocessing worker orchestration for indexing."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import aiosqlite

from paper_scanner.index.db.client import IPCDatabaseClient
from paper_scanner.index.db.retry import (
    commit_with_retry,
    execute_with_retry,
    executemany_with_retry,
)
from paper_scanner.index.db.schema import init_db
from paper_scanner.index.fetcher import process_journal
from paper_scanner.shared.constants import DB_TIMEOUT_SECONDS
from paper_scanner.shared.runtime_config import apply_runtime_config
from paper_scanner.sources.cnki import CnkiClient
from paper_scanner.sources.scholarly import ScholarlyClient


async def writer_main(
    db_path: str, request_queue: Any, response_queues: list[Any]
) -> None:
    """
    Run the single-writer database process loop.

    Args:
        db_path: SQLite database path.
        request_queue: Multiprocessing request queue.

    Returns:
        None.
    """
    async with aiosqlite.connect(db_path, timeout=DB_TIMEOUT_SECONDS) as db:
        await init_db(db)
        while True:
            message = await asyncio.to_thread(request_queue.get)
            if message is None:
                continue
            if message.get("type") == "stop":
                break
            worker_id = message.get("worker_id")
            if worker_id is None or not isinstance(worker_id, int):
                continue
            if worker_id < 0 or worker_id >= len(response_queues):
                continue
            response_queue = response_queues[worker_id]
            request_id = message.get("id")
            try:
                message_type = message.get("type")
                payload = message.get("payload") or {}
                result: Any = None
                if message_type == "execute":
                    sql = payload.get("sql")
                    if not sql:
                        raise RuntimeError("Missing SQL for execute request")
                    await execute_with_retry(db, sql, payload.get("params"))
                elif message_type == "executemany":
                    sql = payload.get("sql")
                    if not sql:
                        raise RuntimeError("Missing SQL for executemany request")
                    await executemany_with_retry(db, sql, payload.get("rows", []))
                elif message_type == "commit":
                    await commit_with_retry(db)
                elif message_type == "fetchall":
                    cursor = await db.execute(
                        payload.get("sql"), payload.get("params") or ()
                    )
                    result = await cursor.fetchall()
                    await cursor.close()
                elif message_type == "fetchone":
                    cursor = await db.execute(
                        payload.get("sql"), payload.get("params") or ()
                    )
                    result = await cursor.fetchone()
                    await cursor.close()
                else:
                    raise RuntimeError(f"Unknown IPC request type: {message_type}")
                await asyncio.to_thread(
                    response_queue.put,
                    {"id": request_id, "ok": True, "result": result},
                )
            except Exception as exc:
                await asyncio.to_thread(
                    response_queue.put,
                    {"id": request_id, "ok": False, "error": str(exc)},
                )


def writer_process(
    db_path: str, request_queue: Any, response_queues: list[Any]
) -> None:
    """
    Entry point for the writer process.

    Args:
        db_path: SQLite database path.
        request_queue: Multiprocessing request queue.

    Returns:
        None.
    """
    asyncio.run(writer_main(db_path, request_queue, response_queues))


def process_journal_worker_ipc(
    worker_id: int,
    request_queue: Any,
    response_queue: Any,
    csv_path: str,
    row: dict[str, str],
    issue_batch_size: int,
    thread_workers: int,
    timeout: int,
    show_year_progress: bool,
    resume: bool,
    update: bool,
    process_count: int = 1,
) -> tuple[str, str]:
    """
    Run journal processing in a worker process with IPC database access.

    Args:
        worker_id: Worker identifier for response routing.
        request_queue: Multiprocessing request queue.
        response_queue: Multiprocessing response queue.
        csv_path: Source CSV path.
        row: CSV row for the journal.
        issue_batch_size: Number of issues per fetch batch.
        thread_workers: Maximum concurrent HTTP requests.
        timeout: HTTP request timeout in seconds.
        show_year_progress: Whether to display year progress with tqdm.
        resume: Whether to resume from completed years and journals.
        update: Whether to perform incremental updates for existing years.
        process_count: Total process count sharing upstream limits.

    Returns:
        Tuple of journal ID and journal title.
    """

    async def run_worker() -> None:
        apply_runtime_config()
        scholarly_client = ScholarlyClient(
            timeout=timeout,
            worker_id=worker_id,
            process_count=process_count,
        )
        cnki_client = CnkiClient(timeout=timeout)
        db_client = IPCDatabaseClient(request_queue, response_queue, worker_id)
        try:
            await process_journal(
                db_client,
                scholarly_client,
                cnki_client,
                Path(csv_path),
                row,
                issue_batch_size,
                thread_workers,
                show_year_progress,
                resume,
                update,
            )
        finally:
            await scholarly_client.aclose()
            await cnki_client.aclose()

    asyncio.run(run_worker())
    journal_id = row.get("id") or ""
    title = row.get("title") or ""
    return journal_id, title


def run_worker_batch(
    worker_id: int,
    process_count: int,
    request_queue: Any,
    response_queue: Any,
    status_queue: Any,
    csv_path: str,
    rows: list[dict[str, str]],
    issue_batch_size: int,
    request_workers: int,
    timeout: int,
    resume: bool,
    update: bool,
) -> None:
    """
    Run a batch of journal rows in a worker process.

    Args:
        worker_id: Worker identifier for response routing.
        process_count: Total process count sharing upstream limits.
        request_queue: Multiprocessing request queue.
        response_queue: Multiprocessing response queue.
        status_queue: Multiprocessing status queue.
        csv_path: Source CSV path.
        rows: CSV rows for this worker.
        issue_batch_size: Number of issues per fetch batch.
        request_workers: Maximum concurrent HTTP requests.
        timeout: HTTP request timeout in seconds.
        resume: Whether to resume from completed years and journals.
        update: Whether to perform incremental updates for existing years.

    Returns:
        None.
    """

    async def run_batch() -> None:
        apply_runtime_config()
        scholarly_client = ScholarlyClient(
            timeout=timeout,
            worker_id=worker_id,
            process_count=process_count,
        )
        cnki_client = CnkiClient(timeout=timeout)
        db_client = IPCDatabaseClient(request_queue, response_queue, worker_id)
        try:
            for row in rows:
                try:
                    await process_journal(
                        db_client,
                        scholarly_client,
                        cnki_client,
                        Path(csv_path),
                        row,
                        issue_batch_size,
                        request_workers,
                        False,
                        resume,
                        update,
                    )
                    status_queue.put(
                        {
                            "ok": True,
                            "journal_id": row.get("id"),
                            "title": row.get("title"),
                        }
                    )
                except Exception as exc:
                    status_queue.put(
                        {
                            "ok": False,
                            "journal_id": row.get("id"),
                            "title": row.get("title"),
                            "error": str(exc),
                        }
                    )
        finally:
            await scholarly_client.aclose()
            await cnki_client.aclose()

    asyncio.run(run_batch())
