"""Index run statistics for source paths and upstream API calls."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any
from urllib.parse import urlsplit

ERROR_SAMPLE_LIMIT = 5
ERROR_SAMPLE_LENGTH = 500
RATE_LIMIT_STATUS_CODES = {429}
SECRET_PARAMETER_RE = re.compile(
    r"(?i)\b(api[_-]?key|x-api-key|token|secret|password|proxy)=([^&\s]+)"
)
URL_RE = re.compile(r"https?://[^\s'\"<>]+")


def utc_now_iso() -> str:
    """
    Return the current UTC timestamp as an ISO string.

    Returns:
        Current UTC timestamp.
    """
    return datetime.now(UTC).isoformat()


def sanitize_url_path(url: str | None) -> str:
    """
    Convert a URL or path into a query-free path value.

    Args:
        url: Raw URL or path.

    Returns:
        Sanitized path without query parameters or fragments.
    """
    if not url:
        return ""
    text = str(url).strip()
    if not text:
        return ""
    parsed = urlsplit(text)
    if parsed.scheme or parsed.netloc:
        return parsed.path or "/"
    return text.split("?", maxsplit=1)[0].split("#", maxsplit=1)[0]


def sanitize_error_sample(error: BaseException | str | None) -> str | None:
    """
    Build a compact secret-free error sample.

    Args:
        error: Error object or text.

    Returns:
        Sanitized error sample or None.
    """
    if error is None:
        return None
    if isinstance(error, BaseException):
        prefix = type(error).__name__
        message = str(error)
        text = f"{prefix}: {message}" if message else prefix
    else:
        text = str(error)
    text = URL_RE.sub(_sanitize_url_match, text)
    text = SECRET_PARAMETER_RE.sub(r"\1=<redacted>", text)
    text = re.sub(r"\s+", " ", text).strip()
    if len(text) > ERROR_SAMPLE_LENGTH:
        return text[:ERROR_SAMPLE_LENGTH]
    return text


def _sanitize_url_match(match: re.Match[str]) -> str:
    """
    Sanitize one URL regex match.

    Args:
        match: URL regex match.

    Returns:
        Sanitized URL containing only scheme, host, and path.
    """
    parsed = urlsplit(match.group(0))
    if parsed.scheme and parsed.netloc:
        return f"{parsed.scheme}://{parsed.netloc}{parsed.path or '/'}"
    return sanitize_url_path(match.group(0))


@dataclass(frozen=True)
class ApiStatsKey:
    """Identify one aggregate API statistics bucket."""

    source: str
    service: str
    endpoint: str
    method: str
    url_path: str
    journal_id: int | None = None
    journal_title: str = ""

    def to_dict(self) -> dict[str, Any]:
        """
        Serialize the key to a dictionary.

        Returns:
            Serializable key dictionary.
        """
        return {
            "source": self.source,
            "service": self.service,
            "endpoint": self.endpoint,
            "method": self.method,
            "url_path": self.url_path,
            "journal_id": self.journal_id,
            "journal_title": self.journal_title,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> ApiStatsKey:
        """
        Deserialize an API statistics key.

        Args:
            payload: Serialized key dictionary.

        Returns:
            API statistics key.
        """
        journal_id = payload.get("journal_id")
        return cls(
            source=str(payload.get("source") or ""),
            service=str(payload.get("service") or ""),
            endpoint=str(payload.get("endpoint") or ""),
            method=str(payload.get("method") or ""),
            url_path=str(payload.get("url_path") or ""),
            journal_id=journal_id if isinstance(journal_id, int) else None,
            journal_title=str(payload.get("journal_title") or ""),
        )


@dataclass
class ApiCallStats:
    """Aggregate request statistics for one API bucket."""

    key: ApiStatsKey
    logical_calls: int = 0
    attempts: int = 0
    successes: int = 0
    failures: int = 0
    retry_count: int = 0
    status_codes: dict[int, int] = field(default_factory=dict)
    transport_errors: int = 0
    rate_limit_failures: int = 0
    total_latency_ms: int = 0
    error_samples: list[str] = field(default_factory=list)

    def record_logical_call(self) -> None:
        """
        Record one logical API call.

        Returns:
            None.
        """
        self.logical_calls += 1

    def record_attempt(
        self,
        status_code: int | None,
        did_succeed: bool,
        elapsed_ms: float = 0.0,
        error: BaseException | str | None = None,
        did_retry: bool = False,
    ) -> None:
        """
        Record one HTTP attempt for this API bucket.

        Args:
            status_code: HTTP status code when available.
            did_succeed: Whether the attempt succeeded.
            elapsed_ms: Attempt latency in milliseconds.
            error: Attempt error when available.
            did_retry: Whether a retry followed or preceded this attempt.

        Returns:
            None.
        """
        self.attempts += 1
        self.total_latency_ms += max(0, int(round(elapsed_ms)))
        if did_retry:
            self.retry_count += 1
        if status_code is not None:
            self.status_codes[status_code] = self.status_codes.get(status_code, 0) + 1
            if status_code in RATE_LIMIT_STATUS_CODES and not did_succeed:
                self.rate_limit_failures += 1
        if did_succeed:
            self.successes += 1
            return
        self.failures += 1
        if status_code is None:
            self.transport_errors += 1
        sample = sanitize_error_sample(error)
        if sample and sample not in self.error_samples:
            self.error_samples.append(sample)
            del self.error_samples[ERROR_SAMPLE_LIMIT:]

    def merge(self, other: ApiCallStats) -> None:
        """
        Merge another API statistics bucket into this bucket.

        Args:
            other: API statistics bucket to merge.

        Returns:
            None.
        """
        self.logical_calls += other.logical_calls
        self.attempts += other.attempts
        self.successes += other.successes
        self.failures += other.failures
        self.retry_count += other.retry_count
        self.transport_errors += other.transport_errors
        self.rate_limit_failures += other.rate_limit_failures
        self.total_latency_ms += other.total_latency_ms
        for status_code, count in other.status_codes.items():
            self.status_codes[status_code] = (
                self.status_codes.get(status_code, 0) + count
            )
        for sample in other.error_samples:
            if sample not in self.error_samples:
                self.error_samples.append(sample)
                del self.error_samples[ERROR_SAMPLE_LIMIT:]

    def to_dict(self) -> dict[str, Any]:
        """
        Serialize the API statistics bucket.

        Returns:
            Serializable API statistics dictionary.
        """
        return {
            "key": self.key.to_dict(),
            "logical_calls": self.logical_calls,
            "attempts": self.attempts,
            "successes": self.successes,
            "failures": self.failures,
            "retry_count": self.retry_count,
            "status_codes": {
                str(key): value for key, value in self.status_codes.items()
            },
            "transport_errors": self.transport_errors,
            "rate_limit_failures": self.rate_limit_failures,
            "total_latency_ms": self.total_latency_ms,
            "error_samples": list(self.error_samples),
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> ApiCallStats:
        """
        Deserialize an API statistics bucket.

        Args:
            payload: Serialized API statistics dictionary.

        Returns:
            API statistics bucket.
        """
        status_codes_payload = payload.get("status_codes") or {}
        status_codes = {
            int(status_code): int(count)
            for status_code, count in dict(status_codes_payload).items()
        }
        stats = cls(key=ApiStatsKey.from_dict(dict(payload.get("key") or {})))
        stats.logical_calls = int(payload.get("logical_calls") or 0)
        stats.attempts = int(payload.get("attempts") or 0)
        stats.successes = int(payload.get("successes") or 0)
        stats.failures = int(payload.get("failures") or 0)
        stats.retry_count = int(payload.get("retry_count") or 0)
        stats.status_codes = status_codes
        stats.transport_errors = int(payload.get("transport_errors") or 0)
        stats.rate_limit_failures = int(payload.get("rate_limit_failures") or 0)
        stats.total_latency_ms = int(payload.get("total_latency_ms") or 0)
        stats.error_samples = [
            str(sample) for sample in list(payload.get("error_samples") or [])
        ][:ERROR_SAMPLE_LIMIT]
        return stats

    def to_row(self, run_id: str) -> tuple[Any, ...]:
        """
        Convert the API statistics bucket into a database row.

        Args:
            run_id: Index run identifier.

        Returns:
            Database row tuple.
        """
        status_codes_json = json.dumps(
            {str(key): value for key, value in sorted(self.status_codes.items())},
            sort_keys=True,
        )
        error_samples_json = json.dumps(self.error_samples, ensure_ascii=False)
        return (
            run_id,
            self.key.source,
            self.key.service,
            self.key.endpoint,
            self.key.method,
            self.key.url_path,
            self.key.journal_id,
            self.key.journal_title,
            self.logical_calls,
            self.attempts,
            self.successes,
            self.failures,
            self.retry_count,
            status_codes_json,
            self.transport_errors,
            self.rate_limit_failures,
            self.total_latency_ms,
            error_samples_json,
        )


@dataclass(frozen=True)
class PathStatsKey:
    """Identify one aggregate source path statistics bucket."""

    source: str
    path: str
    journal_id: int | None = None
    journal_title: str = ""

    def to_dict(self) -> dict[str, Any]:
        """
        Serialize the key to a dictionary.

        Returns:
            Serializable path key dictionary.
        """
        return {
            "source": self.source,
            "path": self.path,
            "journal_id": self.journal_id,
            "journal_title": self.journal_title,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> PathStatsKey:
        """
        Deserialize a path statistics key.

        Args:
            payload: Serialized key dictionary.

        Returns:
            Path statistics key.
        """
        journal_id = payload.get("journal_id")
        return cls(
            source=str(payload.get("source") or ""),
            path=str(payload.get("path") or ""),
            journal_id=journal_id if isinstance(journal_id, int) else None,
            journal_title=str(payload.get("journal_title") or ""),
        )


@dataclass
class PathCallStats:
    """Aggregate journal path statistics."""

    key: PathStatsKey
    status: str = "started"
    started_at: str = field(default_factory=utc_now_iso)
    finished_at: str | None = None
    works_count: int = 0
    issues_count: int = 0
    article_summaries_count: int = 0
    article_details_count: int = 0
    articles_written_count: int = 0
    articles_deleted_no_authors_count: int = 0
    error_type: str | None = None
    error_message: str | None = None

    def add_counts(
        self,
        works_count: int = 0,
        issues_count: int = 0,
        article_summaries_count: int = 0,
        article_details_count: int = 0,
        articles_written_count: int = 0,
        articles_deleted_no_authors_count: int = 0,
    ) -> None:
        """
        Add observed path counters.

        Args:
            works_count: Number of scholarly works observed.
            issues_count: Number of issues observed.
            article_summaries_count: Number of article summaries observed.
            article_details_count: Number of article details observed.
            articles_written_count: Number of article records written.
            articles_deleted_no_authors_count: Number of no-author articles deleted.

        Returns:
            None.
        """
        self.works_count += works_count
        self.issues_count += issues_count
        self.article_summaries_count += article_summaries_count
        self.article_details_count += article_details_count
        self.articles_written_count += articles_written_count
        self.articles_deleted_no_authors_count += articles_deleted_no_authors_count

    def finish(
        self,
        status: str,
        error: BaseException | str | None = None,
        finished_at: str | None = None,
    ) -> None:
        """
        Mark the path bucket as finished.

        Args:
            status: Final path status.
            error: Optional failure error.
            finished_at: Optional finish timestamp.

        Returns:
            None.
        """
        self.status = status
        self.finished_at = finished_at or utc_now_iso()
        if error is None:
            return
        self.error_type = (
            type(error).__name__ if isinstance(error, BaseException) else "Error"
        )
        self.error_message = sanitize_error_sample(error)

    def merge(self, other: PathCallStats) -> None:
        """
        Merge another path statistics bucket into this bucket.

        Args:
            other: Path statistics bucket to merge.

        Returns:
            None.
        """
        self.status = _merged_path_status(self.status, other.status)
        self.started_at = min(self.started_at, other.started_at)
        if other.finished_at and (
            self.finished_at is None or other.finished_at > self.finished_at
        ):
            self.finished_at = other.finished_at
        self.add_counts(
            works_count=other.works_count,
            issues_count=other.issues_count,
            article_summaries_count=other.article_summaries_count,
            article_details_count=other.article_details_count,
            articles_written_count=other.articles_written_count,
            articles_deleted_no_authors_count=other.articles_deleted_no_authors_count,
        )
        if other.error_type:
            self.error_type = self.error_type or other.error_type
        if other.error_message:
            self.error_message = self.error_message or other.error_message

    def to_dict(self) -> dict[str, Any]:
        """
        Serialize the path statistics bucket.

        Returns:
            Serializable path statistics dictionary.
        """
        return {
            "key": self.key.to_dict(),
            "status": self.status,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "works_count": self.works_count,
            "issues_count": self.issues_count,
            "article_summaries_count": self.article_summaries_count,
            "article_details_count": self.article_details_count,
            "articles_written_count": self.articles_written_count,
            "articles_deleted_no_authors_count": self.articles_deleted_no_authors_count,
            "error_type": self.error_type,
            "error_message": self.error_message,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> PathCallStats:
        """
        Deserialize a path statistics bucket.

        Args:
            payload: Serialized path statistics dictionary.

        Returns:
            Path statistics bucket.
        """
        stats = cls(key=PathStatsKey.from_dict(dict(payload.get("key") or {})))
        stats.status = str(payload.get("status") or "started")
        stats.started_at = str(payload.get("started_at") or utc_now_iso())
        finished_at = payload.get("finished_at")
        stats.finished_at = str(finished_at) if finished_at else None
        stats.works_count = int(payload.get("works_count") or 0)
        stats.issues_count = int(payload.get("issues_count") or 0)
        stats.article_summaries_count = int(payload.get("article_summaries_count") or 0)
        stats.article_details_count = int(payload.get("article_details_count") or 0)
        stats.articles_written_count = int(payload.get("articles_written_count") or 0)
        stats.articles_deleted_no_authors_count = int(
            payload.get("articles_deleted_no_authors_count") or 0
        )
        error_type = payload.get("error_type")
        error_message = payload.get("error_message")
        stats.error_type = str(error_type) if error_type else None
        stats.error_message = str(error_message) if error_message else None
        return stats

    def to_row(self, run_id: str) -> tuple[Any, ...]:
        """
        Convert the path statistics bucket into a database row.

        Args:
            run_id: Index run identifier.

        Returns:
            Database row tuple.
        """
        return (
            run_id,
            self.key.source,
            self.key.path,
            self.key.journal_id,
            self.key.journal_title,
            self.status,
            self.started_at,
            self.finished_at,
            self.works_count,
            self.issues_count,
            self.article_summaries_count,
            self.article_details_count,
            self.articles_written_count,
            self.articles_deleted_no_authors_count,
            self.error_type,
            self.error_message,
        )


@dataclass
class IndexRunStats:
    """Aggregate statistics for one index run."""

    run_id: str
    csv_file: str
    started_at: str = field(default_factory=utc_now_iso)
    finished_at: str | None = None
    status: str = "running"
    error_summary: str | None = None
    path_stats: dict[PathStatsKey, PathCallStats] = field(default_factory=dict)
    api_stats: dict[ApiStatsKey, ApiCallStats] = field(default_factory=dict)

    def finish(
        self,
        status: str,
        error_summary: str | None = None,
        finished_at: str | None = None,
    ) -> None:
        """
        Mark the run as finished.

        Args:
            status: Final run status.
            error_summary: Optional error summary.
            finished_at: Optional finish timestamp.

        Returns:
            None.
        """
        self.status = status
        self.error_summary = error_summary
        self.finished_at = finished_at or utc_now_iso()

    def merge(self, other: IndexRunStats) -> None:
        """
        Merge another run statistics object into this one.

        Args:
            other: Run statistics to merge.

        Returns:
            None.
        """
        for key, path_stats in other.path_stats.items():
            if key not in self.path_stats:
                self.path_stats[key] = PathCallStats.from_dict(path_stats.to_dict())
                continue
            self.path_stats[key].merge(path_stats)
        for key, api_stats in other.api_stats.items():
            if key not in self.api_stats:
                self.api_stats[key] = ApiCallStats.from_dict(api_stats.to_dict())
                continue
            self.api_stats[key].merge(api_stats)
        if other.error_summary and not self.error_summary:
            self.error_summary = other.error_summary

    def total_journals(self) -> int:
        """
        Count observed journal path buckets.

        Returns:
            Number of journal path buckets.
        """
        return len(self.path_stats)

    def succeeded_journals(self) -> int:
        """
        Count succeeded journal path buckets.

        Returns:
            Number of succeeded journals.
        """
        return sum(
            1 for stats in self.path_stats.values() if stats.status == "succeeded"
        )

    def failed_journals(self) -> int:
        """
        Count failed journal path buckets.

        Returns:
            Number of failed journals.
        """
        return sum(1 for stats in self.path_stats.values() if stats.status == "failed")

    def resumed_journals(self) -> int:
        """
        Count resumed journal path buckets.

        Returns:
            Number of resumed journals.
        """
        return sum(1 for stats in self.path_stats.values() if stats.status == "resumed")

    def to_dict(self) -> dict[str, Any]:
        """
        Serialize the run statistics.

        Returns:
            Serializable run statistics dictionary.
        """
        return {
            "run_id": self.run_id,
            "csv_file": self.csv_file,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "status": self.status,
            "error_summary": self.error_summary,
            "path_stats": [stats.to_dict() for stats in self.path_stats.values()],
            "api_stats": [stats.to_dict() for stats in self.api_stats.values()],
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> IndexRunStats:
        """
        Deserialize run statistics.

        Args:
            payload: Serialized run statistics dictionary.

        Returns:
            Run statistics object.
        """
        stats = cls(
            run_id=str(payload.get("run_id") or ""),
            csv_file=str(payload.get("csv_file") or ""),
            started_at=str(payload.get("started_at") or utc_now_iso()),
        )
        finished_at = payload.get("finished_at")
        stats.finished_at = str(finished_at) if finished_at else None
        stats.status = str(payload.get("status") or "running")
        error_summary = payload.get("error_summary")
        stats.error_summary = str(error_summary) if error_summary else None
        for path_payload in list(payload.get("path_stats") or []):
            path_stats = PathCallStats.from_dict(dict(path_payload))
            stats.path_stats[path_stats.key] = path_stats
        for api_payload in list(payload.get("api_stats") or []):
            api_stats = ApiCallStats.from_dict(dict(api_payload))
            stats.api_stats[api_stats.key] = api_stats
        return stats

    def to_run_row(self) -> tuple[Any, ...]:
        """
        Convert the run statistics into a database row.

        Returns:
            Database row tuple.
        """
        return (
            self.run_id,
            self.csv_file,
            self.started_at,
            self.finished_at,
            self.status,
            self.total_journals(),
            self.succeeded_journals(),
            self.failed_journals(),
            self.resumed_journals(),
            self.error_summary,
        )

    def path_rows(self) -> list[tuple[Any, ...]]:
        """
        Convert path statistics into database rows.

        Returns:
            Path statistics database rows.
        """
        return [stats.to_row(self.run_id) for stats in self.path_stats.values()]

    def api_rows(self) -> list[tuple[Any, ...]]:
        """
        Convert API statistics into database rows.

        Returns:
            API statistics database rows.
        """
        return [stats.to_row(self.run_id) for stats in self.api_stats.values()]


class IndexStatsRecorder:
    """Record index path and API statistics for one run."""

    def __init__(
        self,
        run_id: str,
        csv_file: str,
        started_at: str | None = None,
    ) -> None:
        """
        Initialize the recorder.

        Args:
            run_id: Index run identifier.
            csv_file: Source CSV file name.
            started_at: Optional run start timestamp.
        """
        self.stats = IndexRunStats(
            run_id=run_id,
            csv_file=csv_file,
            started_at=started_at or utc_now_iso(),
        )
        self._current_path_key: PathStatsKey | None = None

    def set_current_path(
        self,
        source: str,
        path: str,
        journal_id: int | None = None,
        journal_title: str | None = None,
    ) -> None:
        """
        Set the current source path context for API calls.

        Args:
            source: Source identifier.
            path: Source path label.
            journal_id: Optional journal identifier.
            journal_title: Optional journal title.

        Returns:
            None.
        """
        self._current_path_key = PathStatsKey(
            source=source,
            path=path,
            journal_id=journal_id,
            journal_title=journal_title or "",
        )

    def clear_current_path(self) -> None:
        """
        Clear the current source path context.

        Returns:
            None.
        """
        self._current_path_key = None

    def record_path_started(
        self,
        source: str,
        path: str,
        journal_id: int | None = None,
        journal_title: str | None = None,
    ) -> PathStatsKey:
        """
        Record a started source path.

        Args:
            source: Source identifier.
            path: Source path label.
            journal_id: Optional journal identifier.
            journal_title: Optional journal title.

        Returns:
            Path statistics key.
        """
        key = PathStatsKey(source, path, journal_id, journal_title or "")
        self.path_stats_for_key(key)
        self._current_path_key = key
        return key

    def record_path_counts(
        self,
        key: PathStatsKey | None = None,
        works_count: int = 0,
        issues_count: int = 0,
        article_summaries_count: int = 0,
        article_details_count: int = 0,
        articles_written_count: int = 0,
        articles_deleted_no_authors_count: int = 0,
    ) -> None:
        """
        Add counters to a source path.

        Args:
            key: Optional path key.
            works_count: Number of scholarly works observed.
            issues_count: Number of issues observed.
            article_summaries_count: Number of article summaries observed.
            article_details_count: Number of article details observed.
            articles_written_count: Number of article records written.
            articles_deleted_no_authors_count: Number of no-author articles deleted.

        Returns:
            None.
        """
        path_stats = self.path_stats_for_key(key or self.required_current_path_key())
        path_stats.add_counts(
            works_count=works_count,
            issues_count=issues_count,
            article_summaries_count=article_summaries_count,
            article_details_count=article_details_count,
            articles_written_count=articles_written_count,
            articles_deleted_no_authors_count=articles_deleted_no_authors_count,
        )

    def record_path_finished(
        self,
        status: str,
        key: PathStatsKey | None = None,
        error: BaseException | str | None = None,
    ) -> None:
        """
        Record a final source path outcome.

        Args:
            status: Final path status.
            key: Optional path key.
            error: Optional path error.

        Returns:
            None.
        """
        path_stats = self.path_stats_for_key(key or self.required_current_path_key())
        path_stats.finish(status, error=error)

    def record_api_call(
        self,
        service: str,
        endpoint: str,
        method: str,
        url: str,
        source: str | None = None,
        journal_id: int | None = None,
        journal_title: str | None = None,
    ) -> ApiStatsKey:
        """
        Record one logical API call and return its aggregate key.

        Args:
            service: Upstream service name.
            endpoint: Endpoint label.
            method: HTTP method.
            url: Request URL.
            source: Optional source identifier override.
            journal_id: Optional journal identifier override.
            journal_title: Optional journal title override.

        Returns:
            API statistics key.
        """
        context = self._current_path_key
        key = ApiStatsKey(
            source=source or (context.source if context else ""),
            service=service,
            endpoint=endpoint,
            method=method.upper(),
            url_path=sanitize_url_path(url),
            journal_id=journal_id
            if journal_id is not None
            else (context.journal_id if context else None),
            journal_title=journal_title
            if journal_title is not None
            else (context.journal_title if context else ""),
        )
        stats = self.api_stats_for_key(key)
        stats.record_logical_call()
        return key

    def record_api_attempt(
        self,
        key: ApiStatsKey,
        status_code: int | None,
        did_succeed: bool,
        elapsed_ms: float = 0.0,
        error: BaseException | str | None = None,
        did_retry: bool = False,
    ) -> None:
        """
        Record one API attempt for a logical call.

        Args:
            key: API statistics key.
            status_code: HTTP status code when available.
            did_succeed: Whether the attempt succeeded.
            elapsed_ms: Attempt latency in milliseconds.
            error: Attempt error when available.
            did_retry: Whether a retry followed or preceded this attempt.

        Returns:
            None.
        """
        self.api_stats_for_key(key).record_attempt(
            status_code=status_code,
            did_succeed=did_succeed,
            elapsed_ms=elapsed_ms,
            error=error,
            did_retry=did_retry,
        )

    def path_stats_for_key(self, key: PathStatsKey) -> PathCallStats:
        """
        Fetch or create a path statistics bucket.

        Args:
            key: Path statistics key.

        Returns:
            Path statistics bucket.
        """
        if key not in self.stats.path_stats:
            self.stats.path_stats[key] = PathCallStats(key=key)
        return self.stats.path_stats[key]

    def api_stats_for_key(self, key: ApiStatsKey) -> ApiCallStats:
        """
        Fetch or create an API statistics bucket.

        Args:
            key: API statistics key.

        Returns:
            API statistics bucket.
        """
        if key not in self.stats.api_stats:
            self.stats.api_stats[key] = ApiCallStats(key=key)
        return self.stats.api_stats[key]

    def required_current_path_key(self) -> PathStatsKey:
        """
        Return the current path key or raise when none exists.

        Returns:
            Current path statistics key.

        Raises:
            RuntimeError: If no source path context is active.
        """
        if self._current_path_key is None:
            raise RuntimeError("No current index path is active")
        return self._current_path_key

    def merge(self, other: IndexRunStats | dict[str, Any]) -> None:
        """
        Merge another stats object or serialized payload into this recorder.

        Args:
            other: Run statistics object or serialized payload.

        Returns:
            None.
        """
        other_stats = (
            other
            if isinstance(other, IndexRunStats)
            else IndexRunStats.from_dict(other)
        )
        self.stats.merge(other_stats)

    def to_dict(self) -> dict[str, Any]:
        """
        Serialize the recorder statistics.

        Returns:
            Serializable run statistics dictionary.
        """
        return self.stats.to_dict()


class NoOpIndexStatsRecorder:
    """Ignore index path and API statistics calls."""

    def set_current_path(
        self,
        source: str,
        path: str,
        journal_id: int | None = None,
        journal_title: str | None = None,
    ) -> None:
        """
        Ignore a source path context update.

        Args:
            source: Source identifier.
            path: Source path label.
            journal_id: Optional journal identifier.
            journal_title: Optional journal title.

        Returns:
            None.
        """

    def clear_current_path(self) -> None:
        """
        Ignore clearing the source path context.

        Returns:
            None.
        """

    def record_path_started(
        self,
        source: str,
        path: str,
        journal_id: int | None = None,
        journal_title: str | None = None,
    ) -> PathStatsKey:
        """
        Return a path key without recording it.

        Args:
            source: Source identifier.
            path: Source path label.
            journal_id: Optional journal identifier.
            journal_title: Optional journal title.

        Returns:
            Path statistics key.
        """
        return PathStatsKey(source, path, journal_id, journal_title or "")

    def record_path_counts(
        self,
        key: PathStatsKey | None = None,
        works_count: int = 0,
        issues_count: int = 0,
        article_summaries_count: int = 0,
        article_details_count: int = 0,
        articles_written_count: int = 0,
        articles_deleted_no_authors_count: int = 0,
    ) -> None:
        """
        Ignore source path counters.

        Args:
            key: Optional path key.
            works_count: Number of scholarly works observed.
            issues_count: Number of issues observed.
            article_summaries_count: Number of article summaries observed.
            article_details_count: Number of article details observed.
            articles_written_count: Number of article records written.
            articles_deleted_no_authors_count: Number of no-author articles deleted.

        Returns:
            None.
        """

    def record_path_finished(
        self,
        status: str,
        key: PathStatsKey | None = None,
        error: BaseException | str | None = None,
    ) -> None:
        """
        Ignore a final source path outcome.

        Args:
            status: Final path status.
            key: Optional path key.
            error: Optional path error.

        Returns:
            None.
        """

    def record_api_call(
        self,
        service: str,
        endpoint: str,
        method: str,
        url: str,
        source: str | None = None,
        journal_id: int | None = None,
        journal_title: str | None = None,
    ) -> ApiStatsKey:
        """
        Return an API key without recording it.

        Args:
            service: Upstream service name.
            endpoint: Endpoint label.
            method: HTTP method.
            url: Request URL.
            source: Optional source identifier.
            journal_id: Optional journal identifier.
            journal_title: Optional journal title.

        Returns:
            API statistics key.
        """
        return ApiStatsKey(
            source=source or "",
            service=service,
            endpoint=endpoint,
            method=method.upper(),
            url_path=sanitize_url_path(url),
            journal_id=journal_id,
            journal_title=journal_title or "",
        )

    def record_api_attempt(
        self,
        key: ApiStatsKey,
        status_code: int | None,
        did_succeed: bool,
        elapsed_ms: float = 0.0,
        error: BaseException | str | None = None,
        did_retry: bool = False,
    ) -> None:
        """
        Ignore an API attempt.

        Args:
            key: API statistics key.
            status_code: HTTP status code when available.
            did_succeed: Whether the attempt succeeded.
            elapsed_ms: Attempt latency in milliseconds.
            error: Attempt error when available.
            did_retry: Whether a retry followed or preceded this attempt.

        Returns:
            None.
        """


def _merged_path_status(first_status: str, second_status: str) -> str:
    """
    Merge two path statuses by severity.

    Args:
        first_status: Existing status.
        second_status: Incoming status.

    Returns:
        Merged status.
    """
    priority = {"failed": 4, "succeeded": 3, "resumed": 2, "started": 1}
    if priority.get(second_status, 0) > priority.get(first_status, 0):
        return second_status
    return first_status
