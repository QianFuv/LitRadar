//! Shared domain models and compatibility primitives for the backend.

pub mod announcements;
pub mod auth;
pub mod business;
pub mod cnki;
pub mod concurrency;
pub mod health;
pub mod ids;
pub mod index;
pub mod index_contract;
pub mod recommend;
pub mod response;
pub mod validation;

pub use announcements::AnnouncementInfo;
pub use auth::{
    is_valid_invite_code_policy, ChangePasswordRequest, InviteCodeResponse, InviteCodeStatus,
    InviteRequiredResponse, LoginRequest, LoginResponse, LogoutResponse, OkResponse,
    RegisterRequest, SessionRevocationErrorDetail, SessionRevocationErrorResponse,
    TokenCreateRequest, TokenCreateResponse, TokenInfo, UserResponse, ACCESS_TOKEN_ACTIVE_LIMIT,
    ACCESS_TOKEN_LIMIT_DETAIL, ACCESS_TOKEN_NAME_LENGTH_DETAIL, ACCESS_TOKEN_NAME_MAX_CODE_POINTS,
    ACCESS_TOKEN_RESERVED_NAME, ACCESS_TOKEN_RESERVED_NAME_DETAIL, ACCESS_TOKEN_TTL_DETAIL,
    ACCESS_TOKEN_TTL_MAX_SECONDS, ACCESS_TOKEN_TTL_MIN_SECONDS, ACCESS_TOKEN_VALIDATION_ORDER,
    DEFAULT_INVITE_CODE_MAX_USES, DEFAULT_INVITE_CODE_TTL_SECONDS, MAX_INVITE_CODE_TTL_SECONDS,
    MAX_INVITE_CODE_USES,
};
pub use business::{
    validate_scheduled_task_timing, AdminInviteCodeCreate, AdminInviteCodeInfo, AdminResetPassword,
    AdminSetAdmin, AdminStatsResponse, AdminUserInfo, AnnouncementCreate, AnnouncementUpdate,
    AuthStats, FavoriteAdd, FavoriteArticlePage, FavoriteArticleRef, FavoriteArticleResponse,
    FavoriteBatchCheckRequest, FavoriteBatchCheckResponse, FavoriteBulkAdd, FavoriteBulkAddResult,
    FavoriteBulkMove, FavoriteBulkRemove, FavoriteBulkResult, FavoriteCheckResponse,
    FavoriteMetadataStatus, FavoriteResponse, FavoriteTrackingResponse, FolderCreate, FolderRename,
    FolderResponse, IndexDatabaseStats, IndexStats, NotificationSettings,
    NotificationSettingsResponse, NotificationSettingsUpdate, ProviderCapabilityInfo,
    ProviderCatalogInfo, ProviderCatalogResponse, ProviderOrderConfiguration, PushStats,
    PushStatsState, RuntimeSecretItemInfo, RuntimeSecretPoolUpdate, RuntimeSettingApplyMode,
    RuntimeSettingControl, RuntimeSettingGroup, RuntimeSettingInfo, RuntimeSettingValue,
    RuntimeSettingsUpdate, ScheduledDeliveryJob, ScheduledIndexJob, ScheduledJobSpec,
    ScheduledJobValidationError, ScheduledTaskCreate, ScheduledTaskInfo, ScheduledTaskRunInfo,
    ScheduledTaskUpdate, ScheduledTaskValidationError, SchedulerRunState, SchedulerStatusResponse,
    SchedulerWorkerInfo, TrackingFolderSummary, TrackingSetRequest, TrackingStatusResponse,
    DELIVERY_RETRY_ATTEMPTS_MAX, DELIVERY_RETRY_ATTEMPTS_MIN, NOTIFICATION_AI_RETRY_ATTEMPTS_MAX,
    NOTIFICATION_AI_RETRY_ATTEMPTS_MIN,
};
pub use cnki::{
    CnkiErrorDetail, CnkiLoginPollRequest, CnkiLoginPollResponse, CnkiLoginStartResponse,
    CnkiSessionStatusResponse, CnkiStatus,
};
pub use concurrency::{
    validate_domestic_cnki_worker_count, validate_index_concurrency, IndexConcurrency,
    IndexConcurrencyError, DOMESTIC_CNKI_WORKER_COUNT_MAX, INDEX_AGGREGATE_CONCURRENCY_MAX,
    INDEX_PROCESS_COUNT_MAX, INDEX_PROCESS_COUNT_MIN, INDEX_WORKER_COUNT_MAX,
    INDEX_WORKER_COUNT_MIN, SCHOLARLY_PROCESS_COUNT_MAX, SCHOLARLY_WORKER_COUNT_MAX,
};
pub use health::{HealthResponse, HealthStatus};
pub use ids::{stable_sqlite_id, ArticleId, JournalId, UserId};
pub use index::{
    ArticleAccessAction, ArticleAccessResponse, ArticlePage, ArticleRecord, ArticleSearchMode,
    IssuePage, IssueRecord, JournalOption, JournalPage, JournalRecord, PageMeta, ValueCount,
    WeeklyArticlePage, WeeklyArticleRecord, WeeklyDatabaseSummary, WeeklyDatabaseUpdate,
    WeeklyJournalSummary, WeeklyJournalUpdate, WeeklyUpdatesResponse, WeeklyUpdatesSummaryResponse,
    YearSummary,
};
pub use index_contract::{
    date_precision, normalize_bibliographic_label, normalize_bibliographic_text,
    normalize_contract_date, normalize_contract_doi, normalize_contract_issn,
    normalize_contract_pmid, normalize_contract_text, ArticleAccessContext, ArticleAuthorDraft,
    ArticleDraft, ArticleFullTextDocument, ArticleFullTextResolution, ArticleLocator,
    ArticleRedirect, CanonicalPartialDate, DatePrecision, IndexFetchContext, IndexSyncMode,
    IssueDraft, JournalCatalogEntry, JournalDraft, JournalRankings, ProviderBatch,
    ProviderCapabilityKind, ProviderProgress, INDEX_CONTRACT_VERSION,
};
pub use recommend::{
    ArticleCandidateInfo, ManualPushState, ManualWeeklyPushStatus, NotificationSubscriberInfo,
    RankedSelectionInfo, SelectionResultInfo,
};
pub use response::ErrorEnvelope;
pub use validation::{
    validate_announcement_fields, validate_characters, validate_favorite_add,
    validate_favorite_article_ref, validate_folder_name, validate_item_count, validate_mcp_array,
    validate_mcp_text, validate_notification_dependencies, validate_notification_settings,
    validate_positive_id, validate_required_characters, InputValidationError,
    MAX_ANNOUNCEMENT_MESSAGE_CHARS, MAX_ANNOUNCEMENT_PRIORITY_CHARS, MAX_ANNOUNCEMENT_TITLE_CHARS,
    MAX_BATCH_ARTICLE_IDS, MAX_DATABASE_NAME_CHARS, MAX_FAVORITE_NOTE_CHARS, MAX_FOLDER_NAME_CHARS,
    MAX_MCP_ARRAY_ITEMS, MAX_MCP_TEXT_CHARS, MAX_NOTIFICATION_DIRECTIONS,
    MAX_NOTIFICATION_KEYWORDS, MAX_NOTIFICATION_MODEL_CHARS, MAX_NOTIFICATION_PREFERENCE_CHARS,
    MAX_NOTIFICATION_PROMPT_CHARS, MAX_NOTIFICATION_SECRET_CHARS, MAX_NOTIFICATION_URL_CHARS,
    MAX_PUSHPLUS_CHANNEL_CHARS, MAX_PUSHPLUS_TEMPLATE_CHARS, MAX_PUSHPLUS_TOPIC_CHARS,
    MAX_SEARCH_FILTER_ITEMS, MAX_SEARCH_TEXT_CHARS, MAX_SELECTED_DATABASES,
    SQLITE_IN_QUERY_CHUNK_SIZE,
};
