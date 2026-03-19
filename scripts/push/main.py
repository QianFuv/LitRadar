"""CLI entrypoint for tracking-folder push command."""

from __future__ import annotations

import argparse

from scripts.push.workflow import run_push
from scripts.shared.constants import PROJECT_ROOT

DEFAULT_STATE_DIR = PROJECT_ROOT / "data" / "folder_push_state"


def build_parser() -> argparse.ArgumentParser:
    """
    Build CLI parser.

    Returns:
        Argument parser.
    """
    parser = argparse.ArgumentParser(
        description="Select and deliver updated articles into tracking folders"
    )
    parser.add_argument(
        "--db",
        type=str,
        default=None,
        help="Database file under data/index. Defaults to the only sqlite file.",
    )
    parser.add_argument(
        "--state-dir",
        type=str,
        default=str(DEFAULT_STATE_DIR.relative_to(PROJECT_ROOT)),
        help="Directory for persisted tracking push state files.",
    )
    parser.add_argument(
        "--changes-file",
        type=str,
        default="",
        help=(
            "Optional change manifest from index update. "
            "When provided, tracking push uses this exact change set."
        ),
    )
    parser.add_argument(
        "--siliconflow-model",
        type=str,
        default="",
        help="Override SiliconFlow model id.",
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
        help="Retry count for SiliconFlow calls.",
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
        help="Run selection without writing favorites into tracking folders.",
    )
    return parser


def main() -> None:
    """
    Parse CLI arguments and run tracking-folder push pipeline.

    Returns:
        None.
    """
    parser = build_parser()
    args = parser.parse_args()
    raise SystemExit(run_push(args))


if __name__ == "__main__":
    main()
