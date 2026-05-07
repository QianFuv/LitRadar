"""Health route registration."""

from __future__ import annotations

from fastapi import APIRouter

from paper_scanner.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX)


async def health() -> dict[str, str]:
    """
    Health check endpoint.

    Returns:
        Health status payload.
    """
    return {"status": "ok"}


router.add_api_route("/health", health, methods=["GET"])
