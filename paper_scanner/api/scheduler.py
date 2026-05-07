"""APScheduler integration for admin-managed shell tasks."""

from __future__ import annotations

import logging
import os
import subprocess
import time

from apscheduler.schedulers.background import BackgroundScheduler
from apscheduler.triggers.cron import CronTrigger

from paper_scanner.api.auth_db import (
    get_scheduled_task,
    list_scheduled_tasks,
    record_scheduled_task_run,
)
from paper_scanner.shared.runtime_config import apply_runtime_config

logger = logging.getLogger(__name__)

_scheduler: BackgroundScheduler | None = None


def validate_cron_expression(cron: str) -> None:
    """
    Validate a five-field crontab expression.

    Args:
        cron: Crontab expression.

    Returns:
        None.

    Raises:
        ValueError: If the cron expression is invalid.
    """
    CronTrigger.from_crontab(cron)


def _build_job_id(task_id: int) -> str:
    """
    Build a stable scheduler job identifier.

    Args:
        task_id: Scheduled task identifier.

    Returns:
        Job identifier string.
    """
    return f"scheduled-task-{task_id}"


def _execute_command(task_id: int, command: str) -> None:
    """
    Execute one scheduled shell command and store the outcome.

    Args:
        task_id: Scheduled task identifier.
        command: Shell command to execute.

    Returns:
        None.
    """
    ran_at = time.time()
    try:
        apply_runtime_config()
        result = subprocess.run(
            command,
            shell=True,
            capture_output=True,
            text=True,
            check=False,
            env=os.environ.copy(),
        )
    except Exception as exc:
        logger.exception("Scheduled task %s crashed", task_id)
        record_scheduled_task_run(task_id, f"error: {exc}", ran_at)
        return

    if result.returncode == 0:
        status = "success"
    else:
        status = f"failed ({result.returncode})"
        stderr = result.stderr.strip()
        if stderr:
            logger.error("Scheduled task %s failed: %s", task_id, stderr)

    record_scheduled_task_run(task_id, status, ran_at)


def reload_scheduler() -> None:
    """
    Reload scheduler jobs from the auth database.

    Returns:
        None.
    """
    if _scheduler is None:
        return

    _scheduler.remove_all_jobs()

    for task in list_scheduled_tasks():
        if not task["enabled"]:
            continue
        try:
            trigger = CronTrigger.from_crontab(str(task["cron"]))
        except ValueError:
            logger.exception(
                "Skipping scheduled task %s because the cron expression is invalid",
                task["id"],
            )
            continue

        _scheduler.add_job(
            _execute_command,
            trigger=trigger,
            args=[int(task["id"]), str(task["command"])],
            id=_build_job_id(int(task["id"])),
            replace_existing=True,
            max_instances=1,
            coalesce=True,
        )


def run_task_now(task_id: int) -> bool:
    """
    Execute a scheduled task immediately in the current process.

    Args:
        task_id: Scheduled task identifier.

    Returns:
        True when the task exists.
    """
    task = get_scheduled_task(task_id)
    if task is None:
        return False
    _execute_command(task_id, str(task["command"]))
    return True


def start_scheduler() -> None:
    """
    Start the background scheduler and load current jobs.

    Returns:
        None.
    """
    global _scheduler

    if _scheduler is not None:
        return

    _scheduler = BackgroundScheduler()
    _scheduler.start()
    reload_scheduler()


def stop_scheduler() -> None:
    """
    Stop the background scheduler if it is running.

    Returns:
        None.
    """
    global _scheduler

    if _scheduler is None:
        return

    _scheduler.shutdown(wait=False)
    _scheduler = None
