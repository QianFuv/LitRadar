"""Authentication routes (register, login, token management)."""

from __future__ import annotations

import re
import sqlite3
from typing import Annotated

from fastapi import APIRouter, Cookie, Depends, Header, HTTPException, Request, Response

from paper_scanner.api.auth_db import (
    change_password,
    count_users,
    create_access_token,
    create_invite_code,
    get_user_invite_code,
    list_access_tokens,
    register_with_invite,
    revoke_access_token,
    revoke_access_token_value,
    revoke_tokens_by_name,
    verify_user,
)
from paper_scanner.api.auth_deps import (
    SESSION_COOKIE_NAME,
    clear_session_cookie,
    get_current_user,
    resolve_auth_token,
    set_session_cookie,
)
from paper_scanner.api.models import (
    ChangePasswordRequest,
    InviteCodeResponse,
    LoginRequest,
    LoginResponse,
    RegisterRequest,
    TokenCreateRequest,
    TokenCreateResponse,
    TokenInfo,
    UserResponse,
)
from paper_scanner.shared.constants import API_PREFIX

router = APIRouter(prefix=f"{API_PREFIX}/auth", tags=["auth"])

_USERNAME_RE = re.compile(r"^[a-zA-Z0-9_]{3,32}$")

CurrentUser = Annotated[dict, Depends(get_current_user)]


@router.post("/register", response_model=UserResponse)
async def register(body: RegisterRequest):
    """Register a new user account (requires invite code unless first user)."""
    username = body.username.strip()
    if not _USERNAME_RE.match(username):
        raise HTTPException(
            status_code=400,
            detail="Username must be 3-32 alphanumeric or underscore characters",
        )
    if len(body.password) < 6:
        raise HTTPException(
            status_code=400,
            detail="Password must be at least 6 characters",
        )
    try:
        user = register_with_invite(username, body.password, body.invite_code or None)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from None
    except sqlite3.IntegrityError:
        raise HTTPException(status_code=409, detail="Username already exists") from None
    return UserResponse(
        id=user["id"],
        username=user["username"],
        is_admin=bool(user.get("is_admin")),
    )


@router.post("/login", response_model=LoginResponse)
async def login(body: LoginRequest, request: Request, response: Response):
    """
    Authenticate and set a browser session cookie.

    Args:
        body: Login credentials.
        request: Incoming request used to infer cookie security.
        response: Response receiving the session cookie.

    Returns:
        Safe login response without the raw session token.
    """
    user = verify_user(body.username.strip(), body.password)
    if not user:
        raise HTTPException(status_code=401, detail="Invalid username or password")
    revoke_tokens_by_name(user["id"], "login")
    token_data = create_access_token(user["id"], name="login")
    set_session_cookie(response, token_data["token"], token_data["expires_at"], request)
    return LoginResponse(
        user=UserResponse(
            id=user["id"],
            username=user["username"],
            is_admin=user.get("is_admin", False),
        ),
        expires_at=token_data["expires_at"],
    )


@router.get("/me", response_model=UserResponse)
async def get_me(user: CurrentUser):
    """Get the authenticated user's profile."""
    return UserResponse(
        id=user["id"],
        username=user["username"],
        is_admin=user.get("is_admin", False),
    )


@router.post("/change-password")
async def api_change_password(body: ChangePasswordRequest, user: CurrentUser):
    """Change the authenticated user's password."""
    if len(body.new_password) < 6:
        raise HTTPException(
            status_code=400,
            detail="New password must be at least 6 characters",
        )
    ok = change_password(user["id"], body.old_password, body.new_password)
    if not ok:
        raise HTTPException(status_code=400, detail="Old password is incorrect")
    return {"ok": True}


@router.post("/logout")
async def logout_current_session(
    user: CurrentUser,
    request: Request,
    response: Response,
    authorization: str | None = Header(default=None),
    session_cookie: str | None = Cookie(default=None, alias=SESSION_COOKIE_NAME),
):
    """
    Revoke the current session token and clear the browser session cookie.

    Args:
        user: Current authenticated user.
        request: Incoming request used to infer cookie security.
        response: Response receiving the cookie deletion header.
        authorization: Optional Authorization header.
        session_cookie: Optional browser session cookie value.

    Returns:
        Logout result.
    """
    raw_token = resolve_auth_token(authorization, session_cookie)
    if not raw_token:
        raise HTTPException(status_code=401, detail="Token required")

    revoke_access_token_value(raw_token)
    clear_session_cookie(response, request)
    return {"ok": True, "user_id": user["id"]}


@router.post("/tokens", response_model=TokenCreateResponse)
async def create_token(body: TokenCreateRequest, user: CurrentUser):
    """Create a new access token for the authenticated user."""
    ttl = max(3600, min(body.ttl, 365 * 24 * 3600))
    data = create_access_token(user["id"], name=body.name.strip(), ttl=ttl)
    return TokenCreateResponse(
        id=data["id"],
        token=data["token"],
        name=data["name"],
        expires_at=data["expires_at"],
    )


@router.get("/tokens", response_model=list[TokenInfo])
async def get_tokens(user: CurrentUser):
    """List all active access tokens for the authenticated user."""
    rows = list_access_tokens(user["id"])
    return [TokenInfo(**r) for r in rows]


@router.delete("/tokens/{token_id}")
async def delete_token(token_id: int, user: CurrentUser):
    """Revoke an access token by ID."""
    ok = revoke_access_token(user["id"], token_id)
    if not ok:
        raise HTTPException(status_code=404, detail="Token not found")
    return {"ok": True}


@router.post("/invite-code", response_model=InviteCodeResponse)
async def generate_invite_code(user: CurrentUser):
    """Generate a one-time invite code. Each user can create exactly one."""
    try:
        data = create_invite_code(user["id"])
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from None
    return InviteCodeResponse(
        id=data["id"],
        code=data["code"],
        used=False,
        created_at=data["created_at"],
    )


@router.get("/invite-code", response_model=InviteCodeResponse | None)
async def get_invite_code(user: CurrentUser):
    """Get the invite code generated by the current user."""
    data = get_user_invite_code(user["id"])
    if not data:
        return None
    return InviteCodeResponse(
        id=data["id"],
        code=data["code"],
        used=data["used_by"] is not None,
        created_at=data["created_at"],
    )


@router.get("/invite-required")
async def check_invite_required():
    """Check if invite code is required (not the first user)."""
    return {"required": count_users() > 0}
