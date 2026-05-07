"""Shared conversion helpers for script modules."""

from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable, Iterator
from typing import Any

from paper_scanner.shared.constants import SQLITE_INT_MAX, SQLITE_INT_MIN


def to_int(value: Any) -> int | None:
    """
    Convert a value to an integer within SQLite integer bounds.

    Args:
        value: Input value.

    Returns:
        Parsed integer when valid, otherwise None.
    """
    if value is None:
        return None
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    if SQLITE_INT_MIN <= parsed <= SQLITE_INT_MAX:
        return parsed
    return None


def to_int_stable(value: Any, prefix: str) -> int | None:
    """
    Convert to int and fall back to a stable hash when needed.

    Args:
        value: Input value.
        prefix: Domain prefix used to reduce cross-domain collisions.

    Returns:
        Stable integer identifier or None when value is empty.
    """
    if value is None:
        return None
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        parsed = None
    if parsed is not None and SQLITE_INT_MIN <= parsed <= SQLITE_INT_MAX:
        return parsed
    text = f"{prefix}:{value}"
    digest = hashlib.sha256(text.encode("utf-8")).digest()
    raw_value = int.from_bytes(digest[:8], "big", signed=False)
    safe_value = raw_value & SQLITE_INT_MAX
    return safe_value if safe_value != 0 else 1


def to_bool_int(value: Any) -> int | None:
    """
    Convert a value to 0 or 1 when possible.

    Args:
        value: Input value.

    Returns:
        0 or 1 when conversion succeeds, otherwise None.
    """
    if value is None:
        return None
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, (int, float)):
        return 1 if value else 0
    if isinstance(value, str):
        lowered = value.strip().lower()
        if lowered in {"true", "1", "yes"}:
            return 1
        if lowered in {"false", "0", "no"}:
            return 0
    return None


def to_text(value: Any) -> str | None:
    """
    Convert a value to text.

    Args:
        value: Input value.

    Returns:
        Converted string or None.
    """
    if value is None:
        return None
    if isinstance(value, (dict, list)):
        return json.dumps(value, ensure_ascii=True)
    return str(value)


def to_float(value: Any) -> float | None:
    """
    Convert a value to float when possible.

    Args:
        value: Input value.

    Returns:
        Parsed float or None.
    """
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def to_string_list(value: Any) -> list[str]:
    """
    Normalize a list-like value into non-empty strings.

    Args:
        value: Input value.

    Returns:
        Normalized string list.
    """
    if not isinstance(value, list):
        return []
    normalized: list[str] = []
    for item in value:
        text = str(item).strip()
        if text:
            normalized.append(text)
    return normalized


def chunked[T](items: Iterable[T], size: int) -> Iterator[list[T]]:
    """
    Yield iterable values in fixed-size chunks.

    Args:
        items: Input iterable.
        size: Chunk size.

    Returns:
        Iterator of chunk lists.
    """
    if size <= 0:
        size = 1
    bucket: list[T] = []
    for item in items:
        bucket.append(item)
        if len(bucket) >= size:
            yield bucket
            bucket = []
    if bucket:
        yield bucket


def truncate_text(value: str | None, max_length: int) -> str:
    """
    Truncate text to a target maximum length.

    Args:
        value: Source text.
        max_length: Maximum length.

    Returns:
        Truncated text.
    """
    text = (value or "").strip()
    if len(text) <= max_length:
        return text
    return text[: max(0, max_length - 1)].rstrip() + "..."
