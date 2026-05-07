"""State persistence utilities."""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


def utc_now_iso() -> str:
    """
    Build current UTC ISO-8601 timestamp.

    Args:
        None.

    Returns:
        Timestamp string.
    """
    return datetime.now(UTC).replace(microsecond=0).isoformat()


def build_issue_key(journal_id: int, issue_id: int) -> str:
    """
    Build issue key string.

    Args:
        journal_id: Journal identifier.
        issue_id: Issue identifier.

    Returns:
        Serialized issue key.
    """
    return f"{journal_id}:{issue_id}"


def parse_issue_key(key: str) -> tuple[int, int]:
    """
    Parse serialized issue key.

    Args:
        key: Issue key string.

    Returns:
        Journal and issue ids.
    """
    parts = key.split(":", maxsplit=1)
    if len(parts) != 2:
        raise ValueError(f"Invalid issue key: {key}")
    journal_id = int(parts[0])
    issue_id = int(parts[1])
    return journal_id, issue_id


def load_json(path: Path, default: Any) -> Any:
    """
    Load JSON payload from disk.

    Args:
        path: Source path.
        default: Default value when file is missing.

    Returns:
        Loaded payload.
    """
    if not path.exists():
        return default
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def save_json_atomic(path: Path, payload: Any) -> None:
    """
    Save JSON payload atomically.

    Args:
        path: Output path.
        payload: Payload object.

    Returns:
        None.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = path.with_name(f"{path.name}.tmp")
    with open(temp_path, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
    temp_path.replace(path)


def build_default_state(db_name: str) -> dict[str, Any]:
    """
    Build default state payload.

    Args:
        db_name: Database filename.

    Returns:
        Initialized state dictionary.
    """
    now = utc_now_iso()
    return {
        "db_name": db_name,
        "status": "idle",
        "last_completed_run_at": None,
        "snapshot": {
            "issue_article_counts": {},
            "inpress_article_counts": {},
        },
        "run": None,
        "delivery_dedupe": {},
        "updated_at": now,
    }


def load_state(path: Path, db_name: str) -> dict[str, Any]:
    """
    Load and normalize persisted state.

    Args:
        path: State path.
        db_name: Database filename.

    Returns:
        Normalized state dictionary.
    """
    state = load_json(path, build_default_state(db_name))
    if not isinstance(state, dict):
        raise ValueError("State file must be a JSON object")

    if state.get("db_name") not in {None, db_name}:
        raise ValueError("State file does not match selected database")

    state["db_name"] = db_name
    state.setdefault("status", "idle")
    state.setdefault("last_completed_run_at", None)
    snapshot = state.get("snapshot")
    if not isinstance(snapshot, dict):
        snapshot = {}
    snapshot.setdefault("issue_article_counts", {})
    snapshot.setdefault("inpress_article_counts", {})
    if not isinstance(snapshot["issue_article_counts"], dict):
        snapshot["issue_article_counts"] = {}
    if not isinstance(snapshot["inpress_article_counts"], dict):
        snapshot["inpress_article_counts"] = {}
    state["snapshot"] = snapshot

    delivery_dedupe = state.get("delivery_dedupe")
    if not isinstance(delivery_dedupe, dict):
        delivery_dedupe = {}
    state["delivery_dedupe"] = delivery_dedupe
    run = state.get("run")
    if run is not None and not isinstance(run, dict):
        state["run"] = None
    elif isinstance(run, dict):
        delivered_article_ids = run.get("delivered_article_ids")
        if not isinstance(delivered_article_ids, list):
            run["delivered_article_ids"] = []
    state.setdefault("updated_at", utc_now_iso())
    return state


def create_run_state(
    run_id: str,
    pending_issue_keys: list[str],
    pending_inpress_keys: list[str],
) -> dict[str, Any]:
    """
    Build run payload for current execution.

    Args:
        run_id: Stable run id.
        pending_issue_keys: Pending issue keys.
        pending_inpress_keys: Pending in-press keys.

    Returns:
        Run dictionary.
    """
    now = utc_now_iso()
    return {
        "run_id": run_id,
        "status": "running",
        "started_at": now,
        "completed_at": None,
        "updated_at": now,
        "pending_issue_keys": pending_issue_keys,
        "done_issue_keys": [],
        "pending_inpress_keys": pending_inpress_keys,
        "done_inpress_keys": [],
        "delivered_article_ids": [],
        "errors": [],
        "user_results": [],
    }
