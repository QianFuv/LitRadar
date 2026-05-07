"""Notification workflow orchestration."""

from __future__ import annotations

import argparse
import sqlite3
from pathlib import Path

from paper_scanner.api.auth_db import bulk_add_favorites
from paper_scanner.notify.ai_selector import OpenAICompatibleSelector
from paper_scanner.notify.candidates import (
    deduplicate_candidates,
    fetch_candidates_for_inpress_keys,
    fetch_candidates_for_issue_keys,
)
from paper_scanner.notify.changes import (
    collect_inpress_article_counts,
    collect_issue_article_counts,
    compute_changed_inpress_keys,
    compute_changed_issue_keys,
)
from paper_scanner.notify.delivery import (
    load_change_manifest,
    prune_delivery_dedupe,
    resolve_path,
)
from paper_scanner.notify.message import build_markdown_content, build_message_title
from paper_scanner.notify.models import MAX_AI_SELECTION_ROUNDS
from paper_scanner.notify.pushplus import PushPlusClient
from paper_scanner.notify.selection import select_articles_for_subscriber
from paper_scanner.notify.state import (
    create_run_state,
    load_json,
    load_state,
    save_json_atomic,
    utc_now_iso,
)
from paper_scanner.notify.subscriptions import (
    load_notification_config,
    load_subscribers_from_db,
)
from paper_scanner.shared.constants import PROJECT_ROOT
from paper_scanner.shared.converters import to_int
from paper_scanner.shared.db_path import (
    is_database_selected,
    list_database_files,
    resolve_db_path,
)


def _resolve_target_db_paths(args: argparse.Namespace) -> list[Path]:
    """
    Resolve one or more target databases for a notification run.

    Args:
        args: Parsed CLI arguments.

    Returns:
        Database paths to process.
    """
    db_name_value = str(args.db or "").strip()
    if db_name_value:
        try:
            return [resolve_db_path(db_name_value)]
        except ValueError as exc:
            raise SystemExit(str(exc)) from exc

    changes_file_value = str(getattr(args, "changes_file", "") or "").strip()
    if changes_file_value:
        changes_file = resolve_path(changes_file_value, PROJECT_ROOT)
        payload = load_json(changes_file, None)
        if not isinstance(payload, dict):
            raise SystemExit(f"Invalid change manifest file: {changes_file}")
        manifest_db = str(payload.get("db_name") or "").strip()
        if not manifest_db:
            raise SystemExit("Change manifest missing db_name; specify --db explicitly")
        try:
            return [resolve_db_path(manifest_db)]
        except ValueError as exc:
            raise SystemExit(str(exc)) from exc

    db_paths = list_database_files()
    if not db_paths:
        raise SystemExit("No SQLite databases found")
    return db_paths


