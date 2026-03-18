"""Notification subscriber and runtime configuration helpers."""

from __future__ import annotations

import os

from scripts.notify.models import (
    DEFAULT_SILICONFLOW_MODEL,
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
    siliconflow_api_key = _read_env("NOTIFY_SILICONFLOW_API_KEY") or _read_env(
        "SILICONFLOW_API_KEY"
    )
    pushplus_channel = _read_env("NOTIFY_PUSHPLUS_CHANNEL") or PUSHPLUS_CHANNEL
    if not pushplus_channel:
        pushplus_channel = PUSHPLUS_CHANNEL
    pushplus_template = _read_env("NOTIFY_PUSHPLUS_TEMPLATE") or "markdown"
    if not pushplus_template:
        pushplus_template = "markdown"
    pushplus_topic = _read_env("NOTIFY_PUSHPLUS_TOPIC") or None
    pushplus_option = _read_env("NOTIFY_PUSHPLUS_OPTION") or None

    global_config = NotificationGlobal(
        siliconflow_api_key=siliconflow_api_key,
        pushplus_channel=pushplus_channel,
        pushplus_template=pushplus_template,
        pushplus_topic=pushplus_topic,
        pushplus_option=pushplus_option,
    )

    max_candidates = to_int(_read_env("NOTIFY_MAX_CANDIDATES")) or 120
    temperature = to_float(_read_env("NOTIFY_TEMPERATURE")) or 0.2

    siliconflow_model = _read_env("NOTIFY_SILICONFLOW_MODEL")
    if not siliconflow_model:
        siliconflow_model = DEFAULT_SILICONFLOW_MODEL

    defaults = NotificationDefaults(
        max_candidates=max(1, max_candidates),
        siliconflow_model=siliconflow_model,
        temperature=max(0.0, min(1.0, temperature)),
    )
    return global_config, defaults


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
            )
        )
    return subscribers
