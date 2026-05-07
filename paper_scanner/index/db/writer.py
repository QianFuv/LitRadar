"""Serialized database write worker."""

from __future__ import annotations

import asyncio
from typing import Any

import aiosqlite

from paper_scanner.index.db.retry import (
    commit_with_retry,
    execute_with_retry,
    executemany_with_retry,
)


class DatabaseWriter:
    """
    Serialize database writes through a single async worker.
    """

    def __init__(self, db: aiosqlite.Connection) -> None:
        """
        Initialize the writer with an open database connection.

        Args:
            db: Open aiosqlite connection.
        """
        self._db = db
        self._queue: asyncio.Queue[
            tuple[str, Any, Any, asyncio.Future[None]] | None
        ] = asyncio.Queue()
        self._task: asyncio.Task[None] | None = None

    async def start(self) -> None:
        """
        Start the background writer task.

        Returns:
            None.
        """
        if self._task is None:
            self._task = asyncio.create_task(self._run())

    async def close(self) -> None:
        """
        Stop the background writer task after pending work completes.

        Returns:
            None.
        """
        if self._task is None:
            return
        await self._queue.put(None)
        await self._task
        self._task = None

    async def execute(self, sql: str, params: tuple[Any, ...] | None = None) -> None:
        """
        Enqueue a SQL statement for execution.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            None.
        """
        loop = asyncio.get_running_loop()
        future: asyncio.Future[None] = loop.create_future()
        await self._queue.put(("execute", sql, params, future))
        await future

    async def executemany(self, sql: str, rows: list[tuple[Any, ...]]) -> None:
        """
        Enqueue a SQL executemany operation.

        Args:
            sql: SQL statement to execute.
            rows: SQL parameter rows.

        Returns:
            None.
        """
        loop = asyncio.get_running_loop()
        future: asyncio.Future[None] = loop.create_future()
        await self._queue.put(("executemany", sql, rows, future))
        await future

    async def commit(self) -> None:
        """
        Enqueue a commit operation.

        Returns:
            None.
        """
        loop = asyncio.get_running_loop()
        future: asyncio.Future[None] = loop.create_future()
        await self._queue.put(("commit", None, None, future))
        await future

    async def _run(self) -> None:
        """
        Execute queued write operations sequentially.

        Returns:
            None.
        """
        while True:
            item = await self._queue.get()
            if item is None:
                break
            kind, sql, payload, future = item
            try:
                if kind == "execute":
                    await execute_with_retry(self._db, sql, payload)
                elif kind == "executemany":
                    await executemany_with_retry(self._db, sql, payload)
                elif kind == "commit":
                    await commit_with_retry(self._db)
                if not future.done():
                    future.set_result(None)
            except Exception as exc:
                if not future.done():
                    future.set_exception(exc)
