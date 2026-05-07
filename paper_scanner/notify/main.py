"""CLI entrypoint for notification command."""

from __future__ import annotations

import argparse

from paper_scanner.notify.models import DEFAULT_STATE_DIR
from paper_scanner.notify.workflow import run_notification
from paper_scanner.shared.constants import PROJECT_ROOT


def build_parser() -> argparse.ArgumentParser:
    """
    Build CLI parser.

    Args:
        None.

    Returns:
        Argument parser.
    """
    parser = argparse.ArgumentParser(
        description=(
            "Select and push updated articles with "
            "an OpenAI-compatible model and PushPlus"
        )
    )
    parser.add_argument(
        "--db",
        type=str,
        default=None,
        help=(
            "Database file under data/index. "
            "When omitted, notification processes all sqlite files."
        ),
    )
    parser.add_argument(
        "--state-dir",
        type=str,
        default=str(DEFAULT_STATE_DIR.relative_to(PROJECT_ROOT)),
        help="Directory for persisted push state files.",
    )
    parser.add_argument(
        "--changes-file",
        type=str,
        default="",
        help=(
            "Optional change manifest from index update. "
            "When provided, notification uses this exact change set."
        ),
    )
    parser.add_argument(
        "--ai-model",
        type=str,
        default="",
        help="Override OpenAI-compatible model id.",
    )
    parser.add_argument(
        "--max-candidates",
        type=int,
        default=0,
        help="Maximum candidates sent to model per run. 0 uses config default.",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=60,
        help="HTTP timeout in seconds.",
    )
    parser.add_argument(
        "--retries",
        type=int,
        default=3,
        help="Retry count for AI and PushPlus calls.",
    )
    parser.add_argument(
        "--dedupe-retention-days",
        type=int,
        default=60,
        help="Days to keep delivery dedupe records.",
    )
    parser.add_argument(
        "--dry-run",
        action=argparse.BooleanOptionalAction,
        default=False,
        help="Run selection without sending PushPlus messages.",
    )
    return parser


def main() -> None:
    """
    Parse CLI arguments and run notification pipeline.

    Args:
        None.

    Returns:
        None.
    """
    parser = build_parser()
    args = parser.parse_args()
    raise SystemExit(run_notification(args))


if __name__ == "__main__":
    main()
