"""FastAPI application factory."""

from __future__ import annotations

from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request

from scripts.api.auth_db import init_auth_db
from scripts.api.scheduler import start_scheduler, stop_scheduler
from scripts.shared.constants import API_PREFIX
from scripts.shared.runtime_config import apply_runtime_config


class CacheControlMiddleware(BaseHTTPMiddleware):
    """
    Add cache control headers to API responses.
    """

    async def dispatch(self, request: Request, call_next):
        response = await call_next(request)
        is_articles = request.url.path.startswith(f"{API_PREFIX}/articles")
        is_meta = request.url.path.startswith(f"{API_PREFIX}/meta")
        if is_articles or is_meta:
            response.headers["Cache-Control"] = (
                "public, max-age=300, stale-while-revalidate=600"
            )
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
        allow_origins=["*"],
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )
    application.add_middleware(CacheControlMiddleware)
    return application


app = build_app()
