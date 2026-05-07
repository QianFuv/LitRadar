"""Runtime configuration loading for external metadata services."""

from __future__ import annotations

import os
import sqlite3
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from paper_scanner.shared.constants import PROJECT_ROOT

RuntimeInputType = Literal["text", "password", "email", "boolean"]


@dataclass(frozen=True)
class RuntimeConfigDefinition:
    """
    Describe one managed runtime configuration value.

    Args:
        field: API field name.
        env_name: Environment variable name used by runtime clients.
        label: Human-readable setting name.
        input_type: Frontend input type.
        is_secret: Whether the value contains credentials.
        description: Human-readable setting description.
        default_value: Default value when neither database nor environment is set.
    """

    field: str
    env_name: str
    label: str
    input_type: RuntimeInputType
    is_secret: bool
    description: str
    default_value: str = ""


RUNTIME_CONFIG_DEFINITIONS = (
    RuntimeConfigDefinition(
        field="openalex_api_key_pool",
        env_name="OPENALEX_API_KEY_POOL",
        label="OpenAlex API key pool",
        input_type="password",
        is_secret=True,
        description="OpenAlex authenticated request key pool.",
    ),
    RuntimeConfigDefinition(
        field="proxy_pool",
        env_name="PROXY_POOL",
        label="Proxy pool",
        input_type="password",
        is_secret=True,
        description=(
            "Comma- or semicolon-separated proxy URLs for scholarly and CNKI requests."
        ),
    ),
    RuntimeConfigDefinition(
        field="crossref_mailto_pool",
        env_name="CROSSREF_MAILTO_POOL",
        label="Crossref mailto pool",
        input_type="text",
        is_secret=False,
        description="Comma- or semicolon-separated Crossref contact emails.",
    ),
    RuntimeConfigDefinition(
        field="unpaywall_email_pool",
        env_name="UNPAYWALL_EMAIL_POOL",
        label="Unpaywall email pool",
        input_type="text",
        is_secret=False,
        description="Comma- or semicolon-separated Unpaywall contact emails.",
    ),
)
RUNTIME_CONFIG_BY_FIELD = {
    definition.field: definition for definition in RUNTIME_CONFIG_DEFINITIONS
}
AUTH_DB_PATH = PROJECT_ROOT / "data" / "auth.sqlite"


def normalize_runtime_bool(value: object, default: bool = True) -> bool:
    """
    Normalize a runtime boolean value.

    Args:
        value: Raw boolean-like value.
        default: Default value for empty values.

    Returns:
        Normalized boolean.

    Raises:
        ValueError: If the value cannot be parsed as a boolean.
    """
    if value is None:
        return default
    if isinstance(value, bool):
        return value
    text = str(value).strip().lower()
    if not text:
        return default
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    raise ValueError(f"Invalid boolean value: {value}")


def runtime_bool_to_text(value: object, default: bool = True) -> str:
    """
    Convert a boolean-like value to a runtime text value.

    Args:
        value: Raw boolean-like value.
        default: Default value for empty values.

    Returns:
        ``true`` or ``false``.
    """
    return "true" if normalize_runtime_bool(value, default) else "false"


def read_database_runtime_env(db_path: Path = AUTH_DB_PATH) -> dict[str, str]:
    """
    Read managed runtime environment values from the auth database.

    Args:
        db_path: Auth database path.

    Returns:
        Mapping of environment variable names to stored values.
    """
    if not db_path.exists():
        return {}
    conn: sqlite3.Connection | None = None
    try:
        conn = sqlite3.connect(str(db_path))
        rows = conn.execute("SELECT key, value FROM runtime_settings").fetchall()
    except sqlite3.Error:
        return {}
    finally:
        if conn is not None:
            conn.close()
    env_names = {definition.env_name for definition in RUNTIME_CONFIG_DEFINITIONS}
    return {str(key): str(value) for key, value in rows if str(key) in env_names}


def apply_runtime_config(db_path: Path = AUTH_DB_PATH) -> dict[str, str]:
    """
    Apply managed runtime settings to ``os.environ``.

    Args:
        db_path: Auth database path.

    Returns:
        Applied database values keyed by environment variable name.
    """
    values = read_database_runtime_env(db_path)
    for definition in RUNTIME_CONFIG_DEFINITIONS:
        if definition.env_name not in values:
            continue
        value = values[definition.env_name].strip()
        if value:
            os.environ[definition.env_name] = value
        else:
            os.environ.pop(definition.env_name, None)
    return values
