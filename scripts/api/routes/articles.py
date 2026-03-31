"""Article route registration."""

from __future__ import annotations

from fastapi import APIRouter, Depends

from scripts.api.auth_deps import get_current_user
from scripts.api.models import ArticlePage, ArticleRecord
from scripts.api.queries.articles import (
    get_article,
    list_articles,
    redirect_article_fulltext,
)
from scripts.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX, dependencies=[Depends(get_current_user)])

router.add_api_route(
    "/articles",
    list_articles,
    methods=["GET"],
    response_model=ArticlePage,
)
router.add_api_route(
    "/articles/{article_id}",
    get_article,
    methods=["GET"],
    response_model=ArticleRecord,
)
router.add_api_route(
    "/articles/{article_id}/fulltext",
    redirect_article_fulltext,
    methods=["GET"],
)
