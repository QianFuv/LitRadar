"""Notification workflow orchestration."""

from __future__ import annotations

import argparse
import sqlite3

from scripts.notify.ai_selector import OpenAICompatibleSelector
from scripts.notify.candidates import (
    deduplicate_candidates,
    fetch_candidates_for_inpress_keys,
    fetch_candidates_for_issue_keys,
)
from scripts.notify.changes import (
    collect_inpress_article_counts,
    collect_issue_article_counts,
    compute_changed_inpress_keys,
    compute_changed_issue_keys,
)
from scripts.notify.delivery import (
    load_change_manifest,
    prune_delivery_dedupe,
    resolve_path,
)
from scripts.notify.message import build_markdown_content, build_message_title
from scripts.notify.models import (
    MAX_AI_SELECTION_ROUNDS,
    ArticleCandidate,
    RankedSelection,
    Subscriber,
)
from scripts.notify.pushplus import PushPlusClient
from scripts.notify.selection import apply_selection_rules, select_articles_with_retries
from scripts.notify.state import (
    create_run_state,
    load_state,
    save_json_atomic,
    utc_now_iso,
)
from scripts.notify.subscriptions import (
    load_notification_config,
    load_subscribers_from_db,
    resolve_ai_runtime_config,
)
from scripts.shared.constants import PROJECT_ROOT
from scripts.shared.converters import to_int
from scripts.shared.db_path import resolve_db_path


def select_all_candidates(
    subscriber: Subscriber,
    candidates: list[ArticleCandidate],
    delivery_dedupe: dict[str, str],
) -> list[RankedSelection]:
    """
    Return all non-deduplicated candidates in their original order.

    Args:
        subscriber: Subscriber receiving articles.
        candidates: Candidate articles for the current database run.
        delivery_dedupe: Per-subscriber delivery history.

    Returns:
        Ranked selections with neutral scores.
    """
    accepted: list[RankedSelection] = []
    for candidate in candidates:
        delivery_key = f"{subscriber.subscriber_id}:{candidate.article_id}"
        if delivery_key in delivery_dedupe:
            continue
        accepted.append(
            RankedSelection(
                article_id=candidate.article_id,
                score=0.0,
            )
        )
    return accepted


def run_notification(args: argparse.Namespace) -> int:
    """
    Execute notification pipeline.

    Args:
        args: Parsed CLI arguments.

    Returns:
        Process exit code.
    """
    try:
        db_path = resolve_db_path(args.db)
    except ValueError as exc:
        raise SystemExit(str(exc)) from exc
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

        if not subscribers:
            run_state["status"] = "skipped"
            run_state["updated_at"] = utc_now_iso()
            state["status"] = "skipped"
            state["updated_at"] = utc_now_iso()
            save_json_atomic(state_file, state)
            print(
                "No valid subscribers found — skipping run"
                " so articles remain available for the next attempt."
            )
            return 0

        model_override = str(args.siliconflow_model or "").strip() or None
        max_candidates = args.max_candidates or defaults.max_candidates
        max_candidates = max(1, max_candidates)
        candidates_for_model = all_candidates[:max_candidates]
        candidates_by_id = {item.article_id: item for item in all_candidates}
        selector_cache: dict[
            tuple[str, str, str, str],
            OpenAICompatibleSelector,
        ] = {}
        push_client = PushPlusClient(timeout_seconds=args.timeout, retries=args.retries)

        delivery_dedupe = state.get("delivery_dedupe")
        if not isinstance(delivery_dedupe, dict):
            delivery_dedupe = {}
        state["delivery_dedupe"] = delivery_dedupe

        errors: list[str] = []

        try:
            for subscriber in subscribers:
                try:
                    final_summary = ""
                    if subscriber.keywords or subscriber.directions:
                        ai_config = resolve_ai_runtime_config(
                            base_url=subscriber.ai_base_url,
                            api_key=subscriber.ai_api_key,
                            model=subscriber.ai_model,
                            system_prompt=subscriber.ai_system_prompt,
                            global_config=global_config,
                            defaults=defaults,
                            override_model=model_override,
                        )
                        if ai_config is None:
                            accepted = select_all_candidates(
                                subscriber,
                                all_candidates,
                                delivery_dedupe,
                            )
                        else:
                            selector_key = (
                                ai_config["base_url"],
                                ai_config["api_key"],
                                ai_config["model"],
                                ai_config["system_prompt"],
                            )
                            selector = selector_cache.get(selector_key)
                            if selector is None:
                                selector = OpenAICompatibleSelector(
                                    api_key=ai_config["api_key"],
                                    model=ai_config["model"],
                                    timeout_seconds=args.timeout,
                                    retries=args.retries,
                                    temperature=defaults.temperature,
                                    base_url=ai_config["base_url"] or None,
                                    system_prompt=ai_config["system_prompt"],
                                )
                                selector_cache[selector_key] = selector
                            selection_result = select_articles_with_retries(
                                selector,
                                subscriber,
                                defaults,
                                candidates_for_model,
                                candidates_by_id,
                                delivery_dedupe,
                                MAX_AI_SELECTION_ROUNDS,
                            )
                            accepted = apply_selection_rules(
                                selection_result,
                                subscriber,
                                candidates_by_id,
                                delivery_dedupe,
                            )

                            selected_candidates = [
                                candidates_by_id[item.article_id]
                                for item in accepted
                                if item.article_id in candidates_by_id
                            ]

                            final_summary = selection_result.summary
                            if selected_candidates:
                                try:
                                    summarized = selector.summarize_selected_articles(
                                        subscriber,
                                        selected_candidates,
                                    )
                                    if summarized:
                                        final_summary = summarized
                                except Exception:
                                    final_summary = selection_result.summary
                    else:
                        accepted = select_all_candidates(
                            subscriber,
                            all_candidates,
                            delivery_dedupe,
                        )

                    if not accepted:
                        run_state["user_results"].append(
                            {
                                "subscriber_id": subscriber.subscriber_id,
                                "selected_count": 0,
                                "pushed_count": 0,
                                "message_id": None,
                                "status": "skipped",
                                "error": None,
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

                    message_id = ""
                    if args.dry_run:
                        print(
                            "DRY RUN",
                            subscriber.subscriber_id,
                            f"selected={len(accepted)}",
                        )
                    elif (
                        subscriber.delivery_method == "folder"
                        and subscriber.tracking_folder_id is not None
                    ):
                        from scripts.api.auth_db import bulk_add_favorites

                        folder_articles = [
                            {
                                "article_id": item.article_id,
                                "db_name": db_path.name,
                            }
                            for item in accepted
                            if item.article_id in candidates_by_id
                        ]
                        bulk_add_favorites(
                            int(subscriber.subscriber_id),
                            subscriber.tracking_folder_id,
                            folder_articles,
                        )
                        sent_at = utc_now_iso()
                        for item in accepted:
                            delivery_key = (
                                f"{subscriber.subscriber_id}:{item.article_id}"
                            )
                            delivery_dedupe[delivery_key] = sent_at
                    else:
                        message_id = push_client.send(
                            token=subscriber.pushplus_token,
                            title=message_title,
                            content=content,
                            channel=global_config.pushplus_channel,
                            template=subscriber.template
                            or global_config.pushplus_template,
                            topic=subscriber.topic or global_config.pushplus_topic,
                            option=global_config.pushplus_option,
                            to=subscriber.to,
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
