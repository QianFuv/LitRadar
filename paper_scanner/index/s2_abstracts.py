"""Semantic Scholar abstract backfill helpers for scholarly index batches."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any

from paper_scanner.index.db.client import DatabaseClient
from paper_scanner.index.db.schema import ARTICLE_LISTING_BATCH_SIZE
from paper_scanner.index.transforms import normalize_doi
from paper_scanner.shared.converters import chunked
from paper_scanner.sources.scholarly import ScholarlyClient
from paper_scanner.sources.scholarly.client import SEMANTIC_SCHOLAR_BATCH_SIZE


@dataclass(frozen=True)
class S2AbstractBackfillResult:
    """
    Summarize one Semantic Scholar abstract backfill attempt.

    Args:
        candidate_article_count: Number of article rows eligible for S2 lookup.
        candidate_doi_count: Number of unique DOI values eligible for S2 lookup.
        fetched_doi_count: Number of DOI values fetched from S2 by this call.
        matched_doi_count: Number of candidate DOI values with S2 payloads.
        abstract_doi_count: Number of candidate DOI values with S2 abstracts.
        attempted_article_count: Number of article rows targeted for abstract writes.
    """

    candidate_article_count: int = 0
    candidate_doi_count: int = 0
    fetched_doi_count: int = 0
    matched_doi_count: int = 0
    abstract_doi_count: int = 0
    attempted_article_count: int = 0


async def backfill_s2_abstracts_for_records(
    db: DatabaseClient,
    client: ScholarlyClient,
    records: Sequence[Mapping[str, Any]],
    *,
    semantic_scholar_by_doi: Mapping[str, Mapping[str, Any]] | None = None,
    batch_size: int = SEMANTIC_SCHOLAR_BATCH_SIZE,
) -> S2AbstractBackfillResult:
    """
    Backfill missing article abstracts for a just-written record batch.

    Args:
        db: Database client used by the active index worker.
        client: Scholarly client with coordinated S2 request throttles.
        records: Article records from the just-written batch.
        semantic_scholar_by_doi: Optional preloaded S2 payload map.
        batch_size: Maximum DOI count for one S2 request batch.

    Returns:
        Backfill summary counts.
    """
    doi_to_article_ids = _missing_abstract_article_ids_by_doi(records)
    return await backfill_s2_abstracts_for_doi_article_ids(
        db,
        client,
        doi_to_article_ids,
        semantic_scholar_by_doi=semantic_scholar_by_doi,
        batch_size=batch_size,
    )


async def backfill_s2_abstracts_for_doi_article_ids(
    db: DatabaseClient,
    client: ScholarlyClient,
    doi_to_article_ids: Mapping[str, Sequence[int]],
    *,
    semantic_scholar_by_doi: Mapping[str, Mapping[str, Any]] | None = None,
    batch_size: int = SEMANTIC_SCHOLAR_BATCH_SIZE,
) -> S2AbstractBackfillResult:
    """
    Backfill missing article abstracts for selected DOI-to-article mappings.

    Args:
        db: Database client used by the active index worker.
        client: Scholarly client with coordinated S2 request throttles.
        doi_to_article_ids: Mapping from DOI values to target article IDs.
        semantic_scholar_by_doi: Optional preloaded S2 payload map.
        batch_size: Maximum DOI count for one S2 request batch.

    Returns:
        Backfill summary counts.
    """
    normalized_doi_to_article_ids = _normalize_doi_to_article_ids(doi_to_article_ids)
    candidate_dois = list(normalized_doi_to_article_ids)
    candidate_article_count = sum(
        len(article_ids) for article_ids in normalized_doi_to_article_ids.values()
    )
    if not candidate_dois:
        return S2AbstractBackfillResult()

    s2_by_doi, fetched_doi_count = await _semantic_scholar_payloads_by_doi(
        client,
        candidate_dois,
        semantic_scholar_by_doi=semantic_scholar_by_doi,
        batch_size=batch_size,
    )
    abstracts_by_doi = {
        doi: abstract
        for doi in candidate_dois
        if (abstract := _semantic_scholar_abstract(s2_by_doi.get(doi)))
    }
    article_update_rows = [
        (abstract, article_id)
        for doi, abstract in abstracts_by_doi.items()
        for article_id in normalized_doi_to_article_ids[doi]
    ]
    if article_update_rows:
        await db.executemany(
            """
            UPDATE articles
            SET abstract = ?
            WHERE article_id = ?
                AND (abstract IS NULL OR TRIM(abstract) = '')
            """,
            article_update_rows,
        )
        await refresh_article_search_for_article_ids(
            db,
            [article_id for _, article_id in article_update_rows],
        )

    return S2AbstractBackfillResult(
        candidate_article_count=candidate_article_count,
        candidate_doi_count=len(candidate_dois),
        fetched_doi_count=fetched_doi_count,
        matched_doi_count=sum(1 for doi in candidate_dois if doi in s2_by_doi),
        abstract_doi_count=len(abstracts_by_doi),
        attempted_article_count=len(article_update_rows),
    )


async def refresh_article_search_for_article_ids(
    db: DatabaseClient,
    article_ids: Sequence[int],
) -> None:
    """
    Refresh FTS rows for article IDs after abstract-only updates.

    Args:
        db: Database client used by the active index worker.
        article_ids: Article IDs that may need refreshed search rows.

    Returns:
        None.
    """
    unique_article_ids = list(dict.fromkeys(article_ids))
    for batch in chunked(unique_article_ids, ARTICLE_LISTING_BATCH_SIZE):
        placeholders = ", ".join(["?"] * len(batch))
        await db.execute(
            f"""
            INSERT OR REPLACE INTO article_search (
                rowid,
                article_id,
                title,
                abstract,
                doi,
                authors,
                journal_title
            )
            SELECT
                a.article_id,
                a.article_id,
                COALESCE(a.title, ''),
                COALESCE(a.abstract, ''),
                COALESCE(a.doi, ''),
                COALESCE(a.authors, ''),
                COALESCE(j.title, '')
            FROM articles a
            LEFT JOIN journals j ON j.journal_id = a.journal_id
            WHERE a.article_id IN ({placeholders})
            """,
            tuple(batch),
        )


def _missing_abstract_article_ids_by_doi(
    records: Sequence[Mapping[str, Any]],
) -> dict[str, list[int]]:
    """
    Group missing-abstract article IDs by normalized DOI.

    Args:
        records: Article records from a write batch.

    Returns:
        Mapping from DOI to article IDs that still need S2 abstracts.
    """
    doi_to_article_ids: dict[str, list[int]] = {}
    for record in records:
        if _has_text(record.get("abstract")):
            continue
        article_id = record.get("article_id")
        if not isinstance(article_id, int):
            continue
        doi = normalize_doi(record.get("doi"))
        if not doi:
            continue
        doi_to_article_ids.setdefault(doi, []).append(article_id)
    return doi_to_article_ids


def _normalize_doi_to_article_ids(
    doi_to_article_ids: Mapping[str, Sequence[int]],
) -> dict[str, list[int]]:
    """
    Normalize DOI keys and article ID values.

    Args:
        doi_to_article_ids: Raw DOI-to-article mapping.

    Returns:
        Normalized DOI-to-article mapping.
    """
    normalized: dict[str, list[int]] = {}
    for raw_doi, raw_article_ids in doi_to_article_ids.items():
        doi = normalize_doi(raw_doi)
        if not doi:
            continue
        for article_id in raw_article_ids:
            if isinstance(article_id, int):
                normalized.setdefault(doi, []).append(article_id)
    return normalized


async def _semantic_scholar_payloads_by_doi(
    client: ScholarlyClient,
    dois: Sequence[str],
    *,
    semantic_scholar_by_doi: Mapping[str, Mapping[str, Any]] | None,
    batch_size: int,
) -> tuple[dict[str, Mapping[str, Any]], int]:
    """
    Return normalized S2 payloads, using preloaded data when supplied.

    Args:
        client: Scholarly client with coordinated S2 request throttles.
        dois: Candidate DOI values.
        semantic_scholar_by_doi: Optional preloaded S2 payload map.
        batch_size: Maximum DOI count for one S2 request batch.

    Returns:
        Normalized payload mapping and fetched DOI count.
    """
    if semantic_scholar_by_doi is not None:
        return _normalize_semantic_scholar_map(semantic_scholar_by_doi), 0

    fetched: dict[str, Mapping[str, Any]] = {}
    effective_batch_size = min(max(1, batch_size), SEMANTIC_SCHOLAR_BATCH_SIZE)
    for batch in chunked(dois, effective_batch_size):
        fetched.update(await client.fetch_semantic_scholar_by_dois(batch))
    return _normalize_semantic_scholar_map(fetched), len(dois)


def _normalize_semantic_scholar_map(
    semantic_scholar_by_doi: Mapping[str, Mapping[str, Any]],
) -> dict[str, Mapping[str, Any]]:
    """
    Normalize DOI keys from a Semantic Scholar payload map.

    Args:
        semantic_scholar_by_doi: Raw S2 payload map.

    Returns:
        Payload map keyed by normalized DOI.
    """
    normalized: dict[str, Mapping[str, Any]] = {}
    for raw_doi, payload in semantic_scholar_by_doi.items():
        doi = normalize_doi(raw_doi)
        if doi:
            normalized[doi] = payload
    return normalized


def _semantic_scholar_abstract(payload: Mapping[str, Any] | None) -> str | None:
    """
    Extract a non-empty S2 abstract from a payload.

    Args:
        payload: Optional S2 payload.

    Returns:
        Abstract text or None.
    """
    if payload is None:
        return None
    value = payload.get("abstract")
    if not isinstance(value, str):
        return None
    return value.strip() or None


def _has_text(value: Any) -> bool:
    """
    Return whether a value contains non-whitespace text.

    Args:
        value: Raw text-like value.

    Returns:
        Whether the value is non-empty text.
    """
    return isinstance(value, str) and bool(value.strip())
