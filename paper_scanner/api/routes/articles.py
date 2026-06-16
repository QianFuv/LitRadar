"""Article route registration."""

from __future__ import annotations

from typing import Annotated

import aiosqlite
from fastapi import APIRouter, Depends
from starlette.responses import Response

from paper_scanner.api.auth_deps import get_current_user
from paper_scanner.api.dependencies import get_db_dependency
from paper_scanner.api.models import ArticleAccessResponse, ArticlePage, ArticleRecord
from paper_scanner.api.queries.articles import (
    get_article,
    get_article_access,
    list_articles,
    redirect_article_fulltext,
)
from paper_scanner.shared.constants import API_PREFIX

router = APIRouter(prefix=API_PREFIX, dependencies=[Depends(get_current_user)])

CurrentUser = Annotated[dict, Depends(get_current_user)]
Database = Annotated[aiosqlite.Connection, Depends(get_db_dependency)]


async def get_article_access_route(
    article_id: int,
    db: Database,
    user: CurrentUser,
) -> ArticleAccessResponse:
    """
    Return access capabilities for one article through route dependencies.

    Args:
        article_id: Article identifier.
        db: Article database connection.
        user: Current authenticated user.

    Returns:
        Article access capability response.
    """
    return await get_article_access(article_id, db, user)


async def redirect_article_fulltext_route(
    article_id: int,
    db: Database,
    user: CurrentUser,
) -> Response:
    """
    Return an article full-text redirect or provider PDF response.

    Args:
        article_id: Article identifier.
        db: Article database connection.
        user: Current authenticated user.

    Returns:
        Redirect or PDF response.
    """
    return await redirect_article_fulltext(article_id, db, user)


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
    "/articles/{article_id}/access",
    get_article_access_route,
    methods=["GET"],
    response_model=ArticleAccessResponse,
)
router.add_api_route(
    "/articles/{article_id}/fulltext",
    redirect_article_fulltext_route,
    methods=["GET"],
)
