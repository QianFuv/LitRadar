"""CLI entrypoint for API service."""

from __future__ import annotations

import os

import uvicorn

from paper_scanner.api.app import app as _app
from paper_scanner.api.routes import register_routes

register_routes(_app)

app = _app


def main() -> None:
    """
    Run the FastAPI application with Uvicorn.

    Returns:
        None.
    """
    uvicorn.run(
        "paper_scanner.api.main:app",
        host=os.environ.get("API_HOST", "127.0.0.1"),
        port=8000,
    )


if __name__ == "__main__":
    main()
