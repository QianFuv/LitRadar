"""API response models."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime

from pydantic import BaseModel, Field


class JournalRecord(BaseModel):
    """
    Journal record with optional CSV metadata.
    """

    journal_id: int
    library_id: str
    title: str | None = None
    issn: str | None = None
    eissn: str | None = None
    scimago_rank: float | None = None
    cover_url: str | None = None
    available: int | None = None
    toc_data_approved_and_live: int | None = None
    has_articles: int | None = None
    source_csv: str | None = None
    area: str | None = None
    csv_title: str | None = None
    csv_issn: str | None = None
    csv_library: str | None = None


class IssueRecord(BaseModel):
    """
    Issue record.
    """

    issue_id: int
    journal_id: int
    publication_year: int | None = None
    title: str | None = None
    volume: str | None = None
    number: str | None = None
    date: str | None = None
    is_valid_issue: int | None = None
    suppressed: int | None = None
    embargoed: int | None = None
    within_subscription: int | None = None


class ArticleRecord(BaseModel):
    """
    Article record.
    """

    article_id: int
    journal_id: int
    issue_id: int | None = None
    sync_id: int | None = None
    title: str | None = None
    date: str | None = None
    authors: str | None = None
    start_page: str | None = None
    end_page: str | None = None
    abstract: str | None = None
    doi: str | None = None
    pmid: str | None = None
    ill_url: str | None = None
    link_resolver_openurl_link: str | None = None
    email_article_request_link: str | None = None
    permalink: str | None = None
    suppressed: int | None = None
    in_press: int | None = None
    open_access: int | None = None
    platform_id: str | None = None
    retraction_doi: str | None = None
    retraction_date: str | None = None
    retraction_related_urls: str | None = None
    unpaywall_data_suppressed: int | None = None
    expression_of_concern_doi: str | None = None
    within_library_holdings: int | None = None
    noodletools_export_link: str | None = None
    avoid_unpaywall_publisher_links: int | None = None
    browzine_web_in_context_link: str | None = None
    content_location: str | None = None
    libkey_content_location: str | None = None
    full_text_file: str | None = None
    libkey_full_text_file: str | None = None
    nomad_fallback_url: str | None = None
    journal_title: str | None = None
    volume: str | None = None
    number: str | None = None


class PageMeta(BaseModel):
    """
    Pagination metadata.
    """

    total: int | None
    limit: int
    offset: int
    next_cursor: str | None = None
    has_more: bool | None = None


class JournalPage(BaseModel):
    """
    Paginated journals response.
    """

    items: list[JournalRecord]
    page: PageMeta


class IssuePage(BaseModel):
    """
    Paginated issues response.
    """

    items: list[IssueRecord]
    page: PageMeta


class ArticlePage(BaseModel):
    """
    Paginated articles response.
    """

    items: list[ArticleRecord]
    page: PageMeta


class ValueCount(BaseModel):
    """
    Label and count tuple.
    """

    value: str
    count: int


class YearSummary(BaseModel):
    """
    Publication year summary.
    """

    year: int
    issue_count: int
    journal_count: int


class JournalOption(BaseModel):
    """
    Journal option for selection lists.
    """

    journal_id: int
    title: str | None = None


class WeeklyArticleRecord(BaseModel):
    """
    Weekly update article record.
    """

    article_id: int
    journal_id: int
    issue_id: int | None = None
    title: str | None = None
    date: str | None = None
    doi: str | None = None
    journal_title: str | None = None
    open_access: int | None = None
    in_press: int | None = None


class WeeklyJournalUpdate(BaseModel):
    """
    Weekly update summary for one journal.
    """

    journal_id: int
    journal_title: str | None = None
    new_article_count: int
    articles: list[WeeklyArticleRecord]


class WeeklyDatabaseUpdate(BaseModel):
    """
    Weekly update summary for one database.
    """

    db_name: str
    run_id: str | None = None
    generated_at: str
    new_article_count: int
    journals: list[WeeklyJournalUpdate]


class WeeklyUpdatesResponse(BaseModel):
    """
    Weekly updates grouped by database and journal.
    """

    generated_at: str
    window_start: str
    window_end: str
    databases: list[WeeklyDatabaseUpdate]


@dataclass(frozen=True)
class WeeklyManifestSummary:
    """
    Parsed weekly changes manifest summary.
    """

    db_name: str
    run_id: str | None
    generated_at: datetime
    article_ids: list[int]


class RegisterRequest(BaseModel):
    username: str
    password: str
    invite_code: str


class LoginRequest(BaseModel):
    username: str
    password: str


class TokenCreateRequest(BaseModel):
    name: str = ""
    ttl: int = 7 * 24 * 3600


class ChangePasswordRequest(BaseModel):
    old_password: str
    new_password: str


class UserResponse(BaseModel):
    id: int
    username: str
    is_admin: bool = False


class LoginResponse(BaseModel):
    user: UserResponse
    access_token: str
    expires_at: float


class TokenInfo(BaseModel):
    id: int
    name: str
    expires_at: float
    created_at: float


class TokenCreateResponse(BaseModel):
    id: int
    token: str
    name: str
    expires_at: float


class FolderCreate(BaseModel):
    name: str
    is_tracking: bool = False


class FolderRename(BaseModel):
    name: str


class FolderResponse(BaseModel):
    id: int
    name: str
    is_tracking: bool
    article_count: int = 0
    created_at: float


class FavoriteAdd(BaseModel):
    article_id: int
    db_name: str = ""
    note: str = ""


class FavoriteArticleRef(BaseModel):
    article_id: int
    db_name: str = ""


class FavoriteResponse(BaseModel):
    id: int
    folder_id: int
    article_id: int
    db_name: str
    note: str
    created_at: float


class FavoriteArticleResponse(FavoriteResponse):
    journal_id: int | None = None
    issue_id: int | None = None
    title: str | None = None
    date: str | None = None
    authors: str | None = None
    abstract: str | None = None
    doi: str | None = None
    platform_id: str | None = None
    journal_title: str | None = None
    open_access: int | None = None
    in_press: int | None = None
    volume: str | None = None
    number: str | None = None
    issn: str | None = None
    eissn: str | None = None
    full_text_file: str | None = None


class FavoriteCheckResponse(BaseModel):
    folder_id: int
    folder_name: str


class FavoriteBatchCheckRequest(BaseModel):
    article_ids: list[int]
    db_name: str = ""


class FavoriteBatchCheckResponse(BaseModel):
    article_id: int
    folders: list[FavoriteCheckResponse]


class FavoriteBulkAdd(BaseModel):
    articles: list[FavoriteAdd]


class FavoriteBulkRemove(BaseModel):
    articles: list[FavoriteArticleRef]


class FavoriteBulkMove(BaseModel):
    target_folder_id: int
    articles: list[FavoriteArticleRef]


class FavoriteBulkResult(BaseModel):
    count: int


class TrackingSetRequest(BaseModel):
    folder_id: int


class InviteCodeResponse(BaseModel):
    id: int
    code: str
    used: bool
    created_at: float


class NotificationSettingsUpdate(BaseModel):
    keywords: list[str] = Field(default_factory=list)
    directions: list[str] = Field(default_factory=list)
    delivery_method: str = "folder"
    pushplus_token: str = ""
    pushplus_template: str = "markdown"
    pushplus_topic: str = ""
    pushplus_to: str = ""
    sync_to_tracking_folder: bool = False
    ai_base_url: str = ""
    ai_api_key: str = ""
    ai_model: str = ""
    ai_system_prompt: str = ""
    enabled: bool = True


class NotificationSettingsResponse(BaseModel):
    id: int
    user_id: int
    keywords: list[str]
    directions: list[str]
    delivery_method: str
    pushplus_token: str
    pushplus_template: str
    pushplus_topic: str
    pushplus_to: str
    sync_to_tracking_folder: bool
    ai_base_url: str
    ai_api_key: str
    ai_model: str
    ai_system_prompt: str
    enabled: bool
    created_at: float
    updated_at: float


class ScheduledTaskInfo(BaseModel):
    id: int
    name: str
    command: str
    cron: str
    enabled: bool
    last_run_at: float | None = None
    last_status: str
    created_at: float
    updated_at: float


class ScheduledTaskCreate(BaseModel):
    name: str
    command: str
    cron: str
    enabled: bool = True


class ScheduledTaskUpdate(BaseModel):
    name: str | None = None
    command: str | None = None
    cron: str | None = None
    enabled: bool | None = None


class AnnouncementInfo(BaseModel):
    id: int
    title: str
    message: str
    priority: str = "normal"
    enabled: bool
    created_at: float
    updated_at: float


class AnnouncementCreate(BaseModel):
    title: str
    message: str
    priority: str = "normal"
    enabled: bool = True


class AnnouncementUpdate(BaseModel):
    title: str | None = None
    message: str | None = None
    priority: str | None = None
    enabled: bool | None = None


class AdminUserInfo(BaseModel):
    id: int
    username: str
    is_admin: bool
    created_at: float
    updated_at: float
    folder_count: int = 0
    favorite_count: int = 0
    notify_enabled: bool = False


class AdminSetAdmin(BaseModel):
    is_admin: bool


class AdminResetPassword(BaseModel):
    new_password: str


class AdminInviteCodeInfo(BaseModel):
    id: int
    code: str
    created_by: int | None = None
    created_by_name: str | None = None
    used_by: int | None = None
    used_by_name: str | None = None
    used_at: float | None = None
    created_at: float
