"""Weekly route registration."""

from __future__ import annotations

from fastapi import APIRouter, Depends

from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.models import WeeklyUpdatesResponse
from paper_scanner.api.queries.weekly import get_weekly_updates
from paper_scanner.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX, dependencies=[Depends(get_current_user)])

router.add_api_route(
    "/weekly-updates",
    get_weekly_updates,
    methods=["GET"],
    response_model=WeeklyUpdatesResponse,
)
