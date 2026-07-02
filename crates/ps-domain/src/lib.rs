//! Shared domain models and compatibility primitives for the backend.

pub mod announcements;
pub mod auth;
pub mod business;
pub mod health;
pub mod ids;
pub mod index;
pub mod response;

pub use announcements::AnnouncementInfo;
pub use auth::{
    ChangePasswordRequest, InviteCodeResponse, InviteRequiredResponse, LoginRequest, LoginResponse,
    LogoutResponse, OkResponse, RegisterRequest, TokenCreateRequest, TokenCreateResponse,
    TokenInfo, UserResponse,
};
pub use business::{
    AdminInviteCodeInfo, AdminResetPassword, AdminSetAdmin, AdminStatsResponse, AdminUserInfo,
    AnnouncementCreate, AnnouncementUpdate, AuthStats, FavoriteAdd, FavoriteArticleRef,
    FavoriteArticleResponse, FavoriteBatchCheckRequest, FavoriteBatchCheckResponse,
    FavoriteBulkAdd, FavoriteBulkAddResult, FavoriteBulkMove, FavoriteBulkRemove,
    FavoriteBulkResult, FavoriteCheckResponse, FavoriteResponse, FavoriteTrackingResponse,
    FolderCreate, FolderRename, FolderResponse, IndexDatabaseStats, IndexStats,
    NotificationSettingsResponse, NotificationSettingsUpdate, PushStats, RuntimeSettingInfo,
    RuntimeSettingsUpdate, ScheduledTaskCreate, ScheduledTaskInfo, ScheduledTaskUpdate,
    TrackingFolderSummary, TrackingSetRequest, TrackingStatusResponse,
};
pub use health::HealthResponse;
pub use ids::{stable_sqlite_id, ArticleId, JournalId, UserId};
pub use index::{
    ArticleAccessAction, ArticleAccessResponse, ArticlePage, ArticleRecord, IssuePage, IssueRecord,
    JournalOption, JournalPage, JournalRecord, PageMeta, ValueCount, WeeklyArticleRecord,
    WeeklyDatabaseUpdate, WeeklyJournalUpdate, WeeklyUpdatesResponse, YearSummary,
};
pub use response::ErrorEnvelope;
