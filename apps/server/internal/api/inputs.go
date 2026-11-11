package api

import domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"

// ListAnnouncementsInput contains parameters for list_announcements.
type ListAnnouncementsInput struct {
	RequestMetadata
}

// CreateAnnouncementInput contains parameters for create_announcement.
type CreateAnnouncementInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body AnnouncementCreate `required:"true"`
}

// UpdateAnnouncementInput contains parameters for update_announcement.
type UpdateAnnouncementInput struct {
	RequestMetadata
	// AnnouncementID describes announcement row identifier.
	AnnouncementID int64 `path:"announcement_id" required:"true" nullable:"false" format:"int64" doc:"Announcement row identifier."`
	// Body contains the decoded operation payload.
	Body AnnouncementUpdate `required:"true"`
}

// DeleteAnnouncementInput contains parameters for delete_announcement.
type DeleteAnnouncementInput struct {
	RequestMetadata
	// AnnouncementID describes announcement row identifier.
	AnnouncementID int64 `path:"announcement_id" required:"true" nullable:"false" format:"int64" doc:"Announcement row identifier."`
}

// ListInviteCodesInput contains parameters for list_invite_codes.
type ListInviteCodesInput struct {
	RequestMetadata
}

// CreateInviteCodeInput contains parameters for create_invite_code.
type CreateInviteCodeInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body *AdminInviteCodeCreate
}

// RevokeAdminInviteCodeInput contains parameters for revoke_admin_invite_code.
type RevokeAdminInviteCodeInput struct {
	RequestMetadata
	// CodeID describes invite code row identifier.
	CodeID int64 `path:"code_id" required:"true" nullable:"false" format:"int64" doc:"Invite code row identifier."`
}

// GetProviderCatalogInput contains parameters for get_provider_catalog.
type GetProviderCatalogInput struct {
	RequestMetadata
}

// ListRuntimeSettingsInput contains parameters for list_runtime_settings.
type ListRuntimeSettingsInput struct {
	RequestMetadata
}

// UpdateRuntimeSettingsInput contains parameters for update_runtime_settings.
type UpdateRuntimeSettingsInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body RuntimeSettingsUpdate `required:"true"`
}

// ListScheduledTasksInput contains parameters for list_scheduled_tasks.
type ListScheduledTasksInput struct {
	RequestMetadata
}

// CreateScheduledTaskInput contains parameters for create_scheduled_task.
type CreateScheduledTaskInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body ScheduledTaskCreate `required:"true"`
}

// UpdateScheduledTaskInput contains parameters for update_scheduled_task.
type UpdateScheduledTaskInput struct {
	RequestMetadata
	// TaskID describes scheduled task row identifier.
	TaskID int64 `path:"task_id" required:"true" nullable:"false" format:"int64" doc:"Scheduled task row identifier."`
	// Body contains the decoded operation payload.
	Body ScheduledTaskUpdate `required:"true"`
}

// DeleteScheduledTaskInput contains parameters for delete_scheduled_task.
type DeleteScheduledTaskInput struct {
	RequestMetadata
	// TaskID describes scheduled task row identifier.
	TaskID int64 `path:"task_id" required:"true" nullable:"false" format:"int64" doc:"Scheduled task row identifier."`
}

// SchedulerStatusInput contains parameters for scheduler_status.
type SchedulerStatusInput struct {
	RequestMetadata
}

// StatsInput contains parameters for stats.
type StatsInput struct {
	RequestMetadata
}

// ListUsersInput contains parameters for list_users.
type ListUsersInput struct {
	RequestMetadata
}

// DeleteUserInput contains parameters for delete_user.
type DeleteUserInput struct {
	RequestMetadata
	// UserID describes user row identifier.
	UserID int64 `path:"user_id" required:"true" nullable:"false" format:"int64" doc:"User row identifier."`
}

// SetAdminInput contains parameters for set_admin.
type SetAdminInput struct {
	RequestMetadata
	// UserID describes user row identifier.
	UserID int64 `path:"user_id" required:"true" nullable:"false" format:"int64" doc:"User row identifier."`
	// Body contains the decoded operation payload.
	Body AdminSetAdmin `required:"true"`
}

