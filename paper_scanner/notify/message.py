"""Message rendering helpers."""

from __future__ import annotations

from paper_scanner.notify.models import (
    MAX_ARTICLES_PER_PUSH,
    MAX_PUSH_CONTENT_LENGTH,
    ArticleCandidate,
    RankedSelection,
    Subscriber,
)
from paper_scanner.shared.converters import truncate_text


def build_message_title(db_name: str, run_id: str) -> str:
    """
    Build push title.

    Args:
        db_name: Database name.
        run_id: Run identifier.

    Returns:
        Push title string.
    """
    return f"Paper Scanner Weekly Update [{db_name}] {run_id[:10]}"


def build_markdown_content(
    db_name: str,
    run_id: str,
    subscriber: Subscriber,
    summary: str,
    selections: list[RankedSelection],
    candidates_by_id: dict[int, ArticleCandidate],
) -> str:
    """
    Build markdown push content.

    Args:
        db_name: Database name.
        run_id: Run id.
        subscriber: Subscriber profile.
        summary: AI summary text.
        selections: Accepted selections.
        candidates_by_id: Candidate map.

    Returns:
        Markdown message.
    """
    base_lines = [
        f"## Weekly Digest for {subscriber.name}",
        "",
        f"- Database: `{db_name}`",
        f"- Run ID: `{run_id}`",
    ]
    intro_text = summary.strip()
    if intro_text:
        base_lines.extend(["", f"{intro_text}"])

    ranked_sections: list[tuple[RankedSelection, str]] = []
    for item in selections[:MAX_ARTICLES_PER_PUSH]:
        candidate = candidates_by_id[item.article_id]
        display_doi = (candidate.doi or "").strip() or "N/A"
        section = "\n".join(
            [
                f"### {len(ranked_sections) + 1}. {candidate.title}",
                f"- Journal: {candidate.journal_title}",
                f"- Date: {candidate.date or 'Unknown'}",
                f"- DOI: {display_doi}",
                f"- Abstract: {candidate.abstract or 'N/A'}",
            ]
        )
        ranked_sections.append((item, section))

    def render_content(sections: list[str], total_selected: int) -> str:
        """
        Render markdown content from base and article sections.

        Args:
            sections: Article section blocks.
            total_selected: Number of selected sections.

        Returns:
            Rendered markdown content.
        """
        header_lines = [*base_lines, f"- Selected Articles: {total_selected}"]
        content_parts: list[str] = ["\n".join(header_lines).strip()]
        content_parts.extend(sections)
        return "\n\n".join(part for part in content_parts if part).strip()

    kept_sections: list[str] = []
    for _, section in ranked_sections:
        trial_sections = [*kept_sections, section]
        trial_content = render_content(trial_sections, len(trial_sections))
        if len(trial_content) <= MAX_PUSH_CONTENT_LENGTH:
            kept_sections.append(section)

    content = render_content(kept_sections, len(kept_sections))
    if len(content) <= MAX_PUSH_CONTENT_LENGTH:
        return content

    base_content = render_content([], 0)
    return truncate_text(base_content, MAX_PUSH_CONTENT_LENGTH)
