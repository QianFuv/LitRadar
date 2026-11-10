package openapi

import (
	"fmt"
	"github.com/danielgtaylor/huma/v2"
	"net/http"
)

// ListAnnouncementsInput contains parameters for list_announcements.
type ListAnnouncementsInput struct {
}

// CreateAnnouncementInput contains parameters for create_announcement.
type CreateAnnouncementInput struct {
	Body AnnouncementCreate `required:"true"`
}

// UpdateAnnouncementInput contains parameters for update_announcement.
type UpdateAnnouncementInput struct {
	AnnouncementID int64              `path:"announcement_id" required:"true" nullable:"false" format:"int64" doc:"Announcement row identifier."`
	Body           AnnouncementUpdate `required:"true"`
}

// DeleteAnnouncementInput contains parameters for delete_announcement.
type DeleteAnnouncementInput struct {
	AnnouncementID int64 `path:"announcement_id" required:"true" nullable:"false" format:"int64" doc:"Announcement row identifier."`
}

// ListInviteCodesInput contains parameters for list_invite_codes.
type ListInviteCodesInput struct {
}

// CreateInviteCodeInput contains parameters for create_invite_code.
type CreateInviteCodeInput struct {
	Body *AdminInviteCodeCreate
}

// RevokeAdminInviteCodeInput contains parameters for revoke_admin_invite_code.
type RevokeAdminInviteCodeInput struct {
	CodeID int64 `path:"code_id" required:"true" nullable:"false" format:"int64" doc:"Invite code row identifier."`
}

// GetProviderCatalogInput contains parameters for get_provider_catalog.
type GetProviderCatalogInput struct {
}

// ListRuntimeSettingsInput contains parameters for list_runtime_settings.
type ListRuntimeSettingsInput struct {
}

// UpdateRuntimeSettingsInput contains parameters for update_runtime_settings.
type UpdateRuntimeSettingsInput struct {
	Body RuntimeSettingsUpdate `required:"true"`
}

// ListScheduledTasksInput contains parameters for list_scheduled_tasks.
type ListScheduledTasksInput struct {
}

// CreateScheduledTaskInput contains parameters for create_scheduled_task.
type CreateScheduledTaskInput struct {
	Body ScheduledTaskCreate `required:"true"`
}

// UpdateScheduledTaskInput contains parameters for update_scheduled_task.
type UpdateScheduledTaskInput struct {
	TaskID int64               `path:"task_id" required:"true" nullable:"false" format:"int64" doc:"Scheduled task row identifier."`
	Body   ScheduledTaskUpdate `required:"true"`
}

// DeleteScheduledTaskInput contains parameters for delete_scheduled_task.
type DeleteScheduledTaskInput struct {
	TaskID int64 `path:"task_id" required:"true" nullable:"false" format:"int64" doc:"Scheduled task row identifier."`
}

// SchedulerStatusInput contains parameters for scheduler_status.
type SchedulerStatusInput struct {
}

// StatsInput contains parameters for stats.
type StatsInput struct {
}

// ListUsersInput contains parameters for list_users.
type ListUsersInput struct {
}

// DeleteUserInput contains parameters for delete_user.
type DeleteUserInput struct {
	UserID int64 `path:"user_id" required:"true" nullable:"false" format:"int64" doc:"User row identifier."`
}

// SetAdminInput contains parameters for set_admin.
type SetAdminInput struct {
	UserID int64         `path:"user_id" required:"true" nullable:"false" format:"int64" doc:"User row identifier."`
	Body   AdminSetAdmin `required:"true"`
}

// ResetPasswordInput contains parameters for reset_password.
type ResetPasswordInput struct {
	UserID int64              `path:"user_id" required:"true" nullable:"false" format:"int64" doc:"User row identifier."`
	Body   AdminResetPassword `required:"true"`
}

// GetAnnouncementsInput contains parameters for get_announcements.
type GetAnnouncementsInput struct {
}