// ResetPasswordInput contains parameters for reset_password.
type ResetPasswordInput struct {
	RequestMetadata
	// UserID describes user row identifier.
	UserID int64 `path:"user_id" required:"true" nullable:"false" format:"int64" doc:"User row identifier."`
	// Body contains the decoded operation payload.
	Body AdminResetPassword `required:"true"`
}

// GetAnnouncementsInput contains parameters for get_announcements.
type GetAnnouncementsInput struct {
	RequestMetadata
}

// ListArticlesInput contains parameters for list_articles.
type ListArticlesInput struct {
	RequestMetadata
	// DB describes database name or filename under data/index.
	DB optional[string] `query:"db" nullable:"false" maxLength:"255" doc:"Database name or filename under data/index."`
	// JournalID describes repeated journal identifier filters.
	JournalID []int64 `query:"journal_id,explode" nullable:"false" maxItems:"500" doc:"Repeated journal identifier filters."`
	// Area describes repeated area filters.
	Area []string `query:"area,explode" nullable:"false" maxItems:"500" doc:"Repeated area filters."`
	// UTDRating describes repeated exact UTD grades; OR within a system and AND across systems/other filters. Blank values are invalid; all filter items share a 500-item bound.
	UTDRating []string `query:"utd_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact UTD grades; OR within a system and AND across systems/other filters. Blank values are invalid; all filter items share a 500-item bound."`
	// ABSRating describes repeated exact ABS grades, including literal 4*.
	ABSRating []string `query:"abs_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact ABS grades, including literal 4*."`
	// FMSRating describes repeated exact FMS grades.
	FMSRating []string `query:"fms_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS grades."`
	// FMSCNRating describes repeated exact FMS China grades.
	FMSCNRating []string `query:"fmscn_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS China grades."`
	// IssueID describes issue identifier filter.
	IssueID optional[int64] `query:"issue_id" nullable:"false" format:"int64" doc:"Issue identifier filter."`
	// Year describes publication year filter.
	Year optional[int64] `query:"year" nullable:"false" format:"int64" doc:"Publication year filter."`
	// InPress describes in-press filter.
	InPress optional[bool] `query:"in_press" nullable:"false" doc:"In-press filter."`
	// OpenAccess describes open-access filter.
	OpenAccess optional[bool] `query:"open_access" nullable:"false" doc:"Open-access filter."`
	// DateFrom describes start date filter.
	DateFrom optional[string] `query:"date_from" nullable:"false" maxLength:"2048" doc:"Start date filter."`
	// DateTo describes end date filter.
	DateTo optional[string] `query:"date_to" nullable:"false" maxLength:"2048" doc:"End date filter."`
	// DOI describes dOI filter.
	DOI optional[string] `query:"doi" nullable:"false" maxLength:"2048" doc:"DOI filter."`
	// PMID describes pubMed identifier filter.
	PMID optional[string] `query:"pmid" nullable:"false" maxLength:"2048" doc:"PubMed identifier filter."`
	// Q describes full-text query.
	Q optional[string] `query:"q" nullable:"false" maxLength:"2048" doc:"Full-text query."`
	// SearchMode describes full-text query interpretation mode; defaults to simple.
	SearchMode optional[domain.SearchMode] `query:"search_mode" nullable:"false" enum:"simple,advanced" doc:"Full-text query interpretation mode; defaults to simple."`
	// Sort describes sort expression.
	Sort optional[string] `query:"sort" nullable:"false" maxLength:"2048" doc:"Sort expression."`
	// Limit describes page size.
	Limit optional[int64] `query:"limit" nullable:"false" format:"int64" doc:"Page size."`
	// Offset describes offset row count.
	Offset optional[int64] `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
	// Cursor describes keyset cursor.
	Cursor optional[string] `query:"cursor" nullable:"false" maxLength:"2048" doc:"Keyset cursor."`
	// IncludeTotal reports whether to include the full filtered row count; defaults to true without a cursor and false with a cursor.
	IncludeTotal optional[bool] `query:"include_total" nullable:"false" doc:"Whether to include the full filtered row count; defaults to true without a cursor and false with a cursor."`
}

// GetArticleInput contains parameters for get_article.
type GetArticleInput struct {
	RequestMetadata
	// ArticleID describes article identifier.
	ArticleID int64 `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListRelatedArticlesInput contains parameters for list_related_articles.
type ListRelatedArticlesInput struct {
	RequestMetadata
	// ArticleID describes source article identifier.
	ArticleID int64 `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Source article identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
	// Limit describes maximum related articles, from 1 to 30. Defaults to 10.
	Limit optional[int64] `query:"limit" nullable:"false" format:"int64" doc:"Maximum related articles, from 1 to 30. Defaults to 10."`
}

// RedirectArticleAbstractInput contains parameters for redirect_article_abstract.
type RedirectArticleAbstractInput struct {
	RequestMetadata
	// ArticleID describes article identifier.
	ArticleID int64 `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// GetArticleAccessInput contains parameters for get_article_access.
type GetArticleAccessInput struct {
	RequestMetadata
	// ArticleID describes article identifier.
	ArticleID int64 `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// RedirectArticleFulltextInput contains parameters for redirect_article_fulltext.
type RedirectArticleFulltextInput struct {
	RequestMetadata
	// ArticleID describes article identifier.
	ArticleID int64 `path:"article_id" required:"true" nullable:"false" format:"int64" doc:"Article identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ChangePasswordInput contains parameters for change_password.
type ChangePasswordInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body ChangePasswordRequest `required:"true"`
}

// GetInviteCodeInput contains parameters for get_invite_code.
type GetInviteCodeInput struct {
	RequestMetadata
}

// GenerateInviteCodeInput contains parameters for generate_invite_code.
type GenerateInviteCodeInput struct {
	RequestMetadata
}

// RevokeInviteCodeInput contains parameters for revoke_invite_code.
type RevokeInviteCodeInput struct {
	RequestMetadata
}

// RotateInviteCodeInput contains parameters for rotate_invite_code.
type RotateInviteCodeInput struct {
	RequestMetadata
}

// CheckInviteRequiredInput contains parameters for check_invite_required.
type CheckInviteRequiredInput struct {
	RequestMetadata
}

// LoginInput contains parameters for login.
type LoginInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body LoginRequest `required:"true"`
}

// LogoutInput contains parameters for logout.
type LogoutInput struct {
	RequestMetadata
}

// LogoutAllInput contains parameters for logout_all.
type LogoutAllInput struct {
	RequestMetadata
}

// GetMeInput contains parameters for get_me.
type GetMeInput struct {
	RequestMetadata
}

// RegisterInput contains parameters for register.
type RegisterInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body RegisterRequest `required:"true"`
}

