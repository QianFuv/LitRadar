"""Issue route registration."""

from __future__ import annotations

from fastapi import APIRouter, Depends

from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.models import IssuePage, IssueRecord
from paper_scanner.api.queries.issues import get_issue, list_issues
from paper_scanner.shared.constants import API_PREFIX

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
