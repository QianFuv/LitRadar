"""Notification subscriber and runtime configuration helpers."""

from __future__ import annotations

import os
from typing import Any

from scripts.notify.models import (
    DEFAULT_OPENAI_BASE_URL,
    DEFAULT_OPENAI_MODEL,
    PUSHPLUS_CHANNEL,
    NotificationDefaults,
    NotificationGlobal,
    Subscriber,
)
from scripts.shared.converters import to_float, to_int


def _read_env(name: str) -> str:
    """
    Read and trim one environment variable.

    Args:
        name: Environment variable name.

    Returns:
        Trimmed string value.
    """
    return str(os.getenv(name) or "").strip()


def load_notification_config() -> tuple[NotificationGlobal, NotificationDefaults]:
    """
    Load runtime notification configuration from environment variables.

    Returns:
        Global config and defaults.
    """
    ai_base_url = _read_env("NOTIFY_AI_BASE_URL") or DEFAULT_OPENAI_BASE_URL
    ai_api_key = _read_env("NOTIFY_AI_API_KEY")
    pushplus_channel = _read_env("NOTIFY_PUSHPLUS_CHANNEL") or PUSHPLUS_CHANNEL
    if not pushplus_channel:
        pushplus_channel = PUSHPLUS_CHANNEL
    pushplus_template = _read_env("NOTIFY_PUSHPLUS_TEMPLATE") or "markdown"
    if not pushplus_template:
        pushplus_template = "markdown"
    pushplus_topic = _read_env("NOTIFY_PUSHPLUS_TOPIC") or None
    pushplus_option = _read_env("NOTIFY_PUSHPLUS_OPTION") or None
    ai_system_prompt = _read_env("NOTIFY_AI_SYSTEM_PROMPT") or None

    global_config = NotificationGlobal(
        ai_base_url=ai_base_url,
        ai_api_key=ai_api_key,
        pushplus_channel=pushplus_channel,
        pushplus_template=pushplus_template,
        pushplus_topic=pushplus_topic,
        pushplus_option=pushplus_option,
        ai_system_prompt=ai_system_prompt,
    )

    max_candidates = to_int(_read_env("NOTIFY_MAX_CANDIDATES")) or 120
    temperature = to_float(_read_env("NOTIFY_TEMPERATURE")) or 0.2

    ai_model = _read_env("NOTIFY_AI_MODEL") or DEFAULT_OPENAI_MODEL

    defaults = NotificationDefaults(
        max_candidates=max(1, max_candidates),
        ai_model=ai_model,
        temperature=max(0.0, min(1.0, temperature)),
    )
    return global_config, defaults


def resolve_ai_runtime_config(
    *,
    base_url: Any,
    api_key: Any,
    model: Any,
    system_prompt: Any,
    global_config: NotificationGlobal,
    defaults: NotificationDefaults,
    override_model: str | None = None,
) -> dict[str, str] | None:
    """
    Resolve the effective OpenAI-compatible AI runtime configuration.

    Args:
        base_url: Per-user base URL value.
        api_key: Per-user API key value.
        model: Per-user model value.
        system_prompt: Per-user system prompt value.
        global_config: Runtime default config from environment.
        defaults: Runtime default model and tuning values.
        override_model: Optional CLI model override.

    Returns:
        Normalized config dict, or None if API key or model is missing.
    """
    resolved_api_key = str(api_key or global_config.ai_api_key or "").strip()
    resolved_model = str(override_model or model or defaults.ai_model or "").strip()
    if not resolved_api_key or not resolved_model:
        return None

    return {
        "base_url": str(base_url or global_config.ai_base_url or "").strip(),
        "api_key": resolved_api_key,
        "model": resolved_model,
        "system_prompt": str(
            system_prompt or global_config.ai_system_prompt or ""
        ).strip(),
    }


