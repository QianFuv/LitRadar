"""Authentication dependencies for FastAPI routes."""

from __future__ import annotations

from fastapi import Header, HTTPException

from scripts.api.auth_db import verify_access_token


async def get_current_user(
    authorization: str | None = Header(default=None),
) -> dict:
    """
    Extract and verify the current user from the Authorization header.

    Expects: Authorization: Bearer <token>
    Returns user dict with id and username.
    """
    if not authorization:
        raise HTTPException(status_code=401, detail="Authorization header required")

    parts = authorization.split(" ", maxsplit=1)
    if len(parts) != 2 or parts[0].lower() != "bearer":
        raise HTTPException(status_code=401, detail="Invalid authorization format")

    token = parts[1].strip()
    if not token:
        raise HTTPException(status_code=401, detail="Token required")

    user = verify_access_token(token)
    if not user:
        raise HTTPException(status_code=401, detail="Invalid or expired token")

    return user


async def get_optional_user(
    authorization: str | None = Header(default=None),
) -> dict | None:
    """Like get_current_user but returns None if no auth header."""
    if not authorization:
        return None
    try:
        return await get_current_user(authorization)
    except HTTPException:
        return None


async def get_admin_user(
    authorization: str | None = Header(default=None),
) -> dict:
    """
    Extract, verify, and require admin privilege.

    Returns admin user dict or raises 401/403.
    """
    user = await get_current_user(authorization)
    if not user.get("is_admin"):
        raise HTTPException(status_code=403, detail="Admin access required")
    return user
