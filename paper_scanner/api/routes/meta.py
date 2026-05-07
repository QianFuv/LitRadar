"""Metadata route registration."""

from __future__ import annotations

from fastapi import APIRouter, Depends

from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.models import JournalOption, ValueCount, YearSummary
from paper_scanner.api.queries.meta import (
    list_areas,
    list_databases,
    list_journal_options,
    list_sources,
    list_years,
)
from paper_scanner.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX, dependencies=[Depends(get_current_user)])

router.add_api_route(
    "/meta/databases",
    list_databases,
    methods=["GET"],
    response_model=list[str],
)
router.add_api_route(
    "/meta/areas",
    list_areas,
    methods=["GET"],
    response_model=list[ValueCount],
)
router.add_api_route(
    "/meta/journals",
    list_journal_options,
    methods=["GET"],
    response_model=list[JournalOption],
)
router.add_api_route(
    "/meta/sources",
    list_sources,
    methods=["GET"],
    response_model=list[ValueCount],
)
router.add_api_route(
    "/years",
    list_years,
    methods=["GET"],
    response_model=list[YearSummary],
)