def resolve_ai_runtime_configs(
    *,
    base_url: Any,
    api_key: Any,
    model: Any,
    system_prompt: Any,
    backup_base_url: Any,
    backup_api_key: Any,
    backup_model: Any,
    backup_system_prompt: Any,
    global_config: NotificationGlobal,
    defaults: NotificationDefaults,
    override_model: str | None = None,
) -> list[dict[str, str]]:
    """
    Resolve primary and backup OpenAI-compatible AI runtime configurations.

    Args:
        base_url: Primary per-user base URL value.
        api_key: Primary per-user API key value.
        model: Primary per-user model value.
        system_prompt: Primary per-user system prompt value.
        backup_base_url: Backup per-user base URL value.
        backup_api_key: Backup per-user API key value.
        backup_model: Backup per-user model value.
        backup_system_prompt: Backup per-user system prompt value.
        global_config: Runtime default config from environment.
        defaults: Runtime default model and tuning values.
        override_model: Optional CLI model override.

    Returns:
        Ordered list of distinct runtime config dicts.
    """
    configs: list[dict[str, str]] = []

    primary_config = resolve_ai_runtime_config(
        base_url=base_url,
        api_key=api_key,
        model=model,
        system_prompt=system_prompt,
        global_config=global_config,
        defaults=defaults,
        override_model=override_model,
    )
    if primary_config is not None:
        configs.append(primary_config)

    has_backup_override = any(
        str(value or "").strip()
        for value in (
            backup_base_url,
            backup_api_key,
            backup_model,
            backup_system_prompt,
        )
    )
    if not has_backup_override:
        return configs

    backup_config = resolve_ai_runtime_config(
        base_url=backup_base_url,
        api_key=backup_api_key,
        model=backup_model,
        system_prompt=backup_system_prompt,
        global_config=global_config,
        defaults=defaults,
        override_model=override_model,
    )
    if backup_config is None:
        return configs

    backup_key = (
        backup_config["base_url"],
        backup_config["api_key"],
        backup_config["model"],
        backup_config["system_prompt"],
    )
    existing_keys = {
        (
            config["base_url"],
            config["api_key"],
            config["model"],
            config["system_prompt"],
        )
        for config in configs
    }
    if backup_key not in existing_keys:
        configs.append(backup_config)
    return configs


def load_subscribers_from_db() -> list[Subscriber]:
    """
    Load notification subscribers from auth database settings only.

    Returns:
        Valid subscriber list.
    """
    from scripts.api.auth_db import (
        get_tracking_folder,
        init_auth_db,
        list_notification_subscribers,
    )

    init_auth_db()
    db_rows = list_notification_subscribers()
    subscribers: list[Subscriber] = []
    for row in db_rows:
        token = str(row.get("pushplus_token") or "").strip()
        method = str(row.get("delivery_method") or "folder")
        if method == "pushplus" and not token:
            continue

        folder = get_tracking_folder(row["user_id"])
        subscriber_id = str(row["user_id"])

        if method == "folder" and not folder:
            continue

        subscribers.append(
            Subscriber(
                subscriber_id=subscriber_id,
                name=str(row.get("username") or subscriber_id),
                pushplus_token=token,
                to=str(row.get("pushplus_to") or "").strip() or None,
                keywords=row.get("keywords", []),
                directions=row.get("directions", []),
                topic=(str(row.get("pushplus_topic") or "").strip() or None),
                template=(str(row.get("pushplus_template") or "").strip() or None),
                delivery_method=method,
                tracking_folder_id=(folder["id"] if folder else None),
                sync_to_tracking_folder=bool(row.get("sync_to_tracking_folder")),
                ai_base_url=(str(row.get("ai_base_url") or "").strip() or None),
                ai_api_key=(str(row.get("ai_api_key") or "").strip() or None),
                ai_model=(str(row.get("ai_model") or "").strip() or None),
                ai_system_prompt=(
                    str(row.get("ai_system_prompt") or "").strip() or None
                ),
                ai_backup_base_url=(
                    str(row.get("ai_backup_base_url") or "").strip() or None
                ),
                ai_backup_api_key=(
                    str(row.get("ai_backup_api_key") or "").strip() or None
                ),
                ai_backup_model=(str(row.get("ai_backup_model") or "").strip() or None),
                ai_backup_system_prompt=(
                    str(row.get("ai_backup_system_prompt") or "").strip() or None
                ),
                ai_retry_attempts=max(1, to_int(row.get("ai_retry_attempts")) or 3),
            )
        )
    return subscribers
