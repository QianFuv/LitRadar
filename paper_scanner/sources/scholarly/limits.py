"""Source-aware request throttles for scholarly metadata services."""

from __future__ import annotations

import asyncio
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TypeVar

CROSSREF_SOURCE = "crossref"
OPENALEX_SOURCE = "openalex"
SEMANTIC_SCHOLAR_SOURCE = "semantic_scholar"
OPENALEX_MIN_INTERVAL_SECONDS = 1.0
SEMANTIC_SCHOLAR_MIN_INTERVAL_SECONDS = 1.0

RequestResult = TypeVar("RequestResult")
SleepFunction = Callable[[float], Awaitable[None]]
ClockFunction = Callable[[], float]


@dataclass(frozen=True)
class SourceThrottleConfig:
    """
    Describe request pacing for one upstream metadata source.

    Args:
        source: Source identifier.
        max_concurrency: Maximum in-flight requests for this source in one process.
        min_interval_seconds: Base interval between request starts.
        worker_id: Current worker process identifier.
        process_count: Total process count sharing the same upstream limit.
    """

    source: str
    max_concurrency: int = 1
    min_interval_seconds: float = 0.0
    worker_id: int = 0
    process_count: int = 1

    @property
    def normalized_process_count(self) -> int:
        """
        Return a positive process count.

        Returns:
            Process count clamped to at least one.
        """
        return max(1, self.process_count)

    @property
    def effective_min_interval_seconds(self) -> float:
        """
        Return the per-process interval for a shared upstream limit.

        Returns:
            Base interval multiplied by process count.
        """
        return max(0.0, self.min_interval_seconds) * self.normalized_process_count

    @property
    def startup_delay_seconds(self) -> float:
        """
        Return the initial offset for this worker.

        Returns:
            Worker offset in seconds.
        """
        worker_index = max(0, self.worker_id) % self.normalized_process_count
        return max(0.0, self.min_interval_seconds) * worker_index


class SourceRequestThrottle:
    """
    Gate request starts and in-flight concurrency for one upstream source.

    Args:
        config: Source throttle configuration.
        clock: Monotonic clock function.
        sleep: Async sleep function.
    """

    def __init__(
        self,
        config: SourceThrottleConfig,
        *,
        clock: ClockFunction = time.monotonic,
        sleep: SleepFunction = asyncio.sleep,
    ) -> None:
        self.config = config
        self._clock = clock
        self._sleep = sleep
        self._lock = asyncio.Lock()
        self._semaphore = asyncio.Semaphore(max(1, config.max_concurrency))
        self._next_available_at = self._clock() + config.startup_delay_seconds

    async def run(
        self,
        operation: Callable[[], Awaitable[RequestResult]],
    ) -> RequestResult:
        """
        Run one request operation after respecting source limits.

        Args:
            operation: Awaitable request factory.

        Returns:
            Operation result.
        """
        async with self._semaphore:
            await self._wait_for_turn()
            return await operation()

    async def _wait_for_turn(self) -> None:
        """
        Wait until this source can start another request.

        Returns:
            None.
        """
        async with self._lock:
            delay = self._next_available_at - self._clock()
            if delay > 0:
                await self._sleep(delay)
            current_time = self._clock()
            interval = self.config.effective_min_interval_seconds
            self._next_available_at = max(current_time, self._next_available_at)
            self._next_available_at += interval


class ScholarlyRequestThrottles:
    """
    Dispatch request operations through source-specific throttles.

    Args:
        configs: Source throttle configurations.
        clock: Monotonic clock function.
        sleep: Async sleep function.
    """

    def __init__(
        self,
        configs: list[SourceThrottleConfig],
        *,
        clock: ClockFunction = time.monotonic,
        sleep: SleepFunction = asyncio.sleep,
    ) -> None:
        self._throttles = {
            config.source: SourceRequestThrottle(
                config,
                clock=clock,
                sleep=sleep,
            )
            for config in configs
        }

    async def run(
        self,
        source: str | None,
        operation: Callable[[], Awaitable[RequestResult]],
    ) -> RequestResult:
        """
        Run one operation under the throttle for a source.

        Args:
            source: Source identifier or None for no throttling.
            operation: Awaitable operation factory.

        Returns:
            Operation result.
        """
        if source is None:
            return await operation()
        throttle = self._throttles.get(source)
        if throttle is None:
            return await operation()
        return await throttle.run(operation)

    def throttle_for_source(self, source: str) -> SourceRequestThrottle | None:
        """
        Return the throttle configured for a source.

        Args:
            source: Source identifier.

        Returns:
            Source throttle or None.
        """
        return self._throttles.get(source)


def build_scholarly_request_throttles(
    *,
    worker_id: int = 0,
    process_count: int = 1,
    clock: ClockFunction = time.monotonic,
    sleep: SleepFunction = asyncio.sleep,
) -> ScholarlyRequestThrottles:
    """
    Build default request throttles for scholarly upstream services.

    Args:
        worker_id: Current worker process identifier.
        process_count: Total process count sharing upstream limits.
        clock: Monotonic clock function.
        sleep: Async sleep function.

    Returns:
        Scholarly request throttle registry.
    """
    return ScholarlyRequestThrottles(
        [
            SourceThrottleConfig(
                source=CROSSREF_SOURCE,
                max_concurrency=1,
                worker_id=worker_id,
                process_count=process_count,
            ),
            SourceThrottleConfig(
                source=OPENALEX_SOURCE,
                max_concurrency=1,
                min_interval_seconds=OPENALEX_MIN_INTERVAL_SECONDS,
                worker_id=worker_id,
                process_count=process_count,
            ),
            SourceThrottleConfig(
                source=SEMANTIC_SCHOLAR_SOURCE,
                max_concurrency=1,
                min_interval_seconds=SEMANTIC_SCHOLAR_MIN_INTERVAL_SECONDS,
                worker_id=worker_id,
                process_count=process_count,
            ),
        ],
        clock=clock,
        sleep=sleep,
    )