// GetTokensInput contains parameters for get_tokens.
type GetTokensInput struct {
	RequestMetadata
}

// CreateTokenInput contains parameters for create_token.
type CreateTokenInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body TokenCreateRequest `required:"true"`
}

// DeleteTokenInput contains parameters for delete_token.
type DeleteTokenInput struct {
	RequestMetadata
	// TokenID describes token row identifier.
	TokenID int64 `path:"token_id" required:"true" nullable:"false" format:"int64" doc:"Token row identifier."`
}

// ListCFPJournalsInput contains parameters for list_cfp_journals.
type ListCFPJournalsInput struct {
	RequestMetadata
	// DB describes database filename, such as english_journals.sqlite.
	DB string `query:"db" required:"true" nullable:"false" doc:"Database filename, such as english_journals.sqlite."`
	// Q describes case-insensitive journal title, ID or subject search, limited to 256 characters.
	Q optional[string] `query:"q" nullable:"false" doc:"Case-insensitive journal title, ID or subject search, limited to 256 characters."`
}

// ListCFPNoticesInput contains parameters for list_cfp_notices.
type ListCFPNoticesInput struct {
	RequestMetadata
	// CatalogID describes canonical maintained catalog ID or its alias.
	CatalogID string `path:"catalog_id" required:"true" nullable:"false" doc:"Canonical maintained catalog ID or its alias."`
	// DB describes database filename containing the maintained journal.
	DB string `query:"db" required:"true" nullable:"false" doc:"Database filename containing the maintained journal."`
	// IncludeClosed describes include closed and historical records; defaults to false.
	IncludeClosed optional[bool] `query:"include_closed" nullable:"false" doc:"Include closed and historical records; defaults to false."`
	// Limit describes page size, default 50 and maximum 200.
	Limit optional[int64] `query:"limit" nullable:"false" minimum:"0" doc:"Page size, default 50 and maximum 200."`
	// Cursor describes opaque cursor returned by this journal/filter's previous page.
	Cursor optional[string] `query:"cursor" nullable:"false" doc:"Opaque cursor returned by this journal/filter's previous page."`
}

