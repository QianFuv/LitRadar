"""Zhejiang Library CNKI session routes."""

from __future__ import annotations

import asyncio
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException

from paper_scanner.api.auth_db import (
    delete_cnki_session,
    get_cnki_session,
    get_cnki_session_status,
    upsert_cnki_session,
)
from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.models import (
    CnkiLoginPollRequest,
    CnkiLoginPollResponse,
    CnkiLoginStartResponse,
    CnkiSessionStatusResponse,
)
from paper_scanner.shared.constants import API_PREFIX
from paper_scanner.sources.zjlib_cnki import ZhejiangLibraryCnkiClient, ZjlibCnkiError

router = APIRouter(prefix=f"{API_PREFIX}/cnki", tags=["cnki"])
CurrentUser = Annotated[dict, Depends(get_current_user)]


def _cnki_error_detail(code: str, phase: str, message: str) -> dict[str, str]:
    """
    Build a structured CNKI route error payload.

    Args:
        code: Stable error code for frontend branching.
        phase: CNKI login phase that failed.
        message: Human-readable upstream error message.

    Returns:
        Structured error detail payload.
    """
    return {"code": code, "phase": phase, "message": message}


@router.get("/session", response_model=CnkiSessionStatusResponse)
async def get_session(user: CurrentUser):
    """
    Return the current user's CNKI session status.

    Args:
        user: Current authenticated user.

    Returns:
        Safe session status.
    """
    return get_cnki_session_status(user["id"])


@router.post("/login/start", response_model=CnkiLoginStartResponse)
async def start_login(user: CurrentUser):
    """
    Start QR login for the current user's CNKI session.

    Args:
        user: Current authenticated user.

    Returns:
        QR login challenge and safe session status.
    """

    def run_start() -> tuple[str, str, str, dict]:
        """
        Run the blocking QR login start call.

        Returns:
            UUID, status, QR code, and session state data.
        """
        with ZhejiangLibraryCnkiClient() as client:
            qr_login = client.start_qr_login()
            return (
                qr_login.uuid,
                qr_login.status,
                qr_login.qr_code,
                client.to_state_data(),
            )

    try:
        uuid, status, qr_code, session_data = await asyncio.to_thread(run_start)
    except ZjlibCnkiError as exc:
        raise HTTPException(
            status_code=502,
            detail=_cnki_error_detail(
                "cnki_login_start_failed",
                "login",
                str(exc),
            ),
        ) from exc

    session_status = upsert_cnki_session(
        user["id"],
        session_data,
        status="waiting_scan",
        qr_uuid=uuid,
    )
    return CnkiLoginStartResponse(
        uuid=uuid,
        status=status,
        qr_code=qr_code,
        session=CnkiSessionStatusResponse(**session_status),
    )


@router.post("/login/poll", response_model=CnkiLoginPollResponse)
async def poll_login(body: CnkiLoginPollRequest, user: CurrentUser):
    """
    Poll QR login and persist the completed current-user CNKI session.

    Args:
        body: Polling parameters.
        user: Current authenticated user.

    Returns:
        Login status and safe session status.
    """
    row = get_cnki_session(user["id"])
    if not row or not row.get("qr_uuid"):
        raise HTTPException(
            status_code=400,
            detail=_cnki_error_detail(
                "cnki_login_not_started",
                "login",
                "CNKI QR login has not been started",
            ),
        )

    def run_poll() -> dict:
        """
        Run the blocking QR login polling call.

        Returns:
            Completed session state data.
        """
        with ZhejiangLibraryCnkiClient(state_data=row["session_data"]) as client:
            try:
                client.poll_qr_login(
                    timeout_seconds=body.timeout_seconds,
                    interval_seconds=body.interval_seconds,
                )
            except ZjlibCnkiError as exc:
                message = str(exc)
                if "Timed out" in message:
                    raise HTTPException(
                        status_code=408,
                        detail=_cnki_error_detail(
                            "cnki_login_timeout",
                            "login",
                            message,
                        ),
                    ) from exc
                raise HTTPException(
                    status_code=400,
                    detail=_cnki_error_detail(
                        "cnki_login_failed",
                        "login",
                        message,
                    ),
                ) from exc
            try:
                client.warm_up_fulltext_session()
            except ZjlibCnkiError as exc:
                raise HTTPException(
                    status_code=502,
                    detail=_cnki_error_detail(
                        "cnki_warmup_failed",
                        "warmup",
                        str(exc),
                    ),
                ) from exc
            return client.to_state_data()

    session_data = await asyncio.to_thread(run_poll)

    session_status = upsert_cnki_session(
        user["id"],
        session_data,
        status="active",
        qr_uuid=str(session_data.get("qr_uuid") or row.get("qr_uuid") or ""),
    )
    return CnkiLoginPollResponse(
        status="COMPLETE",
        session=CnkiSessionStatusResponse(**session_status),
    )


@router.delete("/session", response_model=CnkiSessionStatusResponse)
async def clear_session(user: CurrentUser):
    """
    Delete the current user's stored CNKI session.

    Args:
        user: Current authenticated user.

    Returns:
        Empty safe session status.
    """
    delete_cnki_session(user["id"])
    return CnkiSessionStatusResponse(**get_cnki_session_status(user["id"]))
