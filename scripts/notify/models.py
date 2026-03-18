"""Notification models and constants."""

from __future__ import annotations

from dataclasses import dataclass

from scripts.shared.constants import PROJECT_ROOT

DEFAULT_STATE_DIR = PROJECT_ROOT / "data" / "push_state"

SILICONFLOW_BASE_URL = "https://api.siliconflow.cn/v1"

PUSHPLUS_ENDPOINT = "https://www.pushplus.plus/send"

PUSHPLUS_CHANNEL = "mail"

DEFAULT_SILICONFLOW_MODEL = "deepseek-ai/DeepSeek-V3"

MAX_ARTICLES_PER_PUSH = 20

MAX_PUSH_CONTENT_LENGTH = 18000

MAX_AI_SELECTION_ROUNDS = 5


@dataclass(frozen=True)
class ArticleCandidate:
    """
    Candidate article for recommendation and push delivery.

    Args:
        article_id: Unique article identifier.
        journal_id: Journal identifier.
        issue_id: Issue identifier when available.
        title: Article title.
        abstract: Article abstract.
        date: Publication date string.
        journal_title: Journal title.
        doi: DOI value.
        full_text_file: Full text file link.
        permalink: External permalink.
        open_access: Open access flag.
        in_press: In-press flag.
        within_library_holdings: Library holdings flag.
    """

    article_id: int
    journal_id: int
    issue_id: int | None
    title: str
    abstract: str
    date: str | None
    journal_title: str
    doi: str | None
    full_text_file: str | None
    permalink: str | None
    open_access: bool
    in_press: bool
    within_library_holdings: bool


@dataclass(frozen=True)
class Subscriber:
    """
    Subscriber configuration for AI selection and delivery.

    Args:
        subscriber_id: Stable subscriber identifier.
        name: Display name.
        pushplus_token: PushPlus token for this user.
        to: Optional recipient value for channels that support it.
        keywords: Keyword preferences.
        directions: Direction preferences.
        topic: Optional per-user PushPlus topic override.
        template: Optional per-user PushPlus template override.
        delivery_method: 'pushplus' or 'folder'.
        tracking_folder_id: Folder id for folder-based delivery.
    """

    subscriber_id: str
    name: str
    pushplus_token: str
    to: str | None
    keywords: list[str]
    directions: list[str]
    topic: str | None
    template: str | None
    delivery_method: str = "pushplus"
    tracking_folder_id: int | None = None


@dataclass(frozen=True)
class NotificationGlobal:
    """
    Global notification configuration loaded from runtime environment.

    Args:
        siliconflow_api_key: SiliconFlow API key used for AI selection.
        pushplus_channel: PushPlus channel name.
        pushplus_template: Default PushPlus template.
        pushplus_topic: Optional default PushPlus topic.
        pushplus_option: Optional PushPlus option value.
    """

    siliconflow_api_key: str
    pushplus_channel: str
    pushplus_template: str
    pushplus_topic: str | None
    pushplus_option: str | None


@dataclass(frozen=True)
class NotificationDefaults:
    """
    Global defaults loaded from runtime environment.

    Args:
        max_candidates: Maximum candidates sent to model.
        siliconflow_model: SiliconFlow model identifier.
        temperature: Model temperature.
    """

    max_candidates: int
    siliconflow_model: str
    temperature: float


@dataclass(frozen=True)
class RankedSelection:
    """
    One model-selected article result.

    Args:
        article_id: Selected article identifier.
        score: Recommendation score from 0 to 100.
    """

    article_id: int
    score: float


@dataclass(frozen=True)
class SelectionResult:
    """
    Structured model selection output.

    Args:
        summary: Short run summary for this subscriber.
        selections: Selected items.
    """

    summary: str
    selections: list[RankedSelection]