def _run_notification_for_db(
    args: argparse.Namespace,
    db_path: Path,
) -> int:
    """
    Execute notification pipeline for one database.

    Args:
        args: Parsed CLI arguments.
        db_path: Database path to process.

    Returns:
        Process exit code.
    """
    state_dir = resolve_path(args.state_dir, PROJECT_ROOT)
    state_file = state_dir / f"{db_path.stem}.json"
    changes_file_value = str(getattr(args, "changes_file", "") or "").strip()
    changes_file = (
        resolve_path(changes_file_value, PROJECT_ROOT) if changes_file_value else None
    )

    with sqlite3.connect(db_path) as connection:
        connection.row_factory = sqlite3.Row
        current_issue_counts = collect_issue_article_counts(connection)
        current_inpress_counts = collect_inpress_article_counts(connection)

        state = load_state(state_file, db_path.name)
        manifest_run_id: str | None = None
        if changes_file is not None:
            (
                pending_issue_keys,
                pending_inpress_keys,
                pending_article_ids,
                manifest_run_id,
            ) = load_change_manifest(changes_file, db_path.name)
        else:
            pending_article_ids = []
            previous_issue_counts = {
                key: int(value)
                for key, value in state["snapshot"]["issue_article_counts"].items()
                if to_int(value) is not None
            }
            previous_inpress_counts = {
                key: int(value)
                for key, value in state["snapshot"]["inpress_article_counts"].items()
                if to_int(value) is not None
            }

            pending_issue_keys = compute_changed_issue_keys(
                previous_issue_counts,
                current_issue_counts,
            )
            pending_inpress_keys = compute_changed_inpress_keys(
                previous_inpress_counts,
                current_inpress_counts,
            )

        if not pending_issue_keys and not pending_inpress_keys:
            state["status"] = "idle"
            state["run"] = None
            state["updated_at"] = utc_now_iso()
            save_json_atomic(state_file, state)
            print("No updated issues or in-press entries to notify.")
            return 0

        run_id = manifest_run_id or utc_now_iso()
        run_state = create_run_state(run_id, pending_issue_keys, pending_inpress_keys)
        state["status"] = "running"
        state["run"] = run_state
        state["updated_at"] = utc_now_iso()
        save_json_atomic(state_file, state)

        issue_candidates = fetch_candidates_for_issue_keys(
            connection,
            pending_issue_keys,
        )
        inpress_candidates = fetch_candidates_for_inpress_keys(
            connection,
            pending_inpress_keys,
        )
        all_candidates = deduplicate_candidates(issue_candidates + inpress_candidates)
        if changes_file is not None:
            pending_article_id_set = set(pending_article_ids)
            all_candidates = [
                item
                for item in all_candidates
                if item.article_id in pending_article_id_set
            ]

        if not all_candidates:
            run_state["status"] = "completed"
            run_state["completed_at"] = utc_now_iso()
            run_state["updated_at"] = utc_now_iso()
            run_state["done_issue_keys"] = pending_issue_keys
            run_state["done_inpress_keys"] = pending_inpress_keys
            run_state["pending_issue_keys"] = []
            run_state["pending_inpress_keys"] = []
            state["snapshot"] = {
                "issue_article_counts": current_issue_counts,
                "inpress_article_counts": current_inpress_counts,
            }
            state["status"] = "completed"
            state["last_completed_run_at"] = utc_now_iso()
            state["updated_at"] = utc_now_iso()
            save_json_atomic(state_file, state)
            print("No visible article candidates found for pending issues.")
            return 0

        run_state["delivered_article_ids"] = [
            item.article_id for item in all_candidates
        ]
        run_state["updated_at"] = utc_now_iso()
        state["updated_at"] = utc_now_iso()
        save_json_atomic(state_file, state)

        global_config, defaults = load_notification_config()
        subscribers = load_subscribers_from_db()
        pushplus_subscribers = [
            sub
            for sub in subscribers
            if sub.delivery_method == "pushplus"
            and is_database_selected(sub.selected_databases, db_path.name)
        ]

        if not pushplus_subscribers:
            run_state["status"] = "skipped"
            run_state["updated_at"] = utc_now_iso()
            state["status"] = "skipped"
            state["updated_at"] = utc_now_iso()
            save_json_atomic(state_file, state)
            print("No PushPlus subscribers found — skipping run.")
            return 0

        model_override = str(args.ai_model or "").strip() or None
        max_candidates = args.max_candidates or defaults.max_candidates
        max_candidates = max(1, max_candidates)
        candidates_for_model = all_candidates[:max_candidates]
        candidates_by_id = {item.article_id: item for item in all_candidates}
        selector_cache: dict[
            tuple[str, str, str, str, int],
            OpenAICompatibleSelector,
        ] = {}
        push_client = PushPlusClient(timeout_seconds=args.timeout, retries=args.retries)

        delivery_dedupe = state.get("delivery_dedupe")
        if not isinstance(delivery_dedupe, dict):
            delivery_dedupe = {}
        state["delivery_dedupe"] = delivery_dedupe

        errors: list[str] = []

        try:
            for subscriber in pushplus_subscribers:
                try:
                    final_summary = ""
                    accepted, final_summary, skip_reason = (
                        select_articles_for_subscriber(
                            subscriber=subscriber,
                            global_config=global_config,
                            defaults=defaults,
                            candidates_for_model=candidates_for_model,
                            candidates_by_id=candidates_by_id,
                            delivery_dedupe=delivery_dedupe,
                            selector_cache=selector_cache,
                            timeout_seconds=args.timeout,
                            retry_attempts=max(
                                args.retries, subscriber.ai_retry_attempts
                            ),
                            override_model=model_override,
                            max_rounds=MAX_AI_SELECTION_ROUNDS,
                        )
                    )
                    if skip_reason is not None:
                        run_state["user_results"].append(
                            {
                                "subscriber_id": subscriber.subscriber_id,
                                "selected_count": 0,
                                "pushed_count": 0,
                                "message_id": None,
                                "status": "skipped",
                                "error": skip_reason,
                            }
                        )
                        run_state["updated_at"] = utc_now_iso()
                        state["updated_at"] = utc_now_iso()
                        save_json_atomic(state_file, state)
                        continue

                    if not accepted:
                        run_state["user_results"].append(
                            {
                                "subscriber_id": subscriber.subscriber_id,
                                "selected_count": 0,
                                "pushed_count": 0,
                                "message_id": None,
                                "status": "skipped",
                                "error": "AI selection found no matching articles",
                            }
                        )
                        run_state["updated_at"] = utc_now_iso()
                        state["updated_at"] = utc_now_iso()
                        save_json_atomic(state_file, state)
                        continue

                    message_title = build_message_title(db_path.name, run_id)
                    content = build_markdown_content(
                        db_path.name,
                        run_id,
                        subscriber,
                        final_summary,
                        accepted,
                        candidates_by_id,
                    )

                    if args.dry_run:
                        synced_count = (
                            len(accepted) if subscriber.sync_to_tracking_folder else 0
                        )
                        print(
                            "DRY RUN",
                            subscriber.subscriber_id,
                            f"selected={len(accepted)}",
                            f"synced={synced_count}",
                        )
                        message_id = ""
                    else:
                        synced_count = 0
                        if subscriber.sync_to_tracking_folder:
                            if subscriber.tracking_folder_id is None:
                                raise RuntimeError("Tracking folder is not configured")
                            folder_articles = [
                                {
                                    "article_id": item.article_id,
                                    "db_name": db_path.name,
                                }
                                for item in accepted
                                if item.article_id in candidates_by_id
                            ]
                            synced_count = bulk_add_favorites(
                                int(subscriber.subscriber_id),
                                subscriber.tracking_folder_id,
                                folder_articles,
                            )
                        message_id = push_client.send(
                            token=subscriber.pushplus_token,
                            title=message_title,
                            content=content,
                            channel=subscriber.channel
                            or global_config.pushplus_channel,
                            template=subscriber.template
                            or global_config.pushplus_template,
                            topic=subscriber.topic or global_config.pushplus_topic,
                            option=global_config.pushplus_option,
                            to=None,
                        )
                        sent_at = utc_now_iso()
                        for item in accepted:
                            delivery_key = (
                                f"{subscriber.subscriber_id}:{item.article_id}"
                            )
                            delivery_dedupe[delivery_key] = sent_at

                    run_state["user_results"].append(
                        {
                            "subscriber_id": subscriber.subscriber_id,
                            "selected_count": len(accepted),
                            "pushed_count": len(accepted),
                            "folder_synced_count": synced_count,
                            "message_id": message_id or None,
                            "status": "ok",
                            "error": None,
                        }
                    )
                except Exception as error:
                    error_message = f"{subscriber.subscriber_id}: {error}"
                    errors.append(error_message)
                    run_state["user_results"].append(
                        {
                            "subscriber_id": subscriber.subscriber_id,
                            "selected_count": 0,
                            "pushed_count": 0,
                            "folder_synced_count": 0,
                            "message_id": None,
                            "status": "error",
                            "error": str(error),
                        }
                    )
                finally:
                    run_state["updated_at"] = utc_now_iso()
                    state["updated_at"] = utc_now_iso()
                    save_json_atomic(state_file, state)
        finally:
            for selector in selector_cache.values():
                selector.close()
            push_client.close()

        if errors:
            run_state["status"] = "failed"
            run_state["errors"] = errors
            run_state["updated_at"] = utc_now_iso()
            state["status"] = "failed"
            state["updated_at"] = utc_now_iso()
            save_json_atomic(state_file, state)
            print("Notification run failed.")
            for message in errors:
                print(message)
            return 1

        state["delivery_dedupe"] = prune_delivery_dedupe(
            delivery_dedupe,
            args.dedupe_retention_days,
        )
        run_state["status"] = "completed"
        run_state["completed_at"] = utc_now_iso()
        run_state["updated_at"] = utc_now_iso()
        run_state["done_issue_keys"] = pending_issue_keys
        run_state["done_inpress_keys"] = pending_inpress_keys
        run_state["pending_issue_keys"] = []
        run_state["pending_inpress_keys"] = []
        state["status"] = "completed"
        state["last_completed_run_at"] = utc_now_iso()
        state["snapshot"] = {
            "issue_article_counts": current_issue_counts,
            "inpress_article_counts": current_inpress_counts,
        }
        state["updated_at"] = utc_now_iso()
        save_json_atomic(state_file, state)
        print("Notification run completed successfully.")
    return 0


def run_notification(args: argparse.Namespace) -> int:
    """
    Execute notification pipeline for one or more databases.

    Args:
        args: Parsed CLI arguments.

    Returns:
        Process exit code.
    """
    db_paths = _resolve_target_db_paths(args)
    exit_code = 0
    for db_path in db_paths:
        result = _run_notification_for_db(args, db_path)
        if result != 0:
            exit_code = result
    return exit_code
