"""Route package registration."""

from __future__ import annotations

from fastapi import FastAPI

from scripts.api.routes import (
    admin,
    articles,
    auth,
    favorites,
    health,
    issues,
    journals,
    meta,
    tracking,
    weekly,
)


def register_routes(app: FastAPI) -> None:
    """
    Register all API routers on the application instance.

    Args:
        app: FastAPI application.

    Returns:
        None.
    """
    routers = (
        health.router,
        meta.router,
        journals.router,
        issues.router,
        articles.router,
        weekly.router,
        auth.router,
        favorites.router,
        tracking.router,
        admin.router,
    )
    for router in routers:
        app.include_router(router)
