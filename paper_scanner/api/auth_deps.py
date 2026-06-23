"""Authentication dependencies for FastAPI routes."""

from __future__ import annotations

import os
import time

from fastapi import Cookie, Header, HTTPException, Request, Response

from paper_scanner.api.auth_db import verify_access_token

SESSION_COOKIE_NAME = "ps_session"
SESSION_COOKIE_PATH = "/"
SESSION_COOKIE_SAMESITE = "lax"
AUTH_COOKIE_SECURE_ENV = "AUTH_COOKIE_SECURE"


def _is_truthy_env_value(value: str) -> bool:
    """
    Check whether an environment value enables a boolean option.

    Args:
        value: Environment variable value.

    Returns:
        True when the value is an enabled boolean value.
    """
    return value.strip().lower() in {"1", "true", "yes", "on"}


def should_use_secure_session_cookie(request: Request | None = None) -> bool:
    """
    Decide whether the browser session cookie should use the Secure flag.

    Args:
        request: Optional request used to infer HTTPS.

    Returns:
        True when the session cookie should be marked Secure.
    """
    configured_value = os.environ.get(AUTH_COOKIE_SECURE_ENV)
    if configured_value is not None:
        return _is_truthy_env_value(configured_value)
    return bool(request and request.url.scheme == "https")


def set_session_cookie(
    response: Response,
    token: str,
    expires_at: float,
    request: Request | None = None,
) -> None:
    """
    Attach the authenticated browser session cookie to a response.

    Args:
        response: Response object receiving the cookie header.
        token: Raw access token stored in the cookie.
        expires_at: Token expiration timestamp.
        request: Optional request used to infer cookie security.

    Returns:
        None.
    """
    max_age = max(0, int(expires_at - time.time()))
    response.set_cookie(
        key=SESSION_COOKIE_NAME,
        value=token,
        max_age=max_age,
        path=SESSION_COOKIE_PATH,
        secure=should_use_secure_session_cookie(request),
        httponly=True,
        samesite=SESSION_COOKIE_SAMESITE,
    )


def clear_session_cookie(
    response: Response,
    request: Request | None = None,
) -> None:
    """
    Clear the authenticated browser session cookie from a response.

    Args:
        response: Response object receiving the cookie deletion header.
        request: Optional request used to infer cookie security.

    Returns:
        None.
    """
    response.delete_cookie(
        key=SESSION_COOKIE_NAME,
        path=SESSION_COOKIE_PATH,
        secure=should_use_secure_session_cookie(request),
        httponly=True,
        samesite=SESSION_COOKIE_SAMESITE,
    )


def _extract_bearer_token(authorization: str | None) -> str:
    """
    Extract a bearer token from an Authorization header.

    Args:
        authorization: Optional Authorization header.

    Returns:
        Raw bearer token.

    Raises:
        HTTPException: If a provided Authorization header is invalid.
    """
    if not authorization:
        return ""
    parts = authorization.split(" ", maxsplit=1)
    if len(parts) != 2 or parts[0].lower() != "bearer":
        raise HTTPException(status_code=401, detail="Invalid authorization format")
    return parts[1].strip()


def resolve_auth_token(
    authorization: str | None = None,
    session_cookie: str | None = None,
) -> str:
    """
    Resolve the raw auth token from supported browser and API transports.

    Args:
        authorization: Optional Authorization header.
        session_cookie: Optional browser session cookie value.

    Returns:
        Raw auth token or an empty string when no token is available.
    """
    bearer_token = _extract_bearer_token(authorization)
    if bearer_token:
        return bearer_token
    return session_cookie.strip() if session_cookie else ""


async def get_current_user(
    authorization: str | None = Header(default=None),
    session_cookie: str | None = Cookie(default=None, alias=SESSION_COOKIE_NAME),
) -> dict:
    """
    Extract and verify the current user from supported auth transports.

    Args:
        authorization: Bearer token passed in the Authorization header.
        session_cookie: Raw login token passed in the browser session cookie.

    Returns:
        Verified user payload with id and username information.
    """
    token = resolve_auth_token(authorization, session_cookie)
    if not token:
        raise HTTPException(status_code=401, detail="Authentication required")

    user = verify_access_token(token)
    if not user:
        raise HTTPException(status_code=401, detail="Invalid or expired token")

    return user


async def get_optional_user(
    authorization: str | None = Header(default=None),
    session_cookie: str | None = Cookie(default=None, alias=SESSION_COOKIE_NAME),
) -> dict | None:
    """
    Extract and verify the current user when authentication is optional.

    Args:
        authorization: Bearer token passed in the Authorization header.
        session_cookie: Raw login token passed in the browser session cookie.

    Returns:
        Verified user payload or None when no valid token is provided.
    """
    if not authorization and not session_cookie:
        return None
    try:
        return await get_current_user(authorization, session_cookie)
    except HTTPException:
        return None


async def get_admin_user(
    authorization: str | None = Header(default=None),
    session_cookie: str | None = Cookie(default=None, alias=SESSION_COOKIE_NAME),
) -> dict:
    """
    Extract, verify, and require admin privilege.

    Args:
        authorization: Bearer token passed in the Authorization header.
        session_cookie: Raw login token passed in the browser session cookie.

    Returns:
        Verified admin user payload.
    """
    user = await get_current_user(authorization, session_cookie)
    if not user.get("is_admin"):
        raise HTTPException(status_code=403, detail="Admin access required")
    return user
