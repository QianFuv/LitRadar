"""Public announcement routes."""

from __future__ import annotations

from fastapi import APIRouter

from paper_scanner.api.auth_db import list_active_announcements
from paper_scanner.api.models import AnnouncementInfo
from paper_scanner.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX, tags=["announcements"])


@router.get("/announcements", response_model=list[AnnouncementInfo])
async def get_announcements():
    """
    List active system announcements.

    Returns:
        Enabled announcement records.
    """
    return [AnnouncementInfo(**item) for item in list_active_announcements()]
