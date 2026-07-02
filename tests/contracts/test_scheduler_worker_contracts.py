"""Shadow contracts for the Rust scheduler worker runtime."""

from __future__ import annotations

import json
import os
import subprocess
import unittest
from pathlib import Path
from typing import Any

import paper_scanner.api.auth_db as auth_db

from .test_public_api_contracts import PROJECT_ROOT, temporary_auth_database


def run_ps_cli(project_root: Path, args: list[str]) -> Any:
    """
    Run the Rust CLI and decode its JSON stdout.

    Args:
        project_root: Temporary project root containing fixture data.
        args: CLI arguments after the binary name.

    Returns:
        Decoded JSON stdout.
    """
    env = os.environ.copy()
    env["PAPER_SCANNER_PROJECT_ROOT"] = str(project_root)
    result = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "ps-cli", "--", *args],
        cwd=PROJECT_ROOT,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def python_exit_command(exit_code: int) -> str:
    """
    Build a platform shell command that exits with a status code.

    Args:
        exit_code: Process exit code.

    Returns:
        Command string.
    """
    if os.name == "nt":
        return f"exit /B {exit_code}"
    return f"exit {exit_code}"


def python_env_check_command(key: str, value: str) -> str:
    """
    Build a platform shell command that validates an environment value.

    Args:
        key: Environment variable name.
        value: Expected environment variable value.

    Returns:
        Command string.
    """
    if os.name == "nt":
        return f'if "%{key}%"=="{value}" (exit /B 0) else (exit /B 9)'
    return f'test "${key}" = "{value}"'


class SchedulerWorkerContractTest(unittest.TestCase):
    """Verify Rust scheduler behavior against Python-authored state."""

    def test_scheduler_dry_run_loads_enabled_tasks_and_skips_invalid_cron(self) -> None:
        """
        Verify dry-run job loading and invalid cron handling.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            valid = auth_db.create_scheduled_task(
                "valid", "echo valid", "*/5 * * * *", enabled=True
            )
            disabled = auth_db.create_scheduled_task(
                "disabled", "echo disabled", "* * * * *", enabled=False
            )
            invalid = auth_db.create_scheduled_task(
                "invalid", "echo invalid", "not cron", enabled=True
            )

            payload = run_ps_cli(
                project_root,
                [
                    "scheduler",
                    "dry-run",
                    "--auth-db",
                    str(project_root / "data" / "auth.sqlite"),
                ],
            )

            self.assertEqual([item["id"] for item in payload["jobs"]], [valid["id"]])
            self.assertEqual(
                payload["jobs"][0]["job_id"], f"scheduled-task-{valid['id']}"
            )
            self.assertEqual(payload["jobs"][0]["max_instances"], 1)
            self.assertTrue(payload["jobs"][0]["coalesce"])
            self.assertEqual(
                [item["id"] for item in payload["skipped"]], [invalid["id"]]
            )
            all_ids = {item["id"] for item in payload["jobs"] + payload["skipped"]}
            self.assertNotIn(disabled["id"], all_ids)

    def test_scheduler_run_once_writes_python_compatible_status(self) -> None:
        """
        Verify shell execution status writeback.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            task = auth_db.create_scheduled_task(
                "failing",
                python_exit_command(7),
                "* * * * *",
                enabled=True,
            )

            outcome = run_ps_cli(
                project_root,
                [
                    "scheduler",
                    "run-once",
                    str(task["id"]),
                    "--auth-db",
                    str(project_root / "data" / "auth.sqlite"),
                ],
            )
            updated = auth_db.get_scheduled_task(int(task["id"]))

            self.assertEqual(
                outcome,
                {"found": True, "did_execute": True, "status": "failed (7)"},
            )
            assert updated is not None
            self.assertEqual(updated["last_status"], "failed (7)")
            self.assertIsNotNone(updated["last_run_at"])

    def test_scheduler_runtime_database_env_is_applied_to_shell_command(self) -> None:
        """
        Verify database runtime settings reach scheduled command environment.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            auth_db.upsert_runtime_settings(
                {"openalex_api_key_pool": "rust-worker-key"}
            )
            task = auth_db.create_scheduled_task(
                "env",
                python_env_check_command("OPENALEX_API_KEY_POOL", "rust-worker-key"),
                "* * * * *",
                enabled=True,
            )

            outcome = run_ps_cli(
                project_root,
                [
                    "scheduler",
                    "run-once",
                    str(task["id"]),
                    "--auth-db",
                    str(project_root / "data" / "auth.sqlite"),
                ],
            )
            updated = auth_db.get_scheduled_task(int(task["id"]))

            self.assertEqual(
                outcome,
                {"found": True, "did_execute": True, "status": "success"},
            )
            assert updated is not None
            self.assertEqual(updated["last_status"], "success")


if __name__ == "__main__":
    unittest.main()