// ListArticlesInput contains parameters for list_articles.
type ListArticlesInput struct {
	Db           string            `query:"db" nullable:"false" maxLength:"255" doc:"Database name or filename under data/index."`
	JournalID    []int64           `query:"journal_id,explode" nullable:"false" maxItems:"500" doc:"Repeated journal identifier filters."`
	Area         []string          `query:"area,explode" nullable:"false" maxItems:"500" doc:"Repeated area filters."`
	UtdRating    []string          `query:"utd_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact UTD grades; OR within a system and AND across systems/other filters. Blank values are invalid; all filter items share a 500-item bound."`
	AbsRating    []string          `query:"abs_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact ABS grades, including literal 4*."`
	FmsRating    []string          `query:"fms_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS grades."`
	FmscnRating  []string          `query:"fmscn_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS China grades."`
	IssueID      int64             `query:"issue_id" nullable:"false" format:"int64" doc:"Issue identifier filter."`
	Year         int64             `query:"year" nullable:"false" format:"int64" doc:"Publication year filter."`
	InPress      bool              `query:"in_press" nullable:"false" doc:"In-press filter."`
	OpenAccess   bool              `query:"open_access" nullable:"false" doc:"Open-access filter."`
	DateFrom     string            `query:"date_from" nullable:"false" maxLength:"2048" doc:"Start date filter."`
	DateTo       string            `query:"date_to" nullable:"false" maxLength:"2048" doc:"End date filter."`
	DOI          string            `query:"doi" nullable:"false" maxLength:"2048" doc:"DOI filter."`
	PMID         string            `query:"pmid" nullable:"false" maxLength:"2048" doc:"PubMed identifier filter."`
	Q            string            `query:"q" nullable:"false" maxLength:"2048" doc:"Full-text query."`
	SearchMode   ArticleSearchMode `query:"search_mode" nullable:"false" enum:"simple,advanced" doc:"Full-text query interpretation mode; defaults to simple."`
	Sort         string            `query:"sort" nullable:"false" maxLength:"2048" doc:"Sort expression."`
	Limit        int64             `query:"limit" nullable:"false" format:"int64" doc:"Page size."`
	Offset       int64             `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
	Cursor       string            `query:"cursor" nullable:"false" maxLength:"2048" doc:"Keyset cursor."`
	IncludeTotal bool              `query:"include_total" nullable:"false" doc:"Whether to include the full filtered row count; defaults to true without a cursor and false with a cursor."`
}

// GetArticleInput contains parameters for get_article.
type GetArticleInput struct {
	ArticleID int64  `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListRelatedArticlesInput contains parameters for list_related_articles.
type ListRelatedArticlesInput struct {
	ArticleID int64  `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Source article identifier."`
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
	Limit     int64  `query:"limit" nullable:"false" format:"int64" doc:"Maximum related articles, from 1 to 30. Defaults to 10."`
}

// RedirectArticleAbstractInput contains parameters for redirect_article_abstract.
type RedirectArticleAbstractInput struct {
	ArticleID int64  `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// GetArticleAccessInput contains parameters for get_article_access.
type GetArticleAccessInput struct {
	ArticleID int64  `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// RedirectArticleFulltextInput contains parameters for redirect_article_fulltext.
type RedirectArticleFulltextInput struct {
	ArticleID int64  `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ChangePasswordInput contains parameters for change_password.
type ChangePasswordInput struct {
	Body ChangePasswordRequest `required:"true"`
}

// GetInviteCodeInput contains parameters for get_invite_code.
type GetInviteCodeInput struct {
}

// GenerateInviteCodeInput contains parameters for generate_invite_code.
type GenerateInviteCodeInput struct {
}

// RevokeInviteCodeInput contains parameters for revoke_invite_code.
type RevokeInviteCodeInput struct {
}

// RotateInviteCodeInput contains parameters for rotate_invite_code.
type RotateInviteCodeInput struct {
}

// CheckInviteRequiredInput contains parameters for check_invite_required.
type CheckInviteRequiredInput struct {
}

// LoginInput contains parameters for login.
type LoginInput struct {
	Body LoginRequest `required:"true"`
}

// LogoutInput contains parameters for logout.
type LogoutInput struct {
}

// LogoutAllInput contains parameters for logout_all.
type LogoutAllInput struct {
}

// GetMeInput contains parameters for get_me.
type GetMeInput struct {
}

// RegisterInput contains parameters for register.
type RegisterInput struct {
	Body RegisterRequest `required:"true"`
}

// GetTokensInput contains parameters for get_tokens.
type GetTokensInput struct {
}

// CreateTokenInput contains parameters for create_token.
type CreateTokenInput struct {
	Body TokenCreateRequest `required:"true"`
}

// DeleteTokenInput contains parameters for delete_token.
type DeleteTokenInput struct {
	TokenID int64 `path:"token_id" required:"true" nullable:"false" format:"int64" doc:"Token row identifier."`
}

// ListCfpJournalsInput contains parameters for list_cfp_journals.
type ListCfpJournalsInput struct {
	Db string `query:"db" required:"true" nullable:"false" doc:"Database filename, such as english_journals.sqlite."`
	Q  string `query:"q" nullable:"false" doc:"Case-insensitive journal title, ID or subject search, limited to 256 characters."`
}

// ListCfpNoticesInput contains parameters for list_cfp_notices.
type ListCfpNoticesInput struct {
	CatalogID     string `path:"catalog_id" required:"true" nullable:"false" doc:"Canonical maintained catalog ID or its alias."`
	Db            string `query:"db" required:"true" nullable:"false" doc:"Database filename containing the maintained journal."`
	IncludeClosed bool   `query:"include_closed" nullable:"false" doc:"Include closed and historical records; defaults to false."`
	Limit         int64  `query:"limit" nullable:"false" minimum:"0" doc:"Page size, default 50 and maximum 200."`
	Cursor        string `query:"cursor" nullable:"false" doc:"Opaque cursor returned by this journal/filter's previous page."`
}

// PollLoginInput contains parameters for poll_login.
type PollLoginInput struct {
	Body CnkiLoginPollRequest `required:"true"`
}

// StartLoginInput contains parameters for start_login.
type StartLoginInput struct {
}

// GetSessionInput contains parameters for get_session.
type GetSessionInput struct {
}

// ClearSessionInput contains parameters for clear_session.
type ClearSessionInput struct {
}

// CheckFavoriteInput contains parameters for check_favorite.
type CheckFavoriteInput struct {
	ArticleID int64  `query:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	DBName    string `query:"db_name" nullable:"false" doc:"Source database name."`
}

// CheckFavoritesBatchInput contains parameters for check_favorites_batch.
type CheckFavoritesBatchInput struct {
	Body FavoriteBatchCheckRequest `required:"true"`
}

// ListFoldersInput contains parameters for list_folders.
type ListFoldersInput struct {
}

// CreateFolderInput contains parameters for create_folder.
type CreateFolderInput struct {
	Body FolderCreate `required:"true"`
}

// RenameFolderInput contains parameters for rename_folder.
type RenameFolderInput struct {
	FolderID int64        `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Body     FolderRename `required:"true"`
}

// DeleteFolderInput contains parameters for delete_folder.
type DeleteFolderInput struct {
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
}

// ListFolderArticlesInput contains parameters for list_folder_articles.
type ListFolderArticlesInput struct {
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Limit    int64 `query:"limit" nullable:"false" format:"int64" doc:"Maximum row count."`
	Offset   int64 `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
}

// AddFavoriteInput contains parameters for add_favorite.
type AddFavoriteInput struct {
	FolderID int64       `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Body     FavoriteAdd `required:"true"`
}

// BulkAddInput contains parameters for bulk_add.
type BulkAddInput struct {
	FolderID int64           `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Body     FavoriteBulkAdd `required:"true"`
}

// BulkMoveInput contains parameters for bulk_move.
type BulkMoveInput struct {
	FolderID int64            `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Body     FavoriteBulkMove `required:"true"`
}

// BulkRemoveInput contains parameters for bulk_remove.
type BulkRemoveInput struct {
	FolderID int64              `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Body     FavoriteBulkRemove `required:"true"`
}

// ListFolderArticlePageInput contains parameters for list_folder_article_page.
type ListFolderArticlePageInput struct {
	FolderID int64  `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Limit    int64  `query:"limit" nullable:"false" minimum:"1" maximum:"500" format:"int64" doc:"Maximum row count, defaulting to fifty."`
	Cursor   string `query:"cursor" nullable:"false" maxLength:"128" doc:"Opaque continuation bound to this user and folder."`
}

// RemoveFavoriteInput contains parameters for remove_favorite.
type RemoveFavoriteInput struct {
	FolderID  int64  `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	ArticleID int64  `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	DBName    string `query:"db_name" nullable:"false" doc:"Source database name."`
}

// FolderCountInput contains parameters for folder_count.
type FolderCountInput struct {
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
}

// ExportFolderInput contains parameters for export_folder.
type ExportFolderInput struct {
	FolderID int64  `path:"folder_id" required:"true" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Format   string `query:"format" nullable:"false" doc:"Export format."`
}

// GetTrackingInput contains parameters for get_tracking.
type GetTrackingInput struct {
}

// SetTrackingInput contains parameters for set_tracking.
type SetTrackingInput struct {
	Body TrackingSetRequest `required:"true"`
}

// ListIssuesInput contains parameters for list_issues.
type ListIssuesInput struct {
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
	JournalID int64  `query:"journal_id" nullable:"false" format:"int64" doc:"Journal identifier filter."`
	Year      int64  `query:"year" nullable:"false" format:"int64" doc:"Publication year filter."`
	Sort      string `query:"sort" nullable:"false" doc:"Sort expression."`
	Limit     int64  `query:"limit" nullable:"false" format:"int64" doc:"Page size."`
	Offset    int64  `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
}

// GetIssueInput contains parameters for get_issue.
type GetIssueInput struct {
	IssueID int64  `path:"issue_id" required:"true" nullable:"false" format:"int64" doc:"Issue identifier."`
	Db      string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListJournalsInput contains parameters for list_journals.
type ListJournalsInput struct {
	Db          string   `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
	Area        string   `query:"area" nullable:"false" doc:"Area filter."`
	HasArticles bool     `query:"has_articles" nullable:"false" doc:"Has-articles filter."`
	Year        int64    `query:"year" nullable:"false" format:"int64" doc:"Publication year filter."`
	Sort        string   `query:"sort" nullable:"false" doc:"Sort expression."`
	Limit       int64    `query:"limit" nullable:"false" format:"int64" doc:"Page size."`
	Offset      int64    `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
	UtdRating   []string `query:"utd_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact UTD grades; OR within a system and AND across systems/other filters. Blank values are invalid; all filter items share a 500-item bound."`
	AbsRating   []string `query:"abs_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact ABS grades, including literal 4*."`
	FmsRating   []string `query:"fms_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS grades."`
	FmscnRating []string `query:"fmscn_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS China grades."`
}

// GetJournalInput contains parameters for get_journal.
type GetJournalInput struct {
	JournalID int64  `path:"journal_id" required:"true" nullable:"false" format:"int64" doc:"Journal identifier."`
	Db        string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListAreasInput contains parameters for list_areas.
type ListAreasInput struct {
	Db string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListDatabasesInput contains parameters for list_databases.
type ListDatabasesInput struct {
}

// ListJournalOptionsInput contains parameters for list_journal_options.
type ListJournalOptionsInput struct {
	Db string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListJournalRatingsInput contains parameters for list_journal_ratings.
type ListJournalRatingsInput struct {
	Db string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// GetAIEndpointsInput contains parameters for get_ai_endpoints.
type GetAIEndpointsInput struct {
}

// GetNotificationSettingsInput contains parameters for get_notification_settings.
type GetNotificationSettingsInput struct {
}

// UpdateNotificationSettingsInput contains parameters for update_notification_settings.
type UpdateNotificationSettingsInput struct {
	Body NotificationSettingsUpdate `required:"true"`
}

// PushWeeklyToTrackingInput contains parameters for push_weekly_to_tracking.
type PushWeeklyToTrackingInput struct {
}

// GetPushWeeklyRunInput contains parameters for get_push_weekly_run.
type GetPushWeeklyRunInput struct {
	RunID string `path:"run_id" required:"true" nullable:"false" doc:"Opaque manual push job id."`
}

// AcknowledgeUnknownPushWeeklyRunInput contains parameters for acknowledge_unknown_push_weekly_run.
type AcknowledgeUnknownPushWeeklyRunInput struct {
	RunID string `path:"run_id" required:"true" nullable:"false" doc:"Opaque ambiguous manual push job id."`
}

// CancelPushWeeklyRunInput contains parameters for cancel_push_weekly_run.
type CancelPushWeeklyRunInput struct {
	RunID string `path:"run_id" required:"true" nullable:"false" doc:"Opaque manual push job id."`
}

// GetPushWeeklyStatusInput contains parameters for get_push_weekly_status.
type GetPushWeeklyStatusInput struct {
}

// StatusInput contains parameters for status.
type StatusInput struct {
}

// GetWeeklyUpdatesInput contains parameters for get_weekly_updates.
type GetWeeklyUpdatesInput struct {
}

// GetWeeklyUpdateArticlesInput contains parameters for get_weekly_update_articles.
type GetWeeklyUpdateArticlesInput struct {
	Db        string `query:"db" required:"true" nullable:"false" maxLength:"255" doc:"Database name or filename under data/index."`
	JournalID int64  `query:"journal_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Positive journal identifier."`
	WindowEnd string `query:"window_end" required:"true" nullable:"false" doc:"RFC3339 fixed weekly window end."`
	Q         string `query:"q" nullable:"false" maxLength:"2048" doc:"Optional simple full-text phrase."`
	Limit     int64  `query:"limit" nullable:"false" minimum:"1" maximum:"200" format:"int64" doc:"Page size from 1 through 200."`
	Cursor    string `query:"cursor" nullable:"false" maxLength:"2048" doc:"Descending date|article_id cursor."`
}

// GetWeeklyUpdatesSummaryInput contains parameters for get_weekly_updates_summary.
type GetWeeklyUpdatesSummaryInput struct {
}

// ListYearsInput contains parameters for list_years.
type ListYearsInput struct {
	Db string `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// LiveInput contains parameters for live.
type LiveInput struct {
}

// ReadyInput contains parameters for ready.
type ReadyInput struct {
}

// ListRelationGraphFavoritesInput contains parameters for list_relation_graph_favorites.
type ListRelationGraphFavoritesInput struct {
	Limit  int64  `query:"limit" nullable:"false" minimum:"1" maximum:"100" default:"50"`
	Cursor string `query:"cursor" nullable:"false" doc:"Opaque owner-bound continuation cursor."`
}

// CreateRelationGraphFavoriteInput contains parameters for create_relation_graph_favorite.
type CreateRelationGraphFavoriteInput struct {
	Body RelationGraphFavoriteCreate `required:"true"`
}

// GetRelationGraphFavoriteInput contains parameters for get_relation_graph_favorite.
type GetRelationGraphFavoriteInput struct {
	GraphID int64 `path:"graph_id" required:"true" nullable:"false" minimum:"1" format:"int64"`
}

// UpdateRelationGraphFavoriteInput contains parameters for update_relation_graph_favorite.
type UpdateRelationGraphFavoriteInput struct {
	GraphID int64                       `path:"graph_id" required:"true" nullable:"false" minimum:"1" format:"int64"`
	Body    RelationGraphFavoriteUpdate `required:"true"`
}

// DeleteRelationGraphFavoriteInput contains parameters for delete_relation_graph_favorite.
type DeleteRelationGraphFavoriteInput struct {
	GraphID int64 `path:"graph_id" required:"true" nullable:"false" minimum:"1" format:"int64"`
}

// UpdateEmailInput contains parameters for update_email.
type UpdateEmailInput struct {
	Body EmailUpdateRequest `required:"true"`
}

// GetAuthProvidersInput contains parameters for get_auth_providers.
type GetAuthProvidersInput struct {
}

// OIDCLoginInput contains parameters for oidc_login.
type OIDCLoginInput struct {
	Next string `query:"next" nullable:"false" default:"/" doc:"Same-site relative path."`
}

// OIDCCallbackInput contains parameters for oidc_callback.
type OIDCCallbackInput struct {
	State string `query:"state" nullable:"false"`
	Code  string `query:"code" nullable:"false"`
	Error string `query:"error" nullable:"false"`
}

// OIDCLinkInput contains parameters for oidc_link.
type OIDCLinkInput struct {
}

// OIDCUnlinkInput contains parameters for oidc_unlink.
type OIDCUnlinkInput struct {
}

// NativeLoginInput contains parameters for native_login.
type NativeLoginInput struct {
	Body NativeLoginRequest `required:"true"`
}

// OIDCNativeLoginInput contains parameters for oidc_native_login.
type OIDCNativeLoginInput struct {
	RedirectURI   string `query:"redirect_uri" required:"true" nullable:"false"`
	CodeChallenge string `query:"code_challenge" required:"true" nullable:"false" minLength:"43" maxLength:"43"`
	State         string `query:"state" required:"true" nullable:"false" pattern:"^[A-Za-z0-9._~-]{16,128}$"`
}

// OIDCNativeExchangeInput contains parameters for oidc_native_exchange.
type OIDCNativeExchangeInput struct {
	Body NativeOIDCExchangeRequest `required:"true"`
}

// TelemetryInput contains parameters for telemetry.
type TelemetryInput struct {
	Period string `query:"period" nullable:"false" default:"24h" enum:"24h,7d,30d" doc:"UTC hour-aligned window including the current partial hour. Default 24h."`
	Kind   string `query:"kind" nullable:"false" enum:"http,mcp,search"`
}

// Register binds a live HTTP operation to its Go request and response contracts.
func Register(api huma.API, binding Operation, handler http.HandlerFunc) error {
	operation := huma.Operation{Method: binding.Method, Path: binding.Path, OperationID: binding.Id, Responses: map[string]*huma.Response{}, Errors: []int{400, 401, 403, 404, 409, 413, 415, 422, 428, 429, 500, 502, 503}}
	switch binding.Id {
	case "list_announcements":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List all announcements for admin management."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListAnnouncementsInput, []AnnouncementInfo](api, operation, handler)
	case "create_announcement":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Create an announcement."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[CreateAnnouncementInput, AnnouncementInfo](api, operation, handler)
	case "update_announcement":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Update an announcement."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[UpdateAnnouncementInput, AnnouncementInfo](api, operation, handler)
	case "delete_announcement":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Delete an announcement."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[DeleteAnnouncementInput, OkResponse](api, operation, handler)
	case "list_invite_codes":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List invite codes."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListInviteCodesInput, []AdminInviteCodeInfo](api, operation, handler)
	case "create_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Create an admin-generated invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n* `body` - Optional bounded lifecycle overrides.\n\n# Returns\n\nCreated invite code metadata."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.RequestBody = &huma.RequestBody{Required: false}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[CreateInviteCodeInput, AdminInviteCodeInfo](api, operation, handler)
	case "revoke_admin_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Irreversibly revoke an invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `code_id` - Invite code row identifier.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nOK response when an unrevoked invite was changed."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[RevokeAdminInviteCodeInput, OkResponse](api, operation, handler)
	case "get_provider_catalog":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Return built-in Provider capabilities and discovered catalog files."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetProviderCatalogInput, ProviderCatalogResponse](api, operation, handler)
	case "list_runtime_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List managed runtime settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListRuntimeSettingsInput, []RuntimeSettingInfo](api, operation, handler)
	case "update_runtime_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Update managed runtime settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[UpdateRuntimeSettingsInput, []RuntimeSettingInfo](api, operation, handler)
	case "list_scheduled_tasks":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List scheduled tasks."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListScheduledTasksInput, []ScheduledTaskInfo](api, operation, handler)
	case "create_scheduled_task":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Create a scheduled task."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[CreateScheduledTaskInput, ScheduledTaskInfo](api, operation, handler)
	case "update_scheduled_task":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Update a scheduled task."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[UpdateScheduledTaskInput, ScheduledTaskInfo](api, operation, handler)
	case "delete_scheduled_task":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Delete a scheduled task."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[DeleteScheduledTaskInput, OkResponse](api, operation, handler)
	case "scheduler_status":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Read durable scheduler cursor, worker heartbeat, and run status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[SchedulerStatusInput, SchedulerStatusResponse](api, operation, handler)
	case "stats":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Return dashboard statistics."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[StatsInput, AdminStatsResponse](api, operation, handler)
	case "list_users":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List all users with admin dashboard counts."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListUsersInput, []AdminUserInfo](api, operation, handler)
	case "delete_user":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Delete a user and associated data."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[DeleteUserInput, OkResponse](api, operation, handler)
	case "set_admin":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Grant or revoke admin status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[SetAdminInput, OkResponse](api, operation, handler)
	case "reset_password":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Reset a user's password."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[ResetPasswordInput, OkResponse](api, operation, handler)
	case "get_announcements":
		operation.DefaultStatus = 200
		operation.Tags = []string{"announcements"}
		operation.Summary = "List enabled announcements from the existing auth database."
		operation.Description = "# Arguments\n\n* `state` - Shared API state containing storage paths.\n\n# Returns\n\nJSON announcement list ordered like the Python API."
		operation.Security = []map[string][]string{}
		registerOperation[GetAnnouncementsInput, []AnnouncementInfo](api, operation, handler)
	case "list_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List articles with filters, FTS, and cursor pagination."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `raw_query` - Raw query string so repeated fields remain available.\n\n# Returns\n\nPaginated articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListArticlesInput, ArticlePage](api, operation, handler)
	case "get_article":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return one article record."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nArticle record."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetArticleInput, ArticleRecord](api, operation, handler)
	case "list_related_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return articles related to one article."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Source article identifier.\n* `query` - Database selector and result limit.\n\n# Returns\n\nArticles ranked by FTS5 bm25 similarity to the source title over titles and abstracts, excluding the source."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListRelatedArticlesInput, []ArticleRecord](api, operation, handler)
	case "redirect_article_abstract":
		operation.DefaultStatus = 307
		operation.Tags = []string{"index"}
		operation.Summary = "Resolve and redirect to an article abstract page online."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nNo-store temporary redirect."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[RedirectArticleAbstractInput, noBody](api, operation, handler)
	case "get_article_access":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return access actions for one article."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nArticle access response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetArticleAccessInput, ArticleAccessResponse](api, operation, handler)
	case "redirect_article_fulltext":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Redirect to an article full-text target."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nTemporary redirect response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.Responses["307"] = &huma.Response{Description: "Temporary redirect to a full-text target."}
		registerOperation[RedirectArticleFulltextInput, noBody](api, operation, handler)
	case "change_password":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Change the current user's password."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `body` - Password change request.\n\n# Returns\n\nOK response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[ChangePasswordInput, OkResponse](api, operation, handler)
	case "get_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Get the invite code generated by the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nInvite code response or null."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetInviteCodeInput, *InviteCodeResponse](api, operation, handler)
	case "generate_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Generate a one-time invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nInvite code response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GenerateInviteCodeInput, InviteCodeResponse](api, operation, handler)
	case "revoke_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Revoke the current user's unrevoked invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nOK response when an invite was revoked."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[RevokeInviteCodeInput, OkResponse](api, operation, handler)
	case "rotate_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Revoke any prior issuance and create one replacement invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nNewly issued replacement invite code."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[RotateInviteCodeInput, InviteCodeResponse](api, operation, handler)
	case "check_invite_required":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Return whether registration requires an invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n\n# Returns\n\nInvite requirement response."
		operation.Security = []map[string][]string{}
		registerOperation[CheckInviteRequiredInput, InviteRequiredResponse](api, operation, handler)
	case "login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Sign in with username or email and password."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `body` - Login request.\n\n# Returns\n\nLogin response without the raw token."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[LoginInput, LoginResponse](api, operation, handler)
	case "logout":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Logout the current session token and clear the browser cookie."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nLogout response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[LogoutInput, LogoutResponse](api, operation, handler)
	case "logout_all":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Revoke every session and personal access token for the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nLogout response after every token is durably revoked."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[LogoutAllInput, LogoutResponse](api, operation, handler)
	case "get_me":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Return the current authenticated user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nCurrent user response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetMeInput, UserResponse](api, operation, handler)
	case "register":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Register a new user account."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `body` - Registration request.\n\n# Returns\n\nCreated user response."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[RegisterInput, UserResponse](api, operation, handler)
	case "get_tokens":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "List active access tokens for the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nActive token metadata."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetTokensInput, []TokenInfo](api, operation, handler)
	case "create_token":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Create an access token for the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `body` - Token creation request.\n\n# Returns\n\nCreated token response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[CreateTokenInput, TokenCreateResponse](api, operation, handler)
	case "delete_token":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Delete an access token by row id."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `token_id` - Token row identifier.\n\n# Returns\n\nOK response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[DeleteTokenInput, OkResponse](api, operation, handler)
	case "list_cfp_journals":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cfp"}
		operation.Summary = "List every maintained journal, including those with no indexed articles or CFP adapter."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListCfpJournalsInput, CfpCatalogResponse](api, operation, handler)
	case "list_cfp_notices":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cfp"}
		operation.Summary = "Read original notices and backend-calculated availability at a cursor's fixed instant."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListCfpNoticesInput, CfpNoticePage](api, operation, handler)
	case "poll_login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Poll a QR login session."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `body` - Polling parameters.\n\n# Returns\n\nPolling result and safe session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[PollLoginInput, CnkiLoginPollResponse](api, operation, handler)
	case "start_login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Start a QR login session."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nQR login challenge and safe session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[StartLoginInput, CnkiLoginStartResponse](api, operation, handler)
	case "get_session":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Return the current user's CNKI session status."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nSafe CNKI session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetSessionInput, CnkiSessionStatusResponse](api, operation, handler)
	case "clear_session":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Clear the current user's CNKI session."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nEmpty safe CNKI session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ClearSessionInput, CnkiSessionStatusResponse](api, operation, handler)
	case "check_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Check which folders contain an article."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[CheckFavoriteInput, []FavoriteCheckResponse](api, operation, handler)
	case "check_favorites_batch":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Batch check which folders contain articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[CheckFavoritesBatchInput, []FavoriteBatchCheckResponse](api, operation, handler)
	case "list_folders":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "List all folders for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListFoldersInput, []FolderResponse](api, operation, handler)
	case "create_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Create a new folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[CreateFolderInput, FolderResponse](api, operation, handler)
	case "rename_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Rename an existing folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[RenameFolderInput, OkResponse](api, operation, handler)
	case "delete_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Delete an existing folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[DeleteFolderInput, OkResponse](api, operation, handler)
	case "list_folder_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "List favorited articles in a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListFolderArticlesInput, []FavoriteArticleResponse](api, operation, handler)
	case "add_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Add one favorite to a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[AddFavoriteInput, FavoriteResponse](api, operation, handler)
	case "bulk_add":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Bulk add favorites to a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[BulkAddInput, FavoriteBulkAddResult](api, operation, handler)
	case "bulk_move":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Bulk move favorites between folders."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[BulkMoveInput, FavoriteBulkResult](api, operation, handler)
	case "bulk_remove":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Bulk remove favorites from a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[BulkRemoveInput, FavoriteBulkResult](api, operation, handler)
	case "list_folder_article_page":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Return one stable cursor page from an owned favorite folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListFolderArticlePageInput, FavoriteArticlePage](api, operation, handler)
	case "remove_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Remove one favorite from a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[RemoveFavoriteInput, OkResponse](api, operation, handler)
	case "folder_count":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Get the favorite count for a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[FolderCountInput, noBody](api, operation, handler)
	case "export_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Export one folder's favorites in a citation format."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ExportFolderInput, noBody](api, operation, handler)
	case "get_tracking":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Get the current tracking folder for the user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetTrackingInput, FavoriteTrackingResponse](api, operation, handler)
	case "set_tracking":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Set a folder as the current tracking folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[SetTrackingInput, OkResponse](api, operation, handler)
	case "list_issues":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List issues with filters and offset pagination."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Issue list filters.\n\n# Returns\n\nPaginated issues."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListIssuesInput, IssuePage](api, operation, handler)
	case "get_issue":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return one issue record."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `issue_id` - Issue identifier.\n* `query` - Database selector.\n\n# Returns\n\nIssue record."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetIssueInput, IssueRecord](api, operation, handler)
	case "list_journals":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List journals with filters and offset pagination."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Journal list filters.\n* `raw_query` - Repeated rating fields; scalar parsing retains its existing contract.\n\n# Returns\n\nPaginated journals."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListJournalsInput, JournalPage](api, operation, handler)
	case "get_journal":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return one journal record."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `journal_id` - Journal identifier.\n* `query` - Database selector.\n\n# Returns\n\nJournal record."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetJournalInput, JournalRecord](api, operation, handler)
	case "list_areas":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List journal area counts."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nJournal area counts."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListAreasInput, []ValueCount](api, operation, handler)
	case "list_databases":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List index database filenames."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nJSON list of database filenames."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListDatabasesInput, []string](api, operation, handler)
	case "list_journal_options":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List journal selector options."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nJournal option records."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListJournalOptionsInput, []JournalOption](api, operation, handler)
	case "list_journal_ratings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List available journal grades with database-wide journal counts."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nFour exact rating groups, excluding unrated journals and counting journals without articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListJournalRatingsInput, JournalRatingOptions](api, operation, handler)
	case "get_ai_endpoints":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get the administrator-approved AI endpoint catalog."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetAIEndpointsInput, []string](api, operation, handler)
	case "get_notification_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get the user's notification settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetNotificationSettingsInput, *NotificationSettingsResponse](api, operation, handler)
	case "update_notification_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Create or update the user's notification settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[UpdateNotificationSettingsInput, NotificationSettingsResponse](api, operation, handler)
	case "push_weekly_to_tracking":
		operation.DefaultStatus = 202
		operation.Tags = []string{"tracking"}
		operation.Summary = "Start one manual weekly-push job for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[PushWeeklyToTrackingInput, ManualWeeklyPushStatus](api, operation, handler)
	case "get_push_weekly_run":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get one durable manual weekly-push run visible to its owner or an administrator."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetPushWeeklyRunInput, ManualWeeklyPushStatus](api, operation, handler)
	case "acknowledge_unknown_push_weekly_run":
		operation.DefaultStatus = 202
		operation.Tags = []string{"tracking"}
		operation.Summary = "Acknowledge the latest ambiguous manual push and enqueue a safe new attempt."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[AcknowledgeUnknownPushWeeklyRunInput, ManualWeeklyPushStatus](api, operation, handler)
	case "cancel_push_weekly_run":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Request cancellation of one durable manual weekly-push run."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[CancelPushWeeklyRunInput, ManualWeeklyPushStatus](api, operation, handler)
	case "get_push_weekly_status":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get the current manual weekly-push job status for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetPushWeeklyStatusInput, ManualWeeklyPushStatus](api, operation, handler)
	case "status":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get tracking status for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[StatusInput, TrackingStatusResponse](api, operation, handler)
	case "get_weekly_updates":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List weekly article updates grouped by database and journal."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nWeekly update response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetWeeklyUpdatesInput, WeeklyUpdatesResponse](api, operation, handler)
	case "get_weekly_update_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List one bounded page of weekly articles from a fixed manifest window."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `raw_query` - Raw weekly page query.\n\n# Returns\n\nCursor-paginated weekly articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetWeeklyUpdateArticlesInput, WeeklyArticlePage](api, operation, handler)
	case "get_weekly_updates_summary":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Summarize weekly updates without returning article bodies."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nFixed-window database and journal counts."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetWeeklyUpdatesSummaryInput, WeeklyUpdatesSummaryResponse](api, operation, handler)
	case "list_years":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List publication year summaries."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nYear summary records."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListYearsInput, []YearSummary](api, operation, handler)
	case "live":
		operation.DefaultStatus = 200
		operation.Tags = []string{"health"}
		operation.Summary = "Return application event-loop liveness."
		operation.Description = "# Returns\n\nJSON health response."
		operation.Security = []map[string][]string{}
		registerOperation[LiveInput, HealthResponse](api, operation, handler)
	case "ready":
		operation.DefaultStatus = 200
		operation.Tags = []string{"health"}
		operation.Summary = "Return whether the embedded scheduler has a recent persisted heartbeat."
		operation.Description = "# Arguments\n\n* `state` - API state containing the shared scheduler database path.\n\n# Returns\n\nHTTP 200 when ready or 503 until the embedded scheduler heartbeat is healthy."
		operation.Security = []map[string][]string{}
		registerOperation[ReadyInput, HealthResponse](api, operation, handler)
	case "list_relation_graph_favorites":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "List saved relation graph summaries."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[ListRelationGraphFavoritesInput, RelationGraphFavoritePage](api, operation, handler)
	case "create_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Save a relation graph snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 3 * 1024 * 1024
		registerOperation[CreateRelationGraphFavoriteInput, RelationGraphFavorite](api, operation, handler)
	case "get_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Read an owned relation graph snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[GetRelationGraphFavoriteInput, RelationGraphFavorite](api, operation, handler)
	case "update_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Update an owned relation graph title or snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 3 * 1024 * 1024
		registerOperation[UpdateRelationGraphFavoriteInput, RelationGraphFavorite](api, operation, handler)
	case "delete_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Delete an owned relation graph snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[DeleteRelationGraphFavoriteInput, OkResponse](api, operation, handler)
	case "update_email":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Update the local account email."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[UpdateEmailInput, UserResponse](api, operation, handler)
	case "get_auth_providers":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "List enabled sign-in providers."
		operation.Security = []map[string][]string{}
		registerOperation[GetAuthProvidersInput, AuthProvidersResponse](api, operation, handler)
	case "oidc_login":
		operation.DefaultStatus = 302
		operation.Tags = []string{"auth"}
		operation.Summary = "Start OIDC sign-in."
		operation.Security = []map[string][]string{}
		registerOperation[OIDCLoginInput, noBody](api, operation, handler)
	case "oidc_callback":
		operation.DefaultStatus = 302
		operation.Tags = []string{"auth"}
		operation.Summary = "Complete OIDC sign-in or linking."
		operation.Security = []map[string][]string{}
		registerOperation[OIDCCallbackInput, noBody](api, operation, handler)
	case "oidc_link":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Link an OIDC identity to a local account."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[OIDCLinkInput, OIDCLinkResponse](api, operation, handler)
	case "oidc_unlink":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Unlink the local account OIDC identity."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[OIDCUnlinkInput, UserResponse](api, operation, handler)
	case "native_login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Sign in a native device with username or email and password."
		operation.Description = "Issues a new personal access token named device:<device_name> with a fixed 90-day TTL. Uses the password login rate limits and credential generation fence. Does not set cookies or replace the browser login session. Send the token as Authorization: Bearer; POST /api/auth/logout revokes that token."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[NativeLoginInput, NativeLoginResponse](api, operation, handler)
	case "oidc_native_login":
		operation.DefaultStatus = 302
		operation.Tags = []string{"auth"}
		operation.Summary = "Start native OIDC sign-in using signed server state and client PKCE."
		operation.Security = []map[string][]string{}
		registerOperation[OIDCNativeLoginInput, noBody](api, operation, handler)
	case "oidc_native_exchange":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Exchange a one-time native OIDC authorization code for a 90-day device token."
		operation.Description = "Issues a new personal access token named device:<device_name> with a fixed 90-day TTL. Uses the password login rate limits and credential generation fence. Does not set cookies or replace the browser login session. Send the token as Authorization: Bearer; POST /api/auth/logout revokes that token."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
		registerOperation[OIDCNativeExchangeInput, NativeLoginResponse](api, operation, handler)
	case "telemetry":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Return anonymous hourly load and search-shape telemetry."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		registerOperation[TelemetryInput, TelemetryResponse](api, operation, handler)
	default:
		return fmt.Errorf("unknown API operation: %s", binding.Id)
	}
	return nil
}
