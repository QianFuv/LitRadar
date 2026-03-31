"""Issue route registration."""

from __future__ import annotations

from fastapi import APIRouter, Depends

from scripts.api.auth_deps import get_current_user
from scripts.api.models import IssuePage, IssueRecord
from scripts.api.queries.issues import get_issue, list_issues
from scripts.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX, dependencies=[Depends(get_current_user)])

router.add_api_route(
    "/issues",
    list_issues,
    methods=["GET"],
    response_model=IssuePage,
)
router.add_api_route(
    "/issues/{issue_id}",
    get_issue,
    methods=["GET"],
    response_model=IssueRecord,
)
