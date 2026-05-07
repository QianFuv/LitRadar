"""Database path resolution helpers shared by API and notify modules."""

from __future__ import annotations

from pathlib import Path

from paper_scanner.shared.constants import INDEX_DIR


def list_database_files() -> list[Path]:
    """
    List available SQLite database files.

    Returns:
        Sorted list of `.sqlite` files under index directory.
    """
    INDEX_DIR.mkdir(parents=True, exist_ok=True)
    return sorted(INDEX_DIR.glob("*.sqlite"))


def normalize_database_names(db_names: object) -> list[str]:
    """
    Normalize database names into unique `.sqlite` filenames.

    Args:
        db_names: Raw database names from API or persisted settings.

    Returns:
        Normalized database filenames in first-seen order.
    """
    if db_names is None:
        return []

    if isinstance(db_names, str):
        raw_db_names = [db_names]
    elif isinstance(db_names, (list, tuple, set)):
        raw_db_names = list(db_names)
    else:
        return []

    normalized: list[str] = []
    seen: set[str] = set()
    for db_name in raw_db_names:
        candidate = Path(str(db_name or "").strip()).name
        if not candidate:
            continue
        if not candidate.endswith(".sqlite"):
            candidate = f"{candidate}.sqlite"
        if candidate in seen:
            continue
        seen.add(candidate)
        normalized.append(candidate)
    return normalized


def is_database_selected(
    selected_databases: object,
    db_name: str,
) -> bool:
    """
    Check whether one database is enabled in a selection list.

    Empty selections mean all databases are enabled.

    Args:
        selected_databases: Normalized or raw selected database names.
        db_name: Database name to test.

    Returns:
        True when the database should be included.
    """
    normalized_target = normalize_database_names([db_name])
    if not normalized_target:
        return False

    normalized_selected = set(normalize_database_names(selected_databases))
    if not normalized_selected:
        return True
    return normalized_target[0] in normalized_selected


def resolve_db_path(db_name: str | None) -> Path:
    """
    Resolve database path from optional name.

    Args:
        db_name: Optional database file stem or filename.

    Returns:
        Resolved database path.

    Raises:
        ValueError: Database cannot be resolved unambiguously.
    """
    if db_name:
        candidate = Path(db_name).name
        if not candidate.endswith(".sqlite"):
            candidate = f"{candidate}.sqlite"
        db_path = INDEX_DIR / candidate
        if not db_path.exists():
            raise ValueError("Database not found")
        return db_path

    sqlite_files = list_database_files()
    if len(sqlite_files) == 1:
        return sqlite_files[0]
    if not sqlite_files:
        raise ValueError("No SQLite databases found")
    raise ValueError("Multiple databases found, specify ?db=<name>")
