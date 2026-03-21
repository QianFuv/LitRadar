"""Notification models and constants."""

from __future__ import annotations

from dataclasses import dataclass

from scripts.shared.constants import PROJECT_ROOT

DEFAULT_STATE_DIR = PROJECT_ROOT / "data" / "push_state"

DEFAULT_OPENAI_BASE_URL = "https://api.siliconflow.cn/v1"

PUSHPLUS_ENDPOINT = "https://www.pushplus.plus/send"

PUSHPLUS_CHANNEL = "wechat"

DEFAULT_OPENAI_MODEL = "deepseek-ai/DeepSeek-V3"

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
        channel: Optional PushPlus channel override.
        keywords: Keyword preferences.
        directions: Direction preferences.
        selected_databases: Enabled databases for this subscriber. Empty means all.
        topic: Optional per-user PushPlus topic override.
        template: Optional per-user PushPlus template override.
        delivery_method: 'pushplus', 'folder', or 'both'.
        tracking_folder_id: Folder id for folder-based delivery.
        sync_to_tracking_folder: Whether PushPlus also writes the tracking folder.
        ai_base_url: Optional OpenAI-compatible API base URL override.
        ai_api_key: Optional OpenAI-compatible API key override.
        ai_model: Optional OpenAI-compatible model override.
        ai_system_prompt: Optional custom system prompt override.
        ai_backup_base_url: Optional backup OpenAI-compatible API base URL.
        ai_backup_api_key: Optional backup OpenAI-compatible API key.
        ai_backup_model: Optional backup OpenAI-compatible model identifier.
        ai_backup_system_prompt: Optional backup custom system prompt override.
        ai_retry_attempts: Retry attempts per AI endpoint.
    """

    subscriber_id: str
    name: str
    pushplus_token: str
    channel: str | None
    keywords: list[str]
    directions: list[str]
    selected_databases: list[str]
    topic: str | None
    template: str | None
    delivery_method: str = "pushplus"
    tracking_folder_id: int | None = None
    sync_to_tracking_folder: bool = False
    ai_base_url: str | None = None
    ai_api_key: str | None = None
    ai_model: str | None = None
    ai_system_prompt: str | None = None
    ai_backup_base_url: str | None = None
    ai_backup_api_key: str | None = None
    ai_backup_model: str | None = None
    ai_backup_system_prompt: str | None = None
    ai_retry_attempts: int = 3


@dataclass(frozen=True)
class NotificationGlobal:
    """
    Global notification configuration loaded from runtime environment.

    Args:
        ai_base_url: Default OpenAI-compatible API base URL.
        ai_api_key: Default OpenAI-compatible API key used for AI selection.
        pushplus_channel: PushPlus channel name.
        pushplus_template: Default PushPlus template.
        pushplus_topic: Optional default PushPlus topic.
        pushplus_option: Optional PushPlus option value.
        ai_system_prompt: Optional default custom system prompt.
    """

    ai_base_url: str
    ai_api_key: str
    pushplus_channel: str
    pushplus_template: str
    pushplus_topic: str | None
    pushplus_option: str | None
    ai_system_prompt: str | None = None


@dataclass(frozen=True)
class NotificationDefaults:
    """
    Global defaults loaded from runtime environment.

    Args:
        max_candidates: Maximum candidates sent to model.
        ai_model: Default OpenAI-compatible model identifier.
        temperature: Model temperature.
    """

    max_candidates: int
    ai_model: str
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
