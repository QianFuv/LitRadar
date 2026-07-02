"""Shadow contracts for Rust notification and tracking delivery workers."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

import paper_scanner.api.auth_db as auth_db
from paper_scanner.notify.models import SelectionResult
from paper_scanner.notify.workflow import run_notification
from paper_scanner.push.workflow import run_push

from .contract_support import (
    CONTRACT_DB_NAME,
    assert_json_matches_fixture,
    isolated_contract_app,
    normalize_dynamic_values,
)
from .test_public_api_contracts import PROJECT_ROOT


class EmptySelector:
    """Fixture selector that lets Python exercise local fallback rules only."""

    def __init__(self, *args: object, **kwargs: object) -> None:
        """
        Accept the production selector constructor shape.

        Args:
            args: Ignored positional arguments.
            kwargs: Ignored keyword arguments.

        Returns:
            None.
        """

    def select_articles(self, *args: object, **kwargs: object) -> SelectionResult:
        """
        Return no model selections so local keyword fallback is used.

        Args:
            args: Ignored positional arguments.
            kwargs: Ignored keyword arguments.

        Returns:
            Empty selection result.
        """
        return SelectionResult(summary="", selections=[])

    def summarize_selected_articles(self, *args: object, **kwargs: object) -> str:
        """
        Return an empty final summary.

        Args:
            args: Ignored positional arguments.
            kwargs: Ignored keyword arguments.

        Returns:
            Empty summary.
        """
        return ""

    def close(self) -> None:
        """
        Close selector resources.

        Returns:
            None.
        """


def run_ps_cli(project_root: Path, args: list[str]) -> Any:
    """
    Run the Rust CLI and decode its JSON stdout.

    Args:
        project_root: Temporary project root.
        args: CLI arguments after the binary name.

    Returns:
        Decoded JSON stdout.
    """
    env = os.environ.copy()
    env["PAPER_SCANNER_PROJECT_ROOT"] = str(project_root)
    for key in (
        "NOTIFY_AI_API_KEY",
        "NOTIFY_AI_BASE_URL",
        "NOTIFY_AI_MODEL",
        "NOTIFY_AI_SYSTEM_PROMPT",
        "NOTIFY_MAX_CANDIDATES",
        "NOTIFY_TEMPERATURE",
    ):
        env.pop(key, None)
    result = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "ps-cli", "--", *args],
        cwd=PROJECT_ROOT,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def seed_subscriber(
    *,
    user_id: int,
    delivery_method: str,
    pushplus_token: str = "",
    sync_to_tracking_folder: bool = False,
    ai_api_key: str = "fixture-ai-key",
) -> dict[str, Any]:
    """
    Seed a tracking folder and enabled notification settings.

    Args:
        user_id: Target user identifier.
        delivery_method: Delivery method to seed.
        pushplus_token: PushPlus token value.
        sync_to_tracking_folder: Whether PushPlus also plans folder writes.
        ai_api_key: Per-user AI key value.

    Returns:
        Created tracking folder.
    """
    folder = auth_db.create_folder(user_id, "Tracking", is_tracking=True)
    auth_db.upsert_notification_settings(
        user_id,
        keywords=["contract"],
        directions=[],
        selected_databases=[CONTRACT_DB_NAME],
        delivery_method=delivery_method,
        pushplus_token=pushplus_token,
        sync_to_tracking_folder=sync_to_tracking_folder,
        ai_api_key=ai_api_key,
        ai_model="fixture-model" if ai_api_key else "",
    )
    return folder


def worker_args(state_dir: Path, workflow: str, project_root: Any) -> list[str]:
    """
    Build common Rust worker CLI arguments.

    Args:
        state_dir: Push state directory.
        workflow: `notify` or `push`.
        project_root: Contract app context.

    Returns:
        CLI argument list.
    """
    return [
        workflow,
        "dry-run",
        "--auth-db",
        str(project_root.auth_db_path),
        "--index-db",
        str(project_root.index_db_path),
        "--db",
        CONTRACT_DB_NAME,
        "--state-dir",
        str(state_dir),
    ]


def workflow_args(state_dir: Path) -> argparse.Namespace:
    """
    Build common Python workflow args.

    Args:
        state_dir: Push state directory.

    Returns:
        Namespace matching Python workflow CLI args.
    """
    return argparse.Namespace(
        db=CONTRACT_DB_NAME,
        changes_file="",
        state_dir=str(state_dir),
        ai_model="",
        max_candidates=None,
        timeout=1.0,
        retries=0,
        dedupe_retention_days=30,
        dry_run=True,
    )


class NotificationWorkerContractTest(unittest.TestCase):
    """Compare Rust delivery workers against Python dry-run contracts."""

    def test_tracking_push_dry_run_matches_python_state_without_writes(self) -> None:
        """
        Verify folder delivery state parity and dry-run write suppression.

        Returns:
            None.
        """
        with isolated_contract_app() as python_contract:
            folder = seed_subscriber(
                user_id=int(python_contract.user["id"]),
                delivery_method="folder",
            )
            python_state_dir = python_contract.root_path / "python_push_state"
            with patch(
                "paper_scanner.notify.selection.OpenAICompatibleSelector",
                EmptySelector,
            ):
                exit_code = run_push(workflow_args(python_state_dir))
            self.assertEqual(exit_code, 0)
            with open(python_state_dir / "contract.json", encoding="utf-8") as handle:
                expected_state = normalize_dynamic_values(json.load(handle))
            self.assertEqual(
                auth_db.count_favorites(
                    int(python_contract.user["id"]),
                    int(folder["id"]),
                ),
                0,
            )

        with isolated_contract_app() as rust_contract:
            folder = seed_subscriber(
                user_id=int(rust_contract.user["id"]),
                delivery_method="folder",
            )
            rust_state_dir = rust_contract.root_path / "rust_push_state"
            payload = run_ps_cli(
                rust_contract.root_path,
                worker_args(rust_state_dir, "push", rust_contract),
            )
            with open(rust_state_dir / "contract.json", encoding="utf-8") as handle:
                actual_state = normalize_dynamic_values(json.load(handle))

            self.assertEqual(payload["status"], "completed")
            self.assertEqual(
                payload["subscribers"][0]["selected_article_ids"],
                [9007199254740997, 9007199254740996, 9007199254740995],
            )
            self.assertEqual(
                payload["subscribers"][0]["favorite_writes"][0]["folder_id"],
                folder["id"],
            )
            self.assertEqual(
                auth_db.count_favorites(
                    int(rust_contract.user["id"]),
                    int(folder["id"]),
                ),
                0,
            )
            assert_json_matches_fixture(self, actual_state, expected_state)

    def test_pushplus_notify_dry_run_matches_python_state_without_network(self) -> None:
        """
        Verify PushPlus dry-run state parity and planned tracking sync.

        Returns:
            None.
        """
        with isolated_contract_app() as python_contract:
            seed_subscriber(
                user_id=int(python_contract.user["id"]),
                delivery_method="pushplus",
                pushplus_token="fixture-token",
                sync_to_tracking_folder=True,
            )
            python_state_dir = python_contract.root_path / "python_notify_state"
            with patch(
                "paper_scanner.notify.selection.OpenAICompatibleSelector",
                EmptySelector,
            ):
                exit_code = run_notification(workflow_args(python_state_dir))
            self.assertEqual(exit_code, 0)
            with open(python_state_dir / "contract.json", encoding="utf-8") as handle:
                expected_state = normalize_dynamic_values(json.load(handle))

        with isolated_contract_app() as rust_contract:
            seed_subscriber(
                user_id=int(rust_contract.user["id"]),
                delivery_method="pushplus",
                pushplus_token="fixture-token",
                sync_to_tracking_folder=True,
            )
            rust_state_dir = rust_contract.root_path / "rust_notify_state"
            payload = run_ps_cli(
                rust_contract.root_path,
                worker_args(rust_state_dir, "notify", rust_contract),
            )
            with open(rust_state_dir / "contract.json", encoding="utf-8") as handle:
                actual_state = normalize_dynamic_values(json.load(handle))

            self.assertEqual(payload["status"], "completed")
            self.assertTrue(payload["subscribers"][0]["would_send_pushplus"])
            self.assertIn(
                "Paper Scanner Weekly Update [contract.sqlite]",
                payload["subscribers"][0]["message_title"],
            )
            self.assertEqual(len(payload["subscribers"][0]["favorite_writes"]), 3)
            assert_json_matches_fixture(self, actual_state, expected_state)

    def test_no_ai_config_skips_like_python_contract(self) -> None:
        """
        Verify unavailable AI configuration records a skipped subscriber result.

        Returns:
            None.
        """
        with isolated_contract_app() as rust_contract:
            seed_subscriber(
                user_id=int(rust_contract.user["id"]),
                delivery_method="folder",
                ai_api_key="",
            )
            state_dir = rust_contract.root_path / "no_config_state"
            payload = run_ps_cli(
                rust_contract.root_path,
                worker_args(state_dir, "push", rust_contract),
            )

            self.assertEqual(payload["status"], "completed")
            self.assertEqual(payload["subscribers"][0]["status"], "skipped")
            self.assertEqual(
                payload["subscribers"][0]["error"],
                "AI configuration is unavailable",
            )


if __name__ == "__main__":
    unittest.main()