// PollLoginInput contains parameters for poll_login.
type PollLoginInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body CNKILoginPollRequest `required:"true"`
}

// StartLoginInput contains parameters for start_login.
type StartLoginInput struct {
	RequestMetadata
}

// GetSessionInput contains parameters for get_session.
type GetSessionInput struct {
	RequestMetadata
}

// ClearSessionInput contains parameters for clear_session.
type ClearSessionInput struct {
	RequestMetadata
}

// CheckFavoriteInput contains parameters for check_favorite.
type CheckFavoriteInput struct {
	RequestMetadata
	// ArticleID describes article identifier.
	ArticleID int64 `query:"article_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Article identifier."`
	// DBName describes source database name.
	DBName optional[string] `query:"db_name" nullable:"false" maxLength:"255" doc:"Source database name."`
}

// CheckFavoritesBatchInput contains parameters for check_favorites_batch.
type CheckFavoritesBatchInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body FavoriteBatchCheckRequest `required:"true"`
}

// ListFoldersInput contains parameters for list_folders.
type ListFoldersInput struct {
	RequestMetadata
}

// CreateFolderInput contains parameters for create_folder.
type CreateFolderInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body FolderCreate `required:"true"`
}

// RenameFolderInput contains parameters for rename_folder.
type RenameFolderInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Body contains the decoded operation payload.
	Body FolderRename `required:"true"`
}

// DeleteFolderInput contains parameters for delete_folder.
type DeleteFolderInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
}

// ListFolderArticlesInput contains parameters for list_folder_articles.
type ListFolderArticlesInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Limit describes maximum row count.
	Limit int64 `query:"limit" default:"100" nullable:"false" minimum:"1" maximum:"500" format:"int64" doc:"Maximum row count."`
	// Offset describes offset row count.
	Offset int64 `query:"offset" default:"0" nullable:"false" minimum:"0" format:"int64" doc:"Offset row count."`
}

// AddFavoriteInput contains parameters for add_favorite.
type AddFavoriteInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Body contains the decoded operation payload.
	Body FavoriteAdd `required:"true"`
}

// BulkAddInput contains parameters for bulk_add.
type BulkAddInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Body contains the decoded operation payload.
	Body FavoriteBulkAdd `required:"true"`
}

// BulkMoveInput contains parameters for bulk_move.
type BulkMoveInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Body contains the decoded operation payload.
	Body FavoriteBulkMove `required:"true"`
}

// BulkRemoveInput contains parameters for bulk_remove.
type BulkRemoveInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Body contains the decoded operation payload.
	Body FavoriteBulkRemove `required:"true"`
}

// ListFolderArticlePageInput contains parameters for list_folder_article_page.
type ListFolderArticlePageInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Limit describes maximum row count, defaulting to fifty.
	Limit int64 `query:"limit" default:"50" nullable:"false" minimum:"1" maximum:"500" format:"int64" doc:"Maximum row count, defaulting to fifty."`
	// Cursor describes opaque continuation bound to this user and folder.
	Cursor optional[string] `query:"cursor" nullable:"false" maxLength:"128" doc:"Opaque continuation bound to this user and folder."`
}

// RemoveFavoriteInput contains parameters for remove_favorite.
type RemoveFavoriteInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// ArticleID describes article identifier.
	ArticleID int64 `path:"article_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Article identifier."`
	// DBName describes source database name.
	DBName optional[string] `query:"db_name" nullable:"false" maxLength:"255" doc:"Source database name."`
}

// FolderCountInput contains parameters for folder_count.
type FolderCountInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
}

