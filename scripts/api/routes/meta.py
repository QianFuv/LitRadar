"""Metadata route registration."""

from __future__ import annotations

from fastapi import APIRouter, Depends

from scripts.api.auth_deps import get_current_user
from scripts.api.models import JournalOption, ValueCount, YearSummary
from scripts.api.queries.meta import (
    list_areas,
    list_databases,
    list_journal_options,
    list_libraries,
    list_years,
)
from scripts.shared.constants import API_PREFIX

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
    "/meta/libraries",
    list_libraries,
    methods=["GET"],
    response_model=list[ValueCount],
)
router.add_api_route(
    "/years",
    list_years,
    methods=["GET"],
    response_model=list[YearSummary],
)
