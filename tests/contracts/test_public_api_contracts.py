"""Shadow contracts for public Rust API endpoints."""

from __future__ import annotations

import json
import os
import socket
import sqlite3
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.routes import register_routes

PROJECT_ROOT = Path(__file__).resolve().parents[2]


@contextmanager
def temporary_auth_database() -> Iterator[Path]:
    """
    Create an auth database fixture under a temporary project root.

    Yields:
        Temporary project root containing `data/auth.sqlite`.
    """
    previous_auth_db_path = auth_db.AUTH_DB_PATH
    temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
    project_root = Path(temp_dir.name)
    auth_db.AUTH_DB_PATH = project_root / "data" / "auth.sqlite"
    try:
        auth_db.init_auth_db()
        seed_announcements(auth_db.AUTH_DB_PATH)
        yield project_root
    finally:
        auth_db.AUTH_DB_PATH = previous_auth_db_path
        temp_dir.cleanup()


def seed_announcements(auth_db_path: Path) -> None:
    """
    Insert deterministic announcement rows for Python and Rust comparison.

    Args:
        auth_db_path: Path to the auth SQLite database.

    Returns:
        None.
    """
    with sqlite3.connect(auth_db_path) as connection:
        connection.executemany(
            "INSERT INTO announcements "
            "(title, message, priority, enabled, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            [
                ("Normal newer", "normal message", "normal", 1, 20.0, 21.0),
                ("High older", "high message", "high", 1, 10.0, 11.0),
                ("Low newest", "low message", "low", 1, 30.0, 31.0),
                ("Disabled", "hidden message", "high", 0, 40.0, 41.0),
            ],
        )
        connection.commit()


def available_port() -> int:
    """
    Reserve and release an available localhost port.

    Returns:
        Port number that was available at selection time.
    """
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def rust_api_command() -> list[str]:
    """
    Return the fastest available Rust API launch command.

    Returns:
        Command that starts the Rust API server.
    """
    executable = (
        PROJECT_ROOT
        / "target"
        / "debug"
        / ("ps-api.exe" if os.name == "nt" else "ps-api")
    )
    if executable.exists():
        return [str(executable)]
    return ["cargo", "run", "--quiet", "-p", "ps-api"]


def start_rust_api(project_root: Path, port: int) -> subprocess.Popen[str]:
    """
    Start the Rust API server for a shadow contract test.

    Args:
        project_root: Temporary project root containing fixture data.
        port: TCP port for the Rust API server.

    Returns:
        Running Rust API process.
    """
    env = os.environ.copy()
    env["PAPER_SCANNER_PROJECT_ROOT"] = str(project_root)
    env["API_HOST"] = "127.0.0.1"
    env["API_PORT"] = str(port)
    env["API_CORS_ALLOWED_ORIGINS"] = ""
    creation_flags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
    process = subprocess.Popen(
        rust_api_command(),
        cwd=PROJECT_ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        creationflags=creation_flags,
    )
    wait_for_rust_api(process, port)
    return process


def wait_for_rust_api(process: subprocess.Popen[str], port: int) -> None:
    """
    Wait until the Rust API server accepts health requests.

    Args:
        process: Running Rust API process.
        port: TCP port for the Rust API server.

    Returns:
        None.
    """
    deadline = time.monotonic() + 30.0
    url = f"http://127.0.0.1:{port}/api/health"
    last_error: BaseException | None = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            stdout = process.stdout.read() if process.stdout is not None else ""
            stderr = process.stderr.read() if process.stderr is not None else ""
            raise RuntimeError(
                f"Rust API exited early with code {process.returncode}\n"
                f"stdout:\n{stdout}\nstderr:\n{stderr}"
            )
        try:
            status, payload = get_json(url)
        except (OSError, urllib.error.URLError) as error:
            last_error = error
            time.sleep(0.25)
            continue
        if status == 200 and payload == {"status": "ok"}:
            return
        time.sleep(0.25)
    raise TimeoutError(f"Rust API did not start on port {port}: {last_error}")


def stop_process(process: subprocess.Popen[str]) -> None:
    """
    Stop a subprocess used by a contract test.

    Args:
        process: Process to terminate.

    Returns:
        None.
    """
    try:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10.0)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10.0)
    finally:
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()


def get_json(url: str) -> tuple[int, Any]:
    """
    Fetch a JSON response from an HTTP URL.

    Args:
        url: URL to fetch.

    Returns:
        HTTP status code and decoded JSON body.
    """
    request = urllib.request.Request(url, method="GET")
    with urllib.request.urlopen(request, timeout=2.0) as response:
        body = response.read()
        return int(response.status), json.loads(body.decode("utf-8"))


def get_status(url: str) -> int:
    """
    Fetch only the HTTP status code from a URL.

    Args:
        url: URL to fetch.

    Returns:
        HTTP status code.
    """
    request = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(request, timeout=2.0) as response:
            return int(response.status)
    except urllib.error.HTTPError as error:
        return int(error.code)


class PublicApiContractTest(unittest.TestCase):
    """Compare migrated public Rust endpoints with Python behavior."""

    def test_health_and_announcements_match_python_shadow(self) -> None:
        """
        Verify Rust public endpoint responses against Python on one auth DB.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            app = FastAPI()
            register_routes(app)
            client = TestClient(app)
            port = available_port()
            process = start_rust_api(project_root, port)
            try:
                python_health = client.get("/api/health")
                rust_health_status, rust_health_payload = get_json(
                    f"http://127.0.0.1:{port}/api/health"
                )
                python_announcements = client.get("/api/announcements")
                rust_announcement_status, rust_announcement_payload = get_json(
                    f"http://127.0.0.1:{port}/api/announcements"
                )
                protected_status = get_status(f"http://127.0.0.1:{port}/api/auth/me")

                self.assertEqual(rust_health_status, python_health.status_code)
                self.assertEqual(rust_health_payload, python_health.json())
                self.assertEqual(
                    rust_announcement_status,
                    python_announcements.status_code,
                )
                self.assertEqual(rust_announcement_payload, python_announcements.json())
                self.assertEqual(
                    [item["title"] for item in rust_announcement_payload],
                    ["High older", "Normal newer", "Low newest"],
                )
                self.assertTrue(
                    all(item["enabled"] is True for item in rust_announcement_payload)
                )
                self.assertEqual(protected_status, 404)
            finally:
                stop_process(process)
                client.close()


if __name__ == "__main__":
    unittest.main()
