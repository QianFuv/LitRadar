"""Admin routes – user management, invite codes, system statistics."""

from __future__ import annotations

import contextlib
import json
import sqlite3
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException

from scripts.api.auth_db import (
    admin_create_invite_code,
    admin_reset_password,
    delete_invite_code,
    delete_user,
    get_auth_stats,
    list_all_invite_codes,
    list_all_users,
    set_user_admin,
)
from scripts.api.auth_deps import get_admin_user
from scripts.api.models import (
    AdminInviteCodeInfo,
    AdminResetPassword,
    AdminSetAdmin,
    AdminUserInfo,
)
from scripts.shared.constants import API_PREFIX, PUSH_STATE_DIR
from scripts.shared.db_path import list_database_files

router = APIRouter(prefix=f"{API_PREFIX}/admin", tags=["admin"])

AdminUser = Annotated[dict, Depends(get_admin_user)]


@router.get("/users", response_model=list[AdminUserInfo])
async def admin_list_users(_admin: AdminUser):
    """List all users with stats."""
    return list_all_users()


@router.put("/users/{user_id}/admin")
async def admin_set_admin(
    user_id: int,
    body: AdminSetAdmin,
    admin: AdminUser,
):
    """Grant or revoke admin status."""
    if user_id == admin["id"] and not body.is_admin:
        raise HTTPException(status_code=400, detail="Cannot revoke own admin status")
    if not set_user_admin(user_id, body.is_admin):
        raise HTTPException(status_code=404, detail="User not found")
    return {"ok": True}


@router.post("/users/{user_id}/reset-password")
async def admin_reset_pw(
    user_id: int,
    body: AdminResetPassword,
    _admin: AdminUser,
):
    """Reset a user's password."""
    if len(body.new_password) < 6:
        raise HTTPException(
            status_code=400,
            detail="Password must be at least 6 characters",
        )
    if not admin_reset_password(user_id, body.new_password):
        raise HTTPException(status_code=404, detail="User not found")
    return {"ok": True}


@router.delete("/users/{user_id}")
async def admin_delete_user(
    user_id: int,
    admin: AdminUser,
):
    """Delete a user and all associated data."""
    if user_id == admin["id"]:
        raise HTTPException(status_code=400, detail="Cannot delete yourself")
    if not delete_user(user_id):
        raise HTTPException(status_code=404, detail="User not found")
    return {"ok": True}


@router.get("/invite-codes", response_model=list[AdminInviteCodeInfo])
async def admin_list_invite_codes(_admin: AdminUser):
    """List all invite codes."""
    return list_all_invite_codes()


@router.post("/invite-codes")
async def admin_generate_invite_code(_admin: AdminUser):
    """Generate an invite code (admin-created, no user limit)."""
    data = admin_create_invite_code()
    return data


@router.delete("/invite-codes/{code_id}")
async def admin_delete_invite_code(code_id: int, _admin: AdminUser):
    """Delete an unused invite code."""
    if not delete_invite_code(code_id):
        raise HTTPException(
            status_code=404,
            detail="Code not found or already used",
        )
    return {"ok": True}


@router.get("/stats")
async def admin_stats(_admin: AdminUser):
    """
    Comprehensive system statistics: auth, index databases, push state.
    """
    auth = get_auth_stats()

    db_files = list_database_files()
    index_stats: list[dict] = []
    total_articles = 0
    total_journals = 0

    for db_path in db_files:
        try:
            conn = sqlite3.connect(str(db_path))
            conn.row_factory = sqlite3.Row
            article_count = conn.execute("SELECT COUNT(*) FROM articles").fetchone()[0]
            journal_count = conn.execute("SELECT COUNT(*) FROM journals").fetchone()[0]
            issue_count = 0
            with contextlib.suppress(sqlite3.OperationalError):
                issue_count = conn.execute("SELECT COUNT(*) FROM issues").fetchone()[0]
            conn.close()
            total_articles += article_count
            total_journals += journal_count
            index_stats.append(
                {
                    "db_name": db_path.name,
                    "articles": article_count,
                    "journals": journal_count,
                    "issues": issue_count,
                }
            )
        except Exception:
            index_stats.append(
                {
                    "db_name": db_path.name,
                    "articles": 0,
                    "journals": 0,
                    "issues": 0,
                    "error": True,
                }
            )

    push_state_files = (
        sorted(PUSH_STATE_DIR.glob("*.json")) if PUSH_STATE_DIR.exists() else []
    )
    push_stats: list[dict] = []
    for pf in push_state_files:
        try:
            with open(pf) as f:
                state = json.load(f)
            run = state.get("run") or {}
            push_stats.append(
                {
                    "db_name": pf.stem,
                    "status": state.get("status", "unknown"),
                    "last_completed": state.get("last_completed_run_at"),
                    "delivered_count": len(run.get("delivered_article_ids", [])),
                    "user_results": len(run.get("user_results", [])),
                }
            )
        except Exception:
            push_stats.append({"db_name": pf.stem, "status": "error"})

    return {
        "auth": auth,
        "index": {
            "databases": index_stats,
            "total_articles": total_articles,
            "total_journals": total_journals,
        },
        "push": push_stats,
    }
