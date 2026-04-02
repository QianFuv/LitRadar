"""Authentication dependencies for FastAPI routes."""

from __future__ import annotations

from fastapi import Header, HTTPException, Query

from scripts.api.auth_db import verify_access_token


async def get_current_user(
    authorization: str | None = Header(default=None),
    access_token: str | None = Query(default=None),
) -> dict:
    """
    Extract and verify the current user from the Authorization header.

    Args:
        authorization: Bearer token passed in the Authorization header.
        access_token: Access token passed as a query parameter.

    Returns:
        Verified user payload with id and username information.
    """
    token = access_token.strip() if access_token else ""
    if not token:
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
    access_token: str | None = Query(default=None),
) -> dict | None:
    """
    Extract and verify the current user when authentication is optional.

    Args:
        authorization: Bearer token passed in the Authorization header.
        access_token: Access token passed as a query parameter.

    Returns:
        Verified user payload or None when no valid token is provided.
    """
    if not authorization and not access_token:
        return None
    try:
        return await get_current_user(authorization, access_token)
    except HTTPException:
        return None


async def get_admin_user(
    authorization: str | None = Header(default=None),
    access_token: str | None = Query(default=None),
) -> dict:
    """
    Extract, verify, and require admin privilege.

    Args:
        authorization: Bearer token passed in the Authorization header.
        access_token: Access token passed as a query parameter.

    Returns:
        Verified admin user payload.
    """
    user = await get_current_user(authorization, access_token)
    if not user.get("is_admin"):
        raise HTTPException(status_code=403, detail="Admin access required")
    return user
