"""Article selection logic."""

from __future__ import annotations

from scripts.notify.ai_selector import OpenAICompatibleSelector
from scripts.notify.models import (
    MAX_ARTICLES_PER_PUSH,
    ArticleCandidate,
    NotificationDefaults,
    RankedSelection,
    SelectionResult,
    Subscriber,
)


def candidate_match_score(candidate: ArticleCandidate, subscriber: Subscriber) -> int:
    """
    Compute keyword and direction match count for one candidate.

    Args:
        candidate: Candidate article.
        subscriber: Subscriber profile.

    Returns:
        Number of matched keyword and direction phrases.
    """
    source_text = f"{candidate.title} {candidate.abstract}".lower()
    phrases: list[str] = []
    phrases.extend(subscriber.keywords)
    phrases.extend(subscriber.directions)

    score = 0
    for phrase in phrases:
        normalized = phrase.lower().strip()
        if normalized and normalized in source_text:
            score += 1
    return score


def select_articles_with_retries(
    selector: OpenAICompatibleSelector,
    subscriber: Subscriber,
    defaults: NotificationDefaults,
    candidates_for_model: list[ArticleCandidate],
    candidates_by_id: dict[int, ArticleCandidate],
    delivery_dedupe: dict[str, str],
    max_rounds: int,
) -> SelectionResult:
    """
    Query model multiple times on remaining candidates when results are sparse.

    Args:
        selector: OpenAI-compatible selector client.
        subscriber: Subscriber profile.
        defaults: Notification defaults.
        candidates_for_model: Candidates sent to model.
        candidates_by_id: Candidate lookup map.
        delivery_dedupe: Delivery dedupe map.
        max_rounds: Maximum model query rounds.

    Returns:
        Aggregated selection result across rounds.
    """
    rounds = max(1, max_rounds)
    remaining_candidates = [*candidates_for_model]
    aggregated: dict[int, RankedSelection] = {}
    summary = ""

    for _ in range(rounds):
        if not remaining_candidates:
            break

        round_result = selector.select_articles(
            subscriber,
            defaults,
            remaining_candidates,
        )
        if not summary and round_result.summary:
            summary = round_result.summary

        for item in round_result.selections:
            existing = aggregated.get(item.article_id)
            if existing is None or item.score > existing.score:
                aggregated[item.article_id] = item

        merged = SelectionResult(
            summary=summary,
            selections=sorted(
                aggregated.values(),
                key=lambda item: item.score,
                reverse=True,
            ),
        )

        accepted = apply_selection_rules(
            merged,
            subscriber,
            candidates_by_id,
            delivery_dedupe,
        )
        if len(accepted) >= MAX_ARTICLES_PER_PUSH:
            return merged

        selected_ids = {item.article_id for item in aggregated.values()}
        remaining_candidates = [
            item for item in remaining_candidates if item.article_id not in selected_ids
        ]

    return SelectionResult(
        summary=summary,
        selections=sorted(
            aggregated.values(),
            key=lambda item: item.score,
            reverse=True,
        ),
    )


def apply_selection_rules(
    selection_result: SelectionResult,
    subscriber: Subscriber,
    candidates_by_id: dict[int, ArticleCandidate],
    delivery_dedupe: dict[str, str],
) -> list[RankedSelection]:
    """
    Apply local rules to model output.

    Args:
        selection_result: Model output.
        subscriber: Subscriber configuration.
        candidates_by_id: Candidate map.
        delivery_dedupe: Delivery dedupe map.

    Returns:
        Filtered selection list.
    """
    eligible: list[RankedSelection] = []
    selected_ids: set[int] = set()

    for selection in selection_result.selections:
        candidate = candidates_by_id.get(selection.article_id)
        if candidate is None:
            continue
        dedupe_key = f"{subscriber.subscriber_id}:{candidate.article_id}"
        if dedupe_key in delivery_dedupe:
            continue
        eligible.append(selection)
        selected_ids.add(selection.article_id)

    supplemental: list[RankedSelection] = []
    if len(eligible) < MAX_ARTICLES_PER_PUSH:
        for candidate in candidates_by_id.values():
            if candidate.article_id in selected_ids:
                continue
            dedupe_key = f"{subscriber.subscriber_id}:{candidate.article_id}"
            if dedupe_key in delivery_dedupe:
                continue
            if candidate_match_score(candidate, subscriber) <= 0:
                continue
            supplemental.append(
                RankedSelection(
                    article_id=candidate.article_id,
                    score=0.0,
                )
            )

        supplemental.sort(
            key=lambda item: (
                candidate_match_score(candidates_by_id[item.article_id], subscriber),
                candidates_by_id[item.article_id].article_id,
            ),
            reverse=True,
        )

    merged = [*eligible, *supplemental]

    if not merged:
        return []

    match_sorted = sorted(
        merged,
        key=lambda item: (
            candidate_match_score(candidates_by_id[item.article_id], subscriber),
            item.score,
        ),
        reverse=True,
    )
    return match_sorted[:MAX_ARTICLES_PER_PUSH]
