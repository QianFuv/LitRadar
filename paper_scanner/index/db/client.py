"""Database client abstractions for local and IPC modes."""

from __future__ import annotations

import asyncio
import time
from typing import Any, Protocol

import aiosqlite

from paper_scanner.index.db.writer import DatabaseWriter


class DatabaseClient(Protocol):
    """
    Database client protocol for read and write operations.
    """

    async def execute(self, sql: str, params: tuple[Any, ...] | None = None) -> None:
        """
        Execute a SQL statement.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            None.
        """

    async def executemany(self, sql: str, rows: list[tuple[Any, ...]]) -> None:
        """
        Execute many SQL statements.

        Args:
            sql: SQL statement to execute.
            rows: SQL parameter rows.

        Returns:
            None.
        """

    async def commit(self) -> None:
        """
        Commit a transaction.

        Returns:
            None.
        """

    async def fetchall(
        self, sql: str, params: tuple[Any, ...] | None = None
    ) -> list[tuple[Any, ...]]:
        """
        Fetch all rows for a query.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            Query result rows.
        """

    async def fetchone(
        self, sql: str, params: tuple[Any, ...] | None = None
    ) -> tuple[Any, ...] | None:
        """
        Fetch a single row for a query.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            Single row or None.
        """


class LocalDatabaseClient:
    """
    Local database client that uses a writer for serialized writes.
    """

    def __init__(self, db: aiosqlite.Connection) -> None:
        """
        Initialize the client.

        Args:
            db: Open aiosqlite connection.
        """
        self._db = db
        self._writer = DatabaseWriter(db)

    async def start(self) -> None:
        """
        Start the write worker.

        Returns:
            None.
        """
        await self._writer.start()

    async def close(self) -> None:
        """
        Stop the write worker.

        Returns:
            None.
        """
        await self._writer.close()

    async def execute(self, sql: str, params: tuple[Any, ...] | None = None) -> None:
        """
        Execute a SQL statement.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            None.
        """
        await self._writer.execute(sql, params)

    async def executemany(self, sql: str, rows: list[tuple[Any, ...]]) -> None:
        """
        Execute many SQL statements.

        Args:
            sql: SQL statement to execute.
            rows: SQL parameter rows.

        Returns:
            None.
        """
        await self._writer.executemany(sql, rows)

    async def commit(self) -> None:
        """
        Commit a transaction.

        Returns:
            None.
        """
        await self._writer.commit()

    async def fetchall(
        self, sql: str, params: tuple[Any, ...] | None = None
    ) -> list[tuple[Any, ...]]:
        """
        Fetch all rows for a query.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            Query result rows.
        """
        cursor = await self._db.execute(sql, params or ())
        rows = await cursor.fetchall()
        await cursor.close()
        return [tuple(row) for row in rows]

    async def fetchone(
        self, sql: str, params: tuple[Any, ...] | None = None
    ) -> tuple[Any, ...] | None:
        """
        Fetch a single row for a query.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            Single row or None.
        """
        cursor = await self._db.execute(sql, params or ())
        row = await cursor.fetchone()
        await cursor.close()
        if row is None:
            return None
        return tuple(row)


class IPCDatabaseClient:
    """
    IPC database client for single-writer multiprocessing.
    """

    def __init__(self, request_queue: Any, response_queue: Any, worker_id: int) -> None:
        """
        Initialize the IPC client.

        Args:
            request_queue: Multiprocessing request queue.
            response_queue: Multiprocessing response queue.
            worker_id: Worker identifier for response routing.
        """
        self._request_queue = request_queue
        self._response_queue = response_queue
        self._worker_id = worker_id

    async def execute(self, sql: str, params: tuple[Any, ...] | None = None) -> None:
        """
        Execute a SQL statement.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            None.
        """
        await self._send_request("execute", {"sql": sql, "params": params})

    async def executemany(self, sql: str, rows: list[tuple[Any, ...]]) -> None:
        """
        Execute many SQL statements.

        Args:
            sql: SQL statement to execute.
            rows: SQL parameter rows.

        Returns:
            None.
        """
        await self._send_request("executemany", {"sql": sql, "rows": rows})

    async def commit(self) -> None:
        """
        Commit a transaction.

        Returns:
            None.
        """
        await self._send_request("commit", {})

    async def fetchall(
        self, sql: str, params: tuple[Any, ...] | None = None
    ) -> list[tuple[Any, ...]]:
        """
        Fetch all rows for a query.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            Query result rows.
        """
        return await self._send_request("fetchall", {"sql": sql, "params": params})

    async def fetchone(
        self, sql: str, params: tuple[Any, ...] | None = None
    ) -> tuple[Any, ...] | None:
        """
        Fetch a single row for a query.

        Args:
            sql: SQL statement to execute.
            params: SQL parameters.

        Returns:
            Single row or None.
        """
        return await self._send_request("fetchone", {"sql": sql, "params": params})

    async def _send_request(self, kind: str, payload: dict[str, Any]) -> Any:
        """
        Send a request to the writer process and await the response.

        Args:
            kind: Request type.
            payload: Request payload.

        Returns:
            Response payload.
        """
        request_id = f"{time.time_ns()}"
        message = {
            "id": request_id,
            "type": kind,
            "payload": payload,
            "worker_id": self._worker_id,
        }
        await asyncio.to_thread(self._request_queue.put, message)
        response = await asyncio.to_thread(self._response_queue.get)
        if response.get("id") != request_id:
            raise RuntimeError("Mismatched IPC response id")
        if not response.get("ok"):
            raise RuntimeError(response.get("error") or "IPC database error")
        return response.get("result")
