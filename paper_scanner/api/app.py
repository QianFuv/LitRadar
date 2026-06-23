"""FastAPI application factory."""

from __future__ import annotations

import os
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request

from paper_scanner.api.auth_db import init_auth_db
from paper_scanner.api.auth_deps import SESSION_COOKIE_NAME
from paper_scanner.api.scheduler import start_scheduler, stop_scheduler
from paper_scanner.shared.constants import API_PREFIX
from paper_scanner.shared.runtime_config import apply_runtime_config

CORS_ALLOWED_ORIGINS_ENV = "API_CORS_ALLOWED_ORIGINS"
AUTHENTICATED_CACHE_CONTROL = "private, no-store"
PUBLIC_INDEX_CACHE_CONTROL = "public, max-age=300, stale-while-revalidate=600"


def get_cors_allowed_origins() -> list[str]:
    """
    Return explicitly configured credentialed CORS origins.

    Returns:
        Allowed origins from the comma-separated environment value.
    """
    value = os.environ.get(CORS_ALLOWED_ORIGINS_ENV, "")
    return [origin.strip() for origin in value.split(",") if origin.strip()]


def _has_auth_credentials(request: Request) -> bool:
    """
    Check whether a request carries authentication credentials.

    Args:
        request: Incoming HTTP request.

    Returns:
        True when Authorization or the browser session cookie is present.
    """
    return bool(
        request.headers.get("authorization") or request.cookies.get(SESSION_COOKIE_NAME)
    )


def _is_public_index_cache_path(path: str) -> bool:
    """
    Check whether an unauthenticated path can use shared index caching.

    Args:
        path: Request path.

    Returns:
        True for article and metadata API paths.
    """
    return path.startswith(f"{API_PREFIX}/articles") or path.startswith(
        f"{API_PREFIX}/meta"
    )


class CacheControlMiddleware(BaseHTTPMiddleware):
    """
    Add cache control headers to API responses.
    """

    async def dispatch(self, request: Request, call_next):
        """
        Attach public or private cache headers after a route response is built.

        Args:
            request: Incoming HTTP request.
            call_next: Downstream ASGI request handler.

        Returns:
            Response with cache headers adjusted for credentialed requests.
        """
        response = await call_next(request)
        if _has_auth_credentials(request):
            response.headers["Cache-Control"] = AUTHENTICATED_CACHE_CONTROL
        elif _is_public_index_cache_path(request.url.path):
            response.headers["Cache-Control"] = PUBLIC_INDEX_CACHE_CONTROL
        return response


@asynccontextmanager
async def lifespan(application: FastAPI) -> AsyncGenerator[None]:
    apply_runtime_config()
    init_auth_db()
    apply_runtime_config()
    start_scheduler()
    try:
        yield
    finally:
        stop_scheduler()


def build_app() -> FastAPI:
    """
    Build and configure the FastAPI application.

    Returns:
        Configured FastAPI application.
    """
    application = FastAPI(title="Paper Scanner API", version="1.0.0", lifespan=lifespan)
    application.add_middleware(
        CORSMiddleware,
        allow_origins=get_cors_allowed_origins(),
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )
    application.add_middleware(CacheControlMiddleware)
    return application


app = build_app()
