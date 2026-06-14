"""Tests for scholarly source request throttles."""

from __future__ import annotations

import unittest

from paper_scanner.sources.scholarly.limits import (
    OPENALEX_SOURCE,
    SEMANTIC_SCHOLAR_SOURCE,
    build_scholarly_request_throttles,
)


class FakeClock:
    """
    Controllable monotonic clock for throttle tests.
    """

    def __init__(self, current_time: float = 100.0) -> None:
        """
        Initialize the fake clock.

        Args:
            current_time: Initial monotonic time.
        """
        self.current_time = current_time
        self.sleeps: list[float] = []

    def now(self) -> float:
        """
        Return the current fake monotonic time.

        Returns:
            Current fake time.
        """
        return self.current_time

    async def sleep(self, delay: float) -> None:
        """
        Advance fake time by the requested delay.

        Args:
            delay: Sleep delay in seconds.

        Returns:
            None.
        """
        self.sleeps.append(delay)
        self.current_time += delay


class ScholarlyRequestThrottlesTest(unittest.IsolatedAsyncioTestCase):
    """
    Verify source-specific scholarly request pacing.
    """

    async def test_semantic_scholar_spacing_uses_worker_offset(self) -> None:
        """
        Ensure S2 requests are staggered across worker processes.
        """
        clock = FakeClock()
        throttles = build_scholarly_request_throttles(
            worker_id=1,
            process_count=3,
            clock=clock.now,
            sleep=clock.sleep,
        )
        request_times: list[float] = []

        async def run_request() -> str:
            """
            Record the fake request start time.

            Returns:
                Request marker.
            """
            request_times.append(clock.now())
            return "ok"

        result = await throttles.run(SEMANTIC_SCHOLAR_SOURCE, run_request)
        second_result = await throttles.run(SEMANTIC_SCHOLAR_SOURCE, run_request)

        self.assertEqual(result, "ok")
        self.assertEqual(second_result, "ok")
        self.assertEqual(clock.sleeps, [1.0, 3.0])
        self.assertEqual(request_times, [101.0, 104.0])

    async def test_openalex_has_no_default_spacing(self) -> None:
        """
        Ensure OpenAlex keeps the current serial but unpaced behavior.
        """
        clock = FakeClock()
        throttles = build_scholarly_request_throttles(
            worker_id=0,
            process_count=4,
            clock=clock.now,
            sleep=clock.sleep,
        )
        request_times: list[float] = []

        async def run_request() -> str:
            """
            Record the fake request start time.

            Returns:
                Request marker.
            """
            request_times.append(clock.now())
            return "ok"

        await throttles.run(OPENALEX_SOURCE, run_request)
        await throttles.run(OPENALEX_SOURCE, run_request)

        self.assertEqual(clock.sleeps, [])
        self.assertEqual(request_times, [100.0, 100.0])

    async def test_unknown_source_runs_without_throttle(self) -> None:
        """
        Ensure unknown sources do not introduce request delays.
        """
        clock = FakeClock()
        throttles = build_scholarly_request_throttles(
            worker_id=2,
            process_count=3,
            clock=clock.now,
            sleep=clock.sleep,
        )

        async def run_request() -> str:
            """
            Return a request marker.

            Returns:
                Request marker.
            """
            return "ok"

        result = await throttles.run("unknown", run_request)

        self.assertEqual(result, "ok")
        self.assertEqual(clock.sleeps, [])

    def test_semantic_scholar_config_scales_interval_by_processes(self) -> None:
        """
        Ensure the S2 effective interval preserves the global request rate.
        """
        clock = FakeClock()
        throttles = build_scholarly_request_throttles(
            worker_id=2,
            process_count=4,
            clock=clock.now,
            sleep=clock.sleep,
        )
        throttle = throttles.throttle_for_source(SEMANTIC_SCHOLAR_SOURCE)

        self.assertIsNotNone(throttle)
        assert throttle is not None
        self.assertEqual(throttle.config.startup_delay_seconds, 2.0)
        self.assertEqual(throttle.config.effective_min_interval_seconds, 4.0)
