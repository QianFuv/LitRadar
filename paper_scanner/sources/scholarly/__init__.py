"""Scholarly metadata integration utilities."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from paper_scanner.sources.scholarly.client import ScholarlyClient

__all__ = ["ScholarlyClient"]


def __getattr__(name: str) -> object:
    """
    Lazily load exported scholarly integration objects.

    Args:
        name: Export name.

    Returns:
        Exported object.

    Raises:
        AttributeError: If the export name is unknown.
    """
    if name == "ScholarlyClient":
        from paper_scanner.sources.scholarly.client import ScholarlyClient

        return ScholarlyClient
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