// ExportFolderInput contains parameters for export_folder.
type ExportFolderInput struct {
	RequestMetadata
	// FolderID describes folder row identifier.
	FolderID int64 `path:"folder_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier."`
	// Format describes export format.
	Format string `query:"format" nullable:"false" default:"bibtex" enum:"bibtex,ris,endnote" doc:"Export format."`
}

// GetTrackingInput contains parameters for get_tracking.
type GetTrackingInput struct {
	RequestMetadata
}

// SetTrackingInput contains parameters for set_tracking.
type SetTrackingInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body TrackingSetRequest `required:"true"`
}

// ListIssuesInput contains parameters for list_issues.
type ListIssuesInput struct {
	RequestMetadata
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
	// JournalID describes journal identifier filter.
	JournalID optional[int64] `query:"journal_id" nullable:"false" format:"int64" doc:"Journal identifier filter."`
	// Year describes publication year filter.
	Year optional[int64] `query:"year" nullable:"false" format:"int64" doc:"Publication year filter."`
	// Sort describes sort expression.
	Sort optional[string] `query:"sort" nullable:"false" doc:"Sort expression."`
	// Limit describes page size.
	Limit optional[int64] `query:"limit" nullable:"false" format:"int64" doc:"Page size."`
	// Offset describes offset row count.
	Offset optional[int64] `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
}

// GetIssueInput contains parameters for get_issue.
type GetIssueInput struct {
	RequestMetadata
	// IssueID describes issue identifier.
	IssueID int64 `path:"issue_id" required:"true" nullable:"false" format:"int64" doc:"Issue identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListJournalsInput contains parameters for list_journals.
type ListJournalsInput struct {
	RequestMetadata
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
	// Area describes area filter.
	Area optional[string] `query:"area" nullable:"false" doc:"Area filter."`
	// HasArticles describes has-articles filter.
	HasArticles optional[bool] `query:"has_articles" nullable:"false" doc:"Has-articles filter."`
	// Year describes publication year filter.
	Year optional[int64] `query:"year" nullable:"false" format:"int64" doc:"Publication year filter."`
	// Sort describes sort expression.
	Sort optional[string] `query:"sort" nullable:"false" doc:"Sort expression."`
	// Limit describes page size.
	Limit optional[int64] `query:"limit" nullable:"false" format:"int64" doc:"Page size."`
	// Offset describes offset row count.
	Offset optional[int64] `query:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
	// UTDRating describes repeated exact UTD grades; OR within a system and AND across systems/other filters. Blank values are invalid; all filter items share a 500-item bound.
	UTDRating []string `query:"utd_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact UTD grades; OR within a system and AND across systems/other filters. Blank values are invalid; all filter items share a 500-item bound."`
	// ABSRating describes repeated exact ABS grades, including literal 4*.
	ABSRating []string `query:"abs_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact ABS grades, including literal 4*."`
	// FMSRating describes repeated exact FMS grades.
	FMSRating []string `query:"fms_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS grades."`
	// FMSCNRating describes repeated exact FMS China grades.
	FMSCNRating []string `query:"fmscn_rating,explode" nullable:"false" maxItems:"500" doc:"Repeated exact FMS China grades."`
}

// GetJournalInput contains parameters for get_journal.
type GetJournalInput struct {
	RequestMetadata
	// JournalID describes journal identifier.
	JournalID int64 `path:"journal_id" required:"true" nullable:"false" format:"int64" doc:"Journal identifier."`
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListAreasInput contains parameters for list_areas.
type ListAreasInput struct {
	RequestMetadata
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListDatabasesInput contains parameters for list_databases.
type ListDatabasesInput struct {
	RequestMetadata
}

// ListJournalOptionsInput contains parameters for list_journal_options.
type ListJournalOptionsInput struct {
	RequestMetadata
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// ListJournalRatingsInput contains parameters for list_journal_ratings.
type ListJournalRatingsInput struct {
	RequestMetadata
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// GetAIEndpointsInput contains parameters for get_ai_endpoints.
type GetAIEndpointsInput struct {
	RequestMetadata
}

// GetNotificationSettingsInput contains parameters for get_notification_settings.
type GetNotificationSettingsInput struct {
	RequestMetadata
}

// UpdateNotificationSettingsInput contains parameters for update_notification_settings.
type UpdateNotificationSettingsInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body NotificationSettingsUpdate `required:"true"`
}

// PushWeeklyToTrackingInput contains parameters for push_weekly_to_tracking.
type PushWeeklyToTrackingInput struct {
	RequestMetadata
}

// GetPushWeeklyRunInput contains parameters for get_push_weekly_run.
type GetPushWeeklyRunInput struct {
	RequestMetadata
	// RunID describes opaque manual push job id.
	RunID string `path:"run_id" required:"true" nullable:"false" pattern:"^[A-Fa-f0-9]{32}$" doc:"Opaque manual push job id."`
}

// AcknowledgeUnknownPushWeeklyRunInput contains parameters for acknowledge_unknown_push_weekly_run.
type AcknowledgeUnknownPushWeeklyRunInput struct {
	RequestMetadata
	// RunID describes opaque ambiguous manual push job id.
	RunID string `path:"run_id" required:"true" nullable:"false" pattern:"^[A-Fa-f0-9]{32}$" doc:"Opaque ambiguous manual push job id."`
}

// CancelPushWeeklyRunInput contains parameters for cancel_push_weekly_run.
type CancelPushWeeklyRunInput struct {
	RequestMetadata
	// RunID describes opaque manual push job id.
	RunID string `path:"run_id" required:"true" nullable:"false" pattern:"^[A-Fa-f0-9]{32}$" doc:"Opaque manual push job id."`
}

// GetPushWeeklyStatusInput contains parameters for get_push_weekly_status.
type GetPushWeeklyStatusInput struct {
	RequestMetadata
}

// StatusInput contains parameters for status.
type StatusInput struct {
	RequestMetadata
}

// GetWeeklyUpdatesInput contains parameters for get_weekly_updates.
type GetWeeklyUpdatesInput struct {
	RequestMetadata
}

// GetWeeklyUpdateArticlesInput contains parameters for get_weekly_update_articles.
type GetWeeklyUpdateArticlesInput struct {
	RequestMetadata
	// DB describes database name or filename under data/index.
	DB string `query:"db" required:"true" nullable:"false" maxLength:"255" doc:"Database name or filename under data/index."`
	// JournalID describes positive journal identifier.
	JournalID int64 `query:"journal_id" required:"true" nullable:"false" minimum:"1" format:"int64" doc:"Positive journal identifier."`
	// WindowEnd describes rFC3339 fixed weekly window end.
	WindowEnd string `query:"window_end" required:"true" nullable:"false" doc:"RFC3339 fixed weekly window end."`
	// Q describes optional simple full-text phrase.
	Q optional[string] `query:"q" nullable:"false" maxLength:"2048" doc:"Optional simple full-text phrase."`
	// Limit describes page size from 1 through 200.
	Limit optional[int64] `query:"limit" nullable:"false" minimum:"1" maximum:"200" format:"int64" doc:"Page size from 1 through 200."`
	// Cursor describes descending date|article_id cursor.
	Cursor optional[string] `query:"cursor" nullable:"false" maxLength:"2048" doc:"Descending date|article_id cursor."`
}

// GetWeeklyUpdatesSummaryInput contains parameters for get_weekly_updates_summary.
type GetWeeklyUpdatesSummaryInput struct {
	RequestMetadata
}

// ListYearsInput contains parameters for list_years.
type ListYearsInput struct {
	RequestMetadata
	// DB describes database name or filename under 'data/index'.
	DB optional[string] `query:"db" nullable:"false" doc:"Database name or filename under 'data/index'."`
}

// LiveInput contains parameters for live.
type LiveInput struct {
	RequestMetadata
}

// ReadyInput contains parameters for ready.
type ReadyInput struct {
	RequestMetadata
}

// ListRelationGraphFavoritesInput contains parameters for list_relation_graph_favorites.
type ListRelationGraphFavoritesInput struct {
	RequestMetadata
	// Limit contains the limit value.
	Limit int64 `query:"limit" nullable:"false" minimum:"1" maximum:"100" default:"50"`
	// Cursor describes opaque owner-bound continuation cursor.
	Cursor optional[string] `query:"cursor" nullable:"false" doc:"Opaque owner-bound continuation cursor."`
}

// CreateRelationGraphFavoriteInput contains parameters for create_relation_graph_favorite.
type CreateRelationGraphFavoriteInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body RelationGraphFavoriteCreate `required:"true"`
}

// GetRelationGraphFavoriteInput contains parameters for get_relation_graph_favorite.
type GetRelationGraphFavoriteInput struct {
	RequestMetadata
	// GraphID contains the graph id value.
	GraphID int64 `path:"graph_id" required:"true" nullable:"false" minimum:"1" format:"int64"`
}

// UpdateRelationGraphFavoriteInput contains parameters for update_relation_graph_favorite.
type UpdateRelationGraphFavoriteInput struct {
	RequestMetadata
	// GraphID contains the graph id value.
	GraphID int64 `path:"graph_id" required:"true" nullable:"false" minimum:"1" format:"int64"`
	// Body contains the decoded operation payload.
	Body RelationGraphFavoriteUpdate `required:"true"`
}

// DeleteRelationGraphFavoriteInput contains parameters for delete_relation_graph_favorite.
type DeleteRelationGraphFavoriteInput struct {
	RequestMetadata
	// GraphID contains the graph id value.
	GraphID int64 `path:"graph_id" required:"true" nullable:"false" minimum:"1" format:"int64"`
}

// UpdateEmailInput contains parameters for update_email.
type UpdateEmailInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body EmailUpdateRequest `required:"true"`
}

// GetAuthProvidersInput contains parameters for get_auth_providers.
type GetAuthProvidersInput struct {
	RequestMetadata
}

// OIDCLoginInput contains parameters for oidc_login.
type OIDCLoginInput struct {
	RequestMetadata
	// Next describes same-site relative path.
	Next string `query:"next" nullable:"false" default:"/" doc:"Same-site relative path."`
}

// OIDCCallbackInput contains parameters for oidc_callback.
type OIDCCallbackInput struct {
	RequestMetadata
	// State contains the state value.
	State []string `query:"state,explode" nullable:"false"`
	// Code contains the code value.
	Code []string `query:"code,explode" nullable:"false"`
	// Error contains the error value.
	Error optional[string] `query:"error" nullable:"false"`
}

// OIDCLinkInput contains parameters for oidc_link.
type OIDCLinkInput struct {
	RequestMetadata
}

// OIDCUnlinkInput contains parameters for oidc_unlink.
type OIDCUnlinkInput struct {
	RequestMetadata
}

// NativeLoginInput contains parameters for native_login.
type NativeLoginInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body NativeLoginRequest `required:"true"`
}

// OIDCNativeLoginInput contains parameters for oidc_native_login.
type OIDCNativeLoginInput struct {
	RequestMetadata
	// RedirectURI contains the redirect uri value.
	RedirectURI []string `query:"redirect_uri,explode" required:"true" nullable:"false" minItems:"1" maxItems:"1"`
	// CodeChallenge contains the code challenge value.
	CodeChallenge []string `query:"code_challenge,explode" required:"true" nullable:"false" minLength:"43" maxLength:"43" minItems:"1" maxItems:"1"`
	// State contains the state value.
	State []string `query:"state,explode" required:"true" nullable:"false" pattern:"^[A-Za-z0-9._~-]{16,128}$" minItems:"1" maxItems:"1"`
}

// OIDCNativeExchangeInput contains parameters for oidc_native_exchange.
type OIDCNativeExchangeInput struct {
	RequestMetadata
	// Body contains the decoded operation payload.
	Body NativeOIDCExchangeRequest `required:"true"`
}

// TelemetryInput contains parameters for telemetry.
type TelemetryInput struct {
	RequestMetadata
	// Period describes uTC hour-aligned window including the current partial hour. Default 24h.
	Period string `query:"period" nullable:"false" default:"24h" enum:"24h,7d,30d" doc:"UTC hour-aligned window including the current partial hour. Default 24h."`
	// Kind contains the kind value.
	Kind optional[string] `query:"kind" nullable:"false" enum:"http,mcp,search"`
}
