package openapi

import "github.com/danielgtaylor/huma/v2"

// AdminInviteCodeCreate describes optional administrator overrides for a newly issued invite code.
type AdminInviteCodeCreate struct {
	ExpiresAt *float64 `json:"expires_at,omitempty" nullable:"true" format:"double" doc:"Absolute expiration timestamp; omitted values use the seven-day default."`
	MaxUses   *int64   `json:"max_uses,omitempty" nullable:"true" minimum:"1" maximum:"1000" format:"int64" doc:"Registration quota; omitted values use one redemption."`
}

// AdminInviteCodeInfo describes admin invite code response payload.
type AdminInviteCodeInfo struct {
	Code          string           `json:"code" nullable:"false" doc:"Raw invite code."`
	CreatedAt     float64          `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp."`
	CreatedBy     *UserID          `json:"created_by,omitempty" nullable:"true" format:"int64" doc:"Creator user identifier."`
	CreatedByName *string          `json:"created_by_name,omitempty" nullable:"true" doc:"Creator username."`
	ExpiresAt     float64          `json:"expires_at" nullable:"false" format:"double" doc:"Absolute expiration timestamp."`
	ID            int64            `json:"id" nullable:"false" format:"int64" doc:"Invite code row identifier."`
	MaxUses       int64            `json:"max_uses" nullable:"false" format:"int64" doc:"Maximum permitted registrations."`
	RevokedAt     *float64         `json:"revoked_at,omitempty" nullable:"true" format:"double" doc:"Optional irreversible revocation timestamp."`
	Status        InviteCodeStatus `json:"status" nullable:"false" enum:"active,expired,revoked,exhausted" doc:"Current lifecycle state."`
	UseCount      int64            `json:"use_count" nullable:"false" format:"int64" doc:"Number of committed registrations."`
	UsedAt        *float64         `json:"used_at,omitempty" nullable:"true" format:"double" doc:"Consumption timestamp."`
	UsedBy        *UserID          `json:"used_by,omitempty" nullable:"true" format:"int64" doc:"Consumer user identifier."`
	UsedByName    *string          `json:"used_by_name,omitempty" nullable:"true" doc:"Consumer username."`
}

// AdminResetPassword describes admin password reset request payload.
type AdminResetPassword struct {
	NewPassword string `json:"new_password" nullable:"false" doc:"Replacement password."`
}

// AdminSetAdmin describes admin grant/revoke request payload.
type AdminSetAdmin struct {
	IsAdmin bool `json:"is_admin" nullable:"false" doc:"Replacement admin flag."`
}

// AdminStatsResponse describes admin stats response payload.
type AdminStatsResponse struct {
	Auth  AuthStats   `json:"auth" nullable:"false" doc:"Auth database statistics."`
	Index IndexStats  `json:"index" nullable:"false" doc:"Index database statistics."`
	Push  []PushStats `json:"push" nullable:"false" doc:"Push-state statistics."`
}

// AdminUserInfo describes admin user response payload.
type AdminUserInfo struct {
	CreatedAt     float64 `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp."`
	FavoriteCount int64   `json:"favorite_count" nullable:"false" format:"int64" doc:"Number of favorites owned by the user."`
	FolderCount   int64   `json:"folder_count" nullable:"false" format:"int64" doc:"Number of folders owned by the user."`
	ID            UserID  `json:"id" nullable:"false" format:"int64" doc:"User row identifier."`
	IsAdmin       bool    `json:"is_admin" nullable:"false" doc:"Whether the user has admin privileges."`
	NotifyEnabled bool    `json:"notify_enabled" nullable:"false" doc:"Whether enabled notification settings exist."`
	UpdatedAt     float64 `json:"updated_at" nullable:"false" format:"double" doc:"Last update timestamp."`
	Username      string  `json:"username" nullable:"false" doc:"Username."`
	Email         *string `json:"email" nullable:"true" doc:"Normalized unique account email; null when unset."`
	AuthKind      string  `json:"auth_kind" nullable:"false" enum:"local,oidc" doc:"Account credential kind."`
	IsOIDCLinked  bool    `json:"is_oidc_linked" nullable:"false" doc:"Whether an external OIDC identity is linked."`
}

// AnnouncementCreate describes announcement creation payload.
type AnnouncementCreate struct {
	Enabled  bool   `json:"enabled,omitempty" nullable:"false" doc:"Whether the announcement is visible."`
	Message  string `json:"message" nullable:"false" minLength:"1" maxLength:"10000" doc:"Announcement message body."`
	Priority string `json:"priority,omitempty" nullable:"false" minLength:"1" maxLength:"16" doc:"Priority label."`
	Title    string `json:"title" nullable:"false" minLength:"1" maxLength:"200" doc:"Announcement title."`
}

// AnnouncementInfo describes public announcement payload returned by the API.
type AnnouncementInfo struct {
	CreatedAt float64 `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp as a Unix epoch float."`
	Enabled   bool    `json:"enabled" nullable:"false" doc:"Whether the announcement is visible to public clients."`
	ID        int64   `json:"id" nullable:"false" format:"int64" doc:"Announcement identifier."`
	Message   string  `json:"message" nullable:"false" doc:"Announcement body text."`
	Priority  string  `json:"priority" nullable:"false" doc:"Priority label used for display ordering."`
	Title     string  `json:"title" nullable:"false" doc:"Announcement title."`
	UpdatedAt float64 `json:"updated_at" nullable:"false" format:"double" doc:"Last update timestamp as a Unix epoch float."`
}

// AnnouncementUpdate describes announcement update payload.
type AnnouncementUpdate struct {
	Enabled  *bool   `json:"enabled,omitempty" nullable:"true" doc:"Optional enabled flag."`
	Message  *string `json:"message,omitempty" nullable:"true" minLength:"1" maxLength:"10000" doc:"Optional replacement message."`
	Priority *string `json:"priority,omitempty" nullable:"true" minLength:"1" maxLength:"16" doc:"Optional replacement priority."`
	Title    *string `json:"title,omitempty" nullable:"true" minLength:"1" maxLength:"200" doc:"Optional replacement title."`
}

// ArticleAccessAction describes article access action.
type ArticleAccessAction struct {
	Available     bool    `json:"available" nullable:"false" doc:"Whether the action is available."`
	Label         string  `json:"label" nullable:"false" doc:"Display label."`
	Message       *string `json:"message,omitempty" nullable:"true" doc:"Optional message."`
	RequiresLogin bool    `json:"requires_login" nullable:"false" doc:"Whether login is required."`
}

// ArticleAccessResponse describes article access response.
type ArticleAccessResponse struct {
	AbstractPage ArticleAccessAction `json:"abstract_page" nullable:"false" doc:"Abstract-page action."`
	Fulltext     ArticleAccessAction `json:"fulltext" nullable:"false" doc:"Full-text action."`
}

// ArticleID describes article identifier stored internally as a SQLite integer.
type ArticleID string

// ArticlePage describes paginated articles response.
type ArticlePage struct {
	Items []ArticleRecord `json:"items" nullable:"false" doc:"Article records."`
	Page  PageMeta        `json:"page" nullable:"false" doc:"Pagination metadata."`
}

// ArticleRecord describes article record.
type ArticleRecord struct {
	Abstract        *string        `json:"abstract,omitempty" nullable:"true" doc:"Abstract text."`
	ArticleID       ArticleID      `json:"article_id" nullable:"false" doc:"Article identifier."`
	Authors         []string       `json:"authors" nullable:"false" doc:"Authors text."`
	Date            *string        `json:"date,omitempty" nullable:"true" doc:"Article date."`
	DatePrecision   *DatePrecision `json:"date_precision,omitempty" nullable:"true" enum:"year,month,day" doc:"Precision represented by the article date."`
	DOI             *string        `json:"doi,omitempty" nullable:"true" doc:"DOI."`
	EndPage         *string        `json:"end_page,omitempty" nullable:"true" doc:"End page."`
	InPress         *bool          `json:"in_press,omitempty" nullable:"true" doc:"In-press flag."`
	IssueID         *int64         `json:"issue_id,omitempty" nullable:"true" format:"int64" doc:"Issue identifier."`
	JournalID       JournalID      `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	JournalTitle    string         `json:"journal_title" nullable:"false" doc:"Journal title."`
	Number          *string        `json:"number,omitempty" nullable:"true" doc:"Issue number."`
	OpenAccess      *bool          `json:"open_access,omitempty" nullable:"true" doc:"Open-access flag."`
	PMID            *string        `json:"pmid,omitempty" nullable:"true" doc:"PubMed identifier."`
	PublicationYear *int64         `json:"publication_year,omitempty" nullable:"true" format:"int64" doc:"Publication year."`
	RetractionDois  []string       `json:"retraction_dois" nullable:"false" doc:"Retraction DOIs in lexical order without duplicates."`
	StartPage       *string        `json:"start_page,omitempty" nullable:"true" doc:"Start page."`
	Title           string         `json:"title" nullable:"false" doc:"Source title; an empty string means unavailable title metadata on a DOI-identified record."`
	Volume          *string        `json:"volume,omitempty" nullable:"true" doc:"Issue volume."`
}

// ArticleSearchMode describes full-text search interpretation mode.
type ArticleSearchMode string

// AuthStats describes auth database statistics payload.
type AuthStats struct {
	ActiveAnnouncements     int64 `json:"active_announcements" nullable:"false" format:"int64" doc:"Enabled announcement count."`
	ActiveTokens            int64 `json:"active_tokens" nullable:"false" format:"int64" doc:"Unexpired access tokens."`
	AdminCount              int64 `json:"admin_count" nullable:"false" format:"int64" doc:"Users with admin privileges."`
	NotificationSubscribers int64 `json:"notification_subscribers" nullable:"false" format:"int64" doc:"Enabled notification settings."`
	ScheduledTasks          int64 `json:"scheduled_tasks" nullable:"false" format:"int64" doc:"Scheduled task count."`
	TotalFavorites          int64 `json:"total_favorites" nullable:"false" format:"int64" doc:"Total favorites."`
	TotalFolders            int64 `json:"total_folders" nullable:"false" format:"int64" doc:"Total folders."`
	TotalInviteCodes        int64 `json:"total_invite_codes" nullable:"false" format:"int64" doc:"Total invite codes."`
	TotalUsers              int64 `json:"total_users" nullable:"false" format:"int64" doc:"Total registered users."`
	UnusedInviteCodes       int64 `json:"unused_invite_codes" nullable:"false" format:"int64" doc:"Unused invite codes."`
	UsedInviteCodes         int64 `json:"used_invite_codes" nullable:"false" format:"int64" doc:"Used invite codes."`
}

// CfpCatalogResponse describes complete lightweight CFP journal catalog for one database.
type CfpCatalogResponse struct {
	Database    string              `json:"database" nullable:"false" doc:"Canonical selected database filename."`
	EvaluatedAt int64               `json:"evaluatedAt" nullable:"false" format:"int64" doc:"UTC Unix instant used to evaluate every state in this response."`
	Items       []CfpJournalSummary `json:"items" nullable:"false" doc:"Lightweight journals matching the optional search, without notice bodies."`
	Summary     CfpCatalogSummary   `json:"summary" nullable:"false" doc:"Whole-database summary."`
}

// CfpCatalogSummary describes aggregate counts for the selected database before the journal-name search filter.
type CfpCatalogSummary struct {
	AdaptedJournals int64 `json:"adaptedJournals" nullable:"false" minimum:"0" doc:"Journals with a reviewed snapshot or scoped empty statement."`
	CurrentNotices  int64 `json:"currentNotices" nullable:"false" minimum:"0" doc:"Notices excluding closed and historical calls."`
	Journals        int64 `json:"journals" nullable:"false" minimum:"0" doc:"Complete maintained journal count, including journals without indexed articles."`
	Notices         int64 `json:"notices" nullable:"false" minimum:"0" doc:"All stored notices associated with this database."`
}

// CfpCoverage describes coverage of a journal's reviewed original-source data.
type CfpCoverage string

// CfpDate describes a valid calendar date and its original clause.
type CfpDate struct {
	Date         string       `json:"date" nullable:"false" doc:"ISO calendar date, with no invented time of day."`
	IsExclusive  bool         `json:"isExclusive" nullable:"false" doc:"Whether the source says strictly before this date."`
	IsOptional   bool         `json:"isOptional" nullable:"false" doc:"Whether this milestone is optional."`
	OriginalText string       `json:"originalText" nullable:"false" doc:"Untranslated source clause."`
	Stage        CfpDateStage `json:"stage" nullable:"false" enum:"paper,abstract,proposal,opens,revision,decision,publication,event,registration" doc:"Purpose of this date."`
}

// CfpDateStage describes purpose of a dated milestone, independent of whether it is required.
type CfpDateStage string

// CfpJournalSummary describes lightweight journal metadata and backend-calculated CFP summary.
type CfpJournalSummary struct {
	AllISSNs        []string         `json:"allIssns" nullable:"false" doc:"All maintained print and electronic ISSNs for display and journal search."`
	Area            *string          `json:"area,omitempty" nullable:"true" doc:"Maintained subject area, if available."`
	CanRefresh      bool             `json:"canRefresh" nullable:"false" doc:"Whether a backend discovery adapter can attempt automatic refresh."`
	CatalogAliases  []string         `json:"catalogAliases" nullable:"false" doc:"Maintained historical aliases in the selected catalog."`
	CatalogID       string           `json:"catalogId" nullable:"false" doc:"Canonical maintained catalog ID used by the notice endpoint."`
	CheckedOn       *string          `json:"checkedOn,omitempty" nullable:"true" doc:"Last verified original-source date, if adapted."`
	Coverage        CfpCoverage      `json:"coverage" nullable:"false" enum:"adapted,unadapted" doc:"Reviewed-source coverage."`
	CurrentCount    int64            `json:"currentCount" nullable:"false" minimum:"0" doc:"Notices remaining after the default closed/historical filter."`
	LastAttempt     *int64           `json:"lastAttempt,omitempty" nullable:"true" format:"int64" doc:"Latest acquisition attempt Unix timestamp."`
	LastError       *string          `json:"lastError,omitempty" nullable:"true" doc:"Sanitized latest failure reason."`
	LastSuccess     *int64           `json:"lastSuccess,omitempty" nullable:"true" format:"int64" doc:"Last successful live acquisition Unix timestamp."`
	NoticeCount     int64            `json:"noticeCount" nullable:"false" minimum:"0" doc:"Total stored notices, including closed and historical calls."`
	RefreshStatus   CfpRefreshStatus `json:"refreshStatus" nullable:"false" enum:"snapshot,success,failed,refreshing,unsupported,unadapted" doc:"Latest acquisition status, including interrupted lease detection."`
	SourceStatement *string          `json:"sourceStatement,omitempty" nullable:"true" doc:"Untranslated scoped statement that no matching calls are listed."`
	SourceURL       *string          `json:"sourceUrl,omitempty" nullable:"true" doc:"Original source URL associated with a scoped publisher statement."`
	StateCounts     map[string]int64 `json:"stateCounts" nullable:"false" doc:"Backend-computed counts for each availability state."`
	Title           string           `json:"title" nullable:"false" doc:"Journal title from the selected metadata database."`
	TitleAliases    []string         `json:"titleAliases" nullable:"false" doc:"Maintained journal title aliases for search."`
}

// CfpKind describes submission purpose supported by the original announcement.
type CfpKind string

// CfpNotice describes normalized notice persisted by the backend; availability is evaluated on read.
type CfpNotice struct {
	CheckedOn    string       `json:"checkedOn" nullable:"false" doc:"Last verified original-source date."`
	Dates        []CfpDate    `json:"dates" nullable:"false" doc:"Validated dates, including non-submission milestones."`
	EntryStage   CfpDateStage `json:"entryStage" nullable:"false" enum:"paper,abstract,proposal,opens,revision,decision,publication,event,registration" doc:"Default initial submission stage."`
	ID           string       `json:"id" nullable:"false" doc:"Stable legacy-compatible URL/title identity within a journal."`
	IsHistorical bool         `json:"isHistorical" nullable:"false" doc:"Whether this announcement is an archival record."`
	Kind         CfpKind      `json:"kind" nullable:"false" enum:"special_issue,general,proposal,conference_linked" doc:"Classified kind of call."`
	RawDateText  string       `json:"rawDateText" nullable:"false" doc:"Unresolved original timeline; never interpreted as perpetual availability."`
	Requirements string       `json:"requirements" nullable:"false" doc:"Original instructions, empty if unavailable."`
	Scope        string       `json:"scope" nullable:"false" doc:"Original topic excerpt, empty if unavailable."`
	SourceStatus *CfpState    `json:"sourceStatus,omitempty" nullable:"true" enum:"open,upcoming,closed,historical,invitation_only,undated,uncertain" doc:"Explicit closed or invitation-only source constraint."`
	SourceURL    string       `json:"sourceUrl" nullable:"false" doc:"Original public announcement URL."`
	TimeZone     *string      `json:"timeZone,omitempty" nullable:"true" doc:"Explicit IANA timezone, if supplied by the source."`
	Title        string       `json:"title" nullable:"false" doc:"Original announcement title."`
	TopicYear    *int64       `json:"topicYear,omitempty" nullable:"true" format:"int32" doc:"Explicit annual topic year, when applicable."`
}

// CfpNoticePage describes a revision-consistent page of original notices for one maintained catalog member.
type CfpNoticePage struct {
	EvaluatedAt int64             `json:"evaluatedAt" nullable:"false" format:"int64" doc:"Fixed UTC Unix evaluation instant shared across cursor pages."`
	Items       []CfpNoticeView   `json:"items" nullable:"false" doc:"Original notice records in stable source order."`
	Journal     CfpJournalSummary `json:"journal" nullable:"false" doc:"Metadata, coverage and source freshness for the journal."`
	Page        PageMeta          `json:"page" nullable:"false" doc:"Standard API pagination metadata, with a scope-bound next cursor."`
}

// CfpNoticeView describes one original notice with its authoritative backend-evaluated state and initial gate.
type CfpNoticeView struct {
	CheckedOn     string       `json:"checkedOn" nullable:"false" doc:"Last verified original-source date."`
	Dates         []CfpDate    `json:"dates" nullable:"false" doc:"Validated dates, including non-submission milestones."`
	EntryStage    CfpDateStage `json:"entryStage" nullable:"false" enum:"paper,abstract,proposal,opens,revision,decision,publication,event,registration" doc:"Default initial submission stage."`
	ID            string       `json:"id" nullable:"false" doc:"Stable legacy-compatible URL/title identity within a journal."`
	IsHistorical  bool         `json:"isHistorical" nullable:"false" doc:"Whether this announcement is an archival record."`
	Kind          CfpKind      `json:"kind" nullable:"false" enum:"special_issue,general,proposal,conference_linked" doc:"Classified kind of call."`
	RawDateText   string       `json:"rawDateText" nullable:"false" doc:"Unresolved original timeline; never interpreted as perpetual availability."`
	Requirements  string       `json:"requirements" nullable:"false" doc:"Original instructions, empty if unavailable."`
	Scope         string       `json:"scope" nullable:"false" doc:"Original topic excerpt, empty if unavailable."`
	SourceStatus  *CfpState    `json:"sourceStatus,omitempty" nullable:"true" enum:"open,upcoming,closed,historical,invitation_only,undated,uncertain" doc:"Explicit closed or invitation-only source constraint."`
	SourceURL     string       `json:"sourceUrl" nullable:"false" doc:"Original public announcement URL."`
	TimeZone      *string      `json:"timeZone,omitempty" nullable:"true" doc:"Explicit IANA timezone, if supplied by the source."`
	Title         string       `json:"title" nullable:"false" doc:"Original announcement title."`
	TopicYear     *int64       `json:"topicYear,omitempty" nullable:"true" format:"int32" doc:"Explicit annual topic year, when applicable."`
	EntryDeadline *CfpDate     `json:"entryDeadline,omitempty" nullable:"false" doc:"Earliest mandatory initial submission deadline."`
	State         CfpState     `json:"state" nullable:"false" enum:"open,upcoming,closed,historical,invitation_only,undated,uncertain" doc:"Availability evaluated at the page's fixed instant."`
}

// TransformSchema retains explicitly nullable object fields.
func (CfpNoticeView) TransformSchema(_ huma.Registry, schema *huma.Schema) *huma.Schema {
	schema.Properties["entryDeadline"] = nullableSchema(schema.Properties["entryDeadline"])
	return schema
}

// CfpRefreshStatus describes source refresh freshness, independent of a notice's submission availability.
type CfpRefreshStatus string

// CfpState describes availability evaluated at a specified instant, without translating source text.
type CfpState string

// ChangePasswordRequest describes password change request.
type ChangePasswordRequest struct {
	NewPassword string `json:"new_password" nullable:"false" doc:"Replacement password."`
	OldPassword string `json:"old_password" nullable:"false" doc:"Current password."`
}

// CnkiLoginPollRequest describes zhejiang Library QR login polling parameters.
type CnkiLoginPollRequest struct {
	IntervalSeconds float64 `json:"interval_seconds,omitempty" nullable:"false" format:"double" doc:"Poll interval in seconds."`
	TimeoutSeconds  int64   `json:"timeout_seconds,omitempty" nullable:"false" format:"int64" doc:"Poll timeout in seconds."`
}

// CnkiLoginPollResponse describes zhejiang Library QR login polling response.
type CnkiLoginPollResponse struct {
	Session CnkiSessionStatusResponse `json:"session" nullable:"false" doc:"Safe session status."`
	Status  string                    `json:"status" nullable:"false" doc:"Poll status."`
}

// CnkiLoginStartResponse describes zhejiang Library QR login challenge response.
type CnkiLoginStartResponse struct {
	QrCode  string                    `json:"qr_code" nullable:"false" doc:"QR code URL or payload."`
	Session CnkiSessionStatusResponse `json:"session" nullable:"false" doc:"Safe session status."`
	Status  string                    `json:"status" nullable:"false" doc:"Upstream login status."`
	UUID    string                    `json:"uuid" nullable:"false" doc:"QR UUID."`
}

// CnkiSessionStatusResponse describes safe per-user Zhejiang Library CNKI session status.
type CnkiSessionStatusResponse struct {
	Configured       bool     `json:"configured" nullable:"false" doc:"Whether a session-like row is configured."`
	CookieNames      []string `json:"cookie_names" nullable:"false" doc:"Stored cookie names without cookie values."`
	ExpiresAt        *float64 `json:"expires_at,omitempty" nullable:"true" format:"double" doc:"Token expiration timestamp."`
	HasBffUserToken  bool     `json:"has_bff_user_token" nullable:"false" doc:"Whether a BFF user token is present."`
	LastUsedAt       *float64 `json:"last_used_at,omitempty" nullable:"true" format:"double" doc:"Last-use timestamp."`
	SecondsRemaining *int64   `json:"seconds_remaining,omitempty" nullable:"true" format:"int64" doc:"Seconds remaining until expiration."`
	Status           string   `json:"status" nullable:"false" doc:"Safe status label."`
	UpdatedAt        *float64 `json:"updated_at,omitempty" nullable:"true" format:"double" doc:"Row update timestamp."`
}

// DatePrecision describes precision retained by one canonical partial publication date.
type DatePrecision string

// FavoriteAdd describes favorite creation request payload.
type FavoriteAdd struct {
	ArticleID ArticleID `json:"article_id" nullable:"false" doc:"Article identifier."`
	DBName    string    `json:"db_name,omitempty" nullable:"false" maxLength:"255" doc:"Source index database name."`
	Note      string    `json:"note,omitempty" nullable:"false" maxLength:"2000" doc:"User note text."`
}

// FavoriteArticlePage describes stable cursor page of favorite references and available article metadata.
type FavoriteArticlePage struct {
	Items []FavoriteArticleResponse `json:"items" nullable:"false" doc:"Favorite rows in descending creation time and row-id order."`
	Page  PageMeta                  `json:"page" nullable:"false" doc:"Cursor continuation metadata without an expensive total count."`
}

// FavoriteArticleRef describes favorite article reference used by bulk operations.
type FavoriteArticleRef struct {
	ArticleID ArticleID `json:"article_id" nullable:"false" doc:"Article identifier."`
	DBName    string    `json:"db_name,omitempty" nullable:"false" maxLength:"255" doc:"Source index database name."`
}

// FavoriteArticleResponse describes favorite row enriched with optional article metadata.
type FavoriteArticleResponse struct {
	Abstract        *string                `json:"abstract,omitempty" nullable:"true" doc:"Article abstract text."`
	ArticleID       ArticleID              `json:"article_id" nullable:"false" doc:"Article identifier."`
	Authors         []string               `json:"authors,omitempty" nullable:"true" doc:"Ordered article author names."`
	CreatedAt       float64                `json:"created_at" nullable:"false" format:"double" doc:"Favorite creation timestamp."`
	Date            *string                `json:"date,omitempty" nullable:"true" doc:"Article publication date."`
	DBName          string                 `json:"db_name" nullable:"false" doc:"Source index database name."`
	DOI             *string                `json:"doi,omitempty" nullable:"true" doc:"Article DOI."`
	Eissn           *string                `json:"eissn,omitempty" nullable:"true" doc:"Journal electronic ISSN."`
	FolderID        int64                  `json:"folder_id" nullable:"false" format:"int64" doc:"Folder row identifier."`
	ID              int64                  `json:"id" nullable:"false" format:"int64" doc:"Favorite row identifier."`
	InPress         *bool                  `json:"in_press,omitempty" nullable:"true" doc:"In-press flag from the index database."`
	ISSN            *string                `json:"issn,omitempty" nullable:"true" doc:"Journal ISSN."`
	IssueID         *int64                 `json:"issue_id,omitempty" nullable:"true" format:"int64" doc:"Issue identifier from the index database."`
	JournalID       *JournalID             `json:"journal_id,omitempty" nullable:"true" doc:"Journal identifier from the index database."`
	JournalTitle    *string                `json:"journal_title,omitempty" nullable:"true" doc:"Journal title."`
	MetadataStatus  FavoriteMetadataStatus `json:"metadata_status" nullable:"false" enum:"available,missing,unavailable" doc:"Availability of the referenced index article metadata."`
	Note            string                 `json:"note" nullable:"false" doc:"User note text."`
	Number          *string                `json:"number,omitempty" nullable:"true" doc:"Issue number."`
	OpenAccess      *bool                  `json:"open_access,omitempty" nullable:"true" doc:"Open-access flag from the index database."`
	PublicationYear *int64                 `json:"publication_year,omitempty" nullable:"true" format:"int64" doc:"Article publication year."`
	Title           *string                `json:"title,omitempty" nullable:"true" doc:"Article title from the index database."`
	Volume          *string                `json:"volume,omitempty" nullable:"true" doc:"Issue volume."`
}

// FavoriteBatchCheckRequest describes batch favorite check request.
type FavoriteBatchCheckRequest struct {
	ArticleIds []ArticleID `json:"article_ids" nullable:"false" maxItems:"500" doc:"Article identifiers to check."`
	DBName     string      `json:"db_name,omitempty" nullable:"false" maxLength:"255" doc:"Source index database name."`
}

// FavoriteBatchCheckResponse describes batch favorite check response item.
type FavoriteBatchCheckResponse struct {
	ArticleID ArticleID               `json:"article_id" nullable:"false" doc:"Article identifier that was checked."`
	Folders   []FavoriteCheckResponse `json:"folders" nullable:"false" doc:"Folder memberships for the article."`
}

// FavoriteBulkAdd describes bulk favorite add request.
type FavoriteBulkAdd struct {
	Articles []FavoriteAdd `json:"articles" nullable:"false" maxItems:"500" doc:"Favorite articles to add."`
}

// FavoriteBulkAddResult describes bulk add response preserving the existing `added` key.
type FavoriteBulkAddResult struct {
	Added int64 `json:"added" nullable:"false" format:"int64" doc:"Number of inserted favorite rows."`
}

// FavoriteBulkMove describes bulk favorite move request.
type FavoriteBulkMove struct {
	Articles       []FavoriteArticleRef `json:"articles" nullable:"false" maxItems:"500" doc:"Favorite articles to move."`
	TargetFolderID int64                `json:"target_folder_id" nullable:"false" format:"int64" doc:"Target folder row identifier."`
}

// FavoriteBulkRemove describes bulk favorite remove request.
type FavoriteBulkRemove struct {
	Articles []FavoriteArticleRef `json:"articles" nullable:"false" maxItems:"500" doc:"Favorite articles to remove."`
}

// FavoriteBulkResult describes bulk operation count response.
type FavoriteBulkResult struct {
	Count int64 `json:"count" nullable:"false" format:"int64" doc:"Number of affected favorite rows."`
}

// FavoriteCheckResponse describes favorite membership row returned by check endpoints.
type FavoriteCheckResponse struct {
	FolderID   int64  `json:"folder_id" nullable:"false" format:"int64" doc:"Folder row identifier."`
	FolderName string `json:"folder_name" nullable:"false" doc:"Folder display name."`
}

// FavoriteMetadataStatus describes availability of index metadata for a stored favorite reference.
type FavoriteMetadataStatus string

// FavoriteResponse describes favorite row response payload.
type FavoriteResponse struct {
	ArticleID ArticleID `json:"article_id" nullable:"false" doc:"Article identifier."`
	CreatedAt float64   `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp."`
	DBName    string    `json:"db_name" nullable:"false" doc:"Source index database name."`
	FolderID  int64     `json:"folder_id" nullable:"false" format:"int64" doc:"Folder row identifier."`
	ID        int64     `json:"id" nullable:"false" format:"int64" doc:"Favorite row identifier."`
	Note      string    `json:"note" nullable:"false" doc:"User note text."`
}

// FavoriteTrackingResponse describes current favorite tracking folder payload.
type FavoriteTrackingResponse struct {
	FolderID   *int64  `json:"folder_id,omitempty" nullable:"true" format:"int64" doc:"Tracking folder identifier, or null when none is configured."`
	FolderName *string `json:"folder_name,omitempty" nullable:"true" doc:"Tracking folder display name, or null when none is configured."`
}

// FolderCreate describes create-folder request payload.
type FolderCreate struct {
	IsTracking bool   `json:"is_tracking,omitempty" nullable:"false" doc:"Whether the folder should become the user's tracking folder."`
	Name       string `json:"name" nullable:"false" minLength:"1" maxLength:"100" doc:"Folder display name."`
}

// FolderRename describes rename-folder request payload.
type FolderRename struct {
	Name string `json:"name" nullable:"false" minLength:"1" maxLength:"100" doc:"Replacement folder display name."`
}

// FolderResponse describes folder response payload.
type FolderResponse struct {
	ArticleCount int64   `json:"article_count" nullable:"false" format:"int64" doc:"Number of favorite rows in the folder."`
	CreatedAt    float64 `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp."`
	ID           int64   `json:"id" nullable:"false" format:"int64" doc:"Folder row identifier."`
	IsTracking   bool    `json:"is_tracking" nullable:"false" doc:"Whether the folder receives tracking pushes."`
	Name         string  `json:"name" nullable:"false" doc:"Folder display name."`
}

// HealthResponse describes health check payload returned by the API.
type HealthResponse struct {
	Status HealthStatus `json:"status" nullable:"false" enum:"ok,unhealthy" doc:"Service status value."`
}

// HealthStatus describes application-owned service health state.
type HealthStatus string

// IndexDatabaseStats describes per-index database statistics payload.
type IndexDatabaseStats struct {
	Articles int64  `json:"articles" nullable:"false" format:"int64" doc:"Article count."`
	DBName   string `json:"db_name" nullable:"false" doc:"Database file name."`
	Error    *bool  `json:"error,omitempty" nullable:"true" doc:"Whether the database failed to read."`
	Issues   int64  `json:"issues" nullable:"false" format:"int64" doc:"Issue count."`
	Journals int64  `json:"journals" nullable:"false" format:"int64" doc:"Journal count."`
}

// IndexStats describes aggregate index statistics payload.
type IndexStats struct {
	Databases     []IndexDatabaseStats `json:"databases" nullable:"false" doc:"Per-database statistics."`
	TotalArticles int64                `json:"total_articles" nullable:"false" format:"int64" doc:"Total article count."`
	TotalJournals int64                `json:"total_journals" nullable:"false" format:"int64" doc:"Total journal count."`
}

// InviteCodeResponse describes invite code response.
type InviteCodeResponse struct {
	Code      string           `json:"code" nullable:"false" doc:"Raw invite code."`
	CreatedAt float64          `json:"created_at" nullable:"false" format:"double" doc:"Invite code creation timestamp."`
	ExpiresAt float64          `json:"expires_at" nullable:"false" format:"double" doc:"Absolute expiration timestamp."`
	ID        int64            `json:"id" nullable:"false" format:"int64" doc:"Invite code row identifier."`
	MaxUses   int64            `json:"max_uses" nullable:"false" format:"int64" doc:"Maximum permitted registrations."`
	RevokedAt *float64         `json:"revoked_at,omitempty" nullable:"true" format:"double" doc:"Optional irreversible revocation timestamp."`
	Status    InviteCodeStatus `json:"status" nullable:"false" enum:"active,expired,revoked,exhausted" doc:"Current lifecycle state."`
	UseCount  int64            `json:"use_count" nullable:"false" format:"int64" doc:"Number of committed registrations."`
	Used      bool             `json:"used" nullable:"false" doc:"Whether the invite code has been consumed."`
}

// InviteCodeStatus describes public lifecycle state for an invite code.
type InviteCodeStatus string

// InviteRequiredResponse describes invite requirement response.
type InviteRequiredResponse struct {
	BootstrapRequired bool `json:"bootstrap_required" nullable:"false" doc:"Whether a local administrator bootstrap must run before invites can be issued."`
	Required          bool `json:"required" nullable:"false" doc:"Whether registration requires an invite code."`
}

// IssuePage describes paginated issues response.
type IssuePage struct {
	Items []IssueRecord `json:"items" nullable:"false" doc:"Issue records."`
	Page  PageMeta      `json:"page" nullable:"false" doc:"Pagination metadata."`
}

// IssueRecord describes issue record.
type IssueRecord struct {
	Date            *string        `json:"date,omitempty" nullable:"true" doc:"Issue date."`
	DatePrecision   *DatePrecision `json:"date_precision,omitempty" nullable:"true" enum:"year,month,day" doc:"Precision represented by the issue date."`
	IssueID         int64          `json:"issue_id" nullable:"false" format:"int64" doc:"Issue identifier."`
	JournalID       JournalID      `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	Number          *string        `json:"number,omitempty" nullable:"true" doc:"Number."`
	PublicationYear *int64         `json:"publication_year,omitempty" nullable:"true" format:"int64" doc:"Publication year."`
	Title           *string        `json:"title,omitempty" nullable:"true" doc:"Issue title."`
	Volume          *string        `json:"volume,omitempty" nullable:"true" doc:"Volume."`
}

// JournalID describes journal identifier stored internally as a SQLite integer.
type JournalID string

// JournalOption describes journal option for selection lists.
type JournalOption struct {
	JournalID JournalID `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	Title     string    `json:"title" nullable:"false" doc:"Journal title."`
}

// JournalPage describes paginated journals response.
type JournalPage struct {
	Items []JournalRecord `json:"items" nullable:"false" doc:"Journal records."`
	Page  PageMeta        `json:"page" nullable:"false" doc:"Pagination metadata."`
}

// JournalRatingOptions describes available exact grades and database-wide journal counts for each rating system.
type JournalRatingOptions struct {
	AbsRating   []ValueCount `json:"abs_rating" nullable:"false" doc:"ABS grade choices; '4*' is an exact label rather than a wildcard."`
	FmsRating   []ValueCount `json:"fms_rating" nullable:"false" doc:"FMS grade choices."`
	FmscnRating []ValueCount `json:"fmscn_rating" nullable:"false" doc:"FMS China grade choices."`
	UtdRating   []ValueCount `json:"utd_rating" nullable:"false" doc:"UTD grade choices, excluding unrated journals."`
}

// JournalRecord describes provider-neutral journal record.
type JournalRecord struct {
	AbsRank      *string   `json:"abs_rank,omitempty" nullable:"true" doc:"ABS rank."`
	AbsRating    *string   `json:"abs_rating,omitempty" nullable:"true" doc:"ABS rating."`
	Area         *string   `json:"area,omitempty" nullable:"true" doc:"Journal area."`
	CatalogID    string    `json:"catalog_id" nullable:"false" doc:"Immutable maintained catalog identifier."`
	Eissn        *string   `json:"eissn,omitempty" nullable:"true" doc:"Electronic ISSN."`
	FmsRank      *string   `json:"fms_rank,omitempty" nullable:"true" doc:"FMS rank."`
	FmsRating    *string   `json:"fms_rating,omitempty" nullable:"true" doc:"FMS rating."`
	FmscnRank    *string   `json:"fmscn_rank,omitempty" nullable:"true" doc:"FMS China rank."`
	FmscnRating  *string   `json:"fmscn_rating,omitempty" nullable:"true" doc:"FMS China rating."`
	HasArticles  bool      `json:"has_articles" nullable:"false" doc:"Whether the content database currently contains articles for the journal."`
	ISSN         *string   `json:"issn,omitempty" nullable:"true" doc:"Print ISSN."`
	ISSNs        []string  `json:"issns" nullable:"false" doc:"All maintained ISSNs."`
	JournalID    JournalID `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	Title        string    `json:"title" nullable:"false" doc:"Journal title."`
	TitleAliases []string  `json:"title_aliases" nullable:"false" doc:"Maintained title aliases."`
	UtdRank      *string   `json:"utd_rank,omitempty" nullable:"true" doc:"UTD rank."`
	UtdRating    *string   `json:"utd_rating,omitempty" nullable:"true" doc:"UTD rating."`
}

// NativeOIDCExchangeRequest describes native OIDC authorization code exchange with client PKCE. No session cookie is issued.
type NativeOIDCExchangeRequest struct {
	Code         string `json:"code" nullable:"false" doc:"One-time unpadded base64url code (32 random bytes); TTL 120 seconds."`
	CodeVerifier string `json:"code_verifier" nullable:"false" minLength:"43" maxLength:"128" pattern:"^[A-Za-z0-9._~-]{43,128}$"`
	RedirectURI  string `json:"redirect_uri" nullable:"false" doc:"Exact native redirect_uri supplied at login."`
	DeviceName   string `json:"device_name" nullable:"false" minLength:"1" maxLength:"93" doc:"Device label; surrounding whitespace is trimmed, blank text and control characters are rejected. The raw input is limited to 93 Unicode code points."`
}

// NativeLoginRequest describes native password login that issues an independent 90-day personal access token.
type NativeLoginRequest struct {
	Password   string `json:"password" nullable:"false" doc:"Password."`
	Username   string `json:"username" nullable:"false" doc:"Username or case-insensitive email address."`
	DeviceName string `json:"device_name" nullable:"false" minLength:"1" maxLength:"93" doc:"Device label; surrounding whitespace is trimmed, blank text and control characters are rejected. The raw input is limited to 93 Unicode code points."`
}

// NativeLoginResponse describes native login credential. Store the raw token in OS secure storage; it is returned only on issuance.
type NativeLoginResponse struct {
	ExpiresAt float64      `json:"expires_at" nullable:"false" format:"double" doc:"Unix expiration timestamp in seconds, 90 days after issuance."`
	User      UserResponse `json:"user" nullable:"false" doc:"Authenticated user."`
	Token     string       `json:"token" nullable:"false" doc:"Personal access token for Authorization: Bearer."`
}

// LoginRequest describes login request.
type LoginRequest struct {
	Password string `json:"password" nullable:"false" doc:"Password."`
	Username string `json:"username" nullable:"false" doc:"Username or case-insensitive email address."`
}

// LoginResponse describes login response that intentionally omits the raw session token.
type LoginResponse struct {
	ExpiresAt float64      `json:"expires_at" nullable:"false" format:"double" doc:"Session expiration timestamp."`
	User      UserResponse `json:"user" nullable:"false" doc:"Authenticated user."`
}

// LogoutResponse describes logout response.
type LogoutResponse struct {
	Ok     bool   `json:"ok" nullable:"false" doc:"Whether the operation succeeded."`
	UserID UserID `json:"user_id" nullable:"false" format:"int64" doc:"Authenticated user identifier."`
}

// ManualPushState describes application-owned state of a manual weekly push.
type ManualPushState string

// ManualWeeklyPushStatus describes manual weekly push job status payload.
type ManualWeeklyPushStatus struct {
	CanCancel             bool            `json:"can_cancel" nullable:"false" doc:"Whether the current job still accepts cancellation."`
	CanRetry              bool            `json:"can_retry" nullable:"false" doc:"Whether starting a new run cannot replay an ambiguous external outcome."`
	CancellationRequested bool            `json:"cancellation_requested" nullable:"false" doc:"Whether durable cancellation has been requested."`
	DeadlineAt            *float64        `json:"deadline_at,omitempty" nullable:"true" format:"double" doc:"Absolute Unix timestamp after which the job must stop."`
	FinishedAt            *float64        `json:"finished_at,omitempty" nullable:"true" format:"double" doc:"Unix timestamp when the job finished."`
	FolderID              *int64          `json:"folder_id,omitempty" nullable:"true" format:"int64" doc:"Tracking folder identifier when applicable."`
	FolderName            *string         `json:"folder_name,omitempty" nullable:"true" doc:"Tracking folder name when applicable."`
	JobID                 *string         `json:"job_id,omitempty" nullable:"true" doc:"Background job identifier."`
	Message               string          `json:"message" nullable:"false" doc:"Human-readable status message."`
	Pushed                int64           `json:"pushed" nullable:"false" format:"int64" doc:"Number of pushed or tracking-folder-synced articles."`
	Selected              int64           `json:"selected" nullable:"false" format:"int64" doc:"Number of selected articles."`
	StartedAt             *float64        `json:"started_at,omitempty" nullable:"true" format:"double" doc:"Unix timestamp when the job started."`
	Status                ManualPushState `json:"status" nullable:"false" enum:"idle,pending,running,completed,failed,cancelled,timed_out,unknown" doc:"Durable job status."`
	Summary               string          `json:"summary" nullable:"false" doc:"AI-generated summary text when available."`
	TotalCandidates       *int64          `json:"total_candidates,omitempty" nullable:"true" format:"int64" doc:"Number of candidate articles considered by AI selection."`
}

// NotificationSettingsResponse describes notification settings response payload.
type NotificationSettingsResponse struct {
	AIAPIKeyMask         string   `json:"ai_api_key_mask" nullable:"false" doc:"Fixed non-secret mask when a primary AI API key is configured."`
	AIBackupAPIKeyMask   string   `json:"ai_backup_api_key_mask" nullable:"false" doc:"Fixed non-secret mask when a backup AI API key is configured."`
	AIBackupBaseURL      string   `json:"ai_backup_base_url" nullable:"false" doc:"Backup AI endpoint base URL."`
	AIBackupModel        string   `json:"ai_backup_model" nullable:"false" doc:"Backup AI model."`
	AIBackupSystemPrompt string   `json:"ai_backup_system_prompt" nullable:"false" doc:"Backup AI system prompt."`
	AIBaseURL            string   `json:"ai_base_url" nullable:"false" doc:"Primary AI endpoint base URL."`
	AIModel              string   `json:"ai_model" nullable:"false" doc:"Primary AI model."`
	AIRetryAttempts      int64    `json:"ai_retry_attempts" nullable:"false" format:"int64" doc:"Retry attempts per AI endpoint."`
	AISystemPrompt       string   `json:"ai_system_prompt" nullable:"false" doc:"Primary AI system prompt."`
	CreatedAt            float64  `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp."`
	DeliveryMethod       string   `json:"delivery_method" nullable:"false" doc:"Delivery method."`
	Directions           []string `json:"directions" nullable:"false" doc:"Research direction preferences."`
	Enabled              bool     `json:"enabled" nullable:"false" doc:"Whether recommendations are enabled."`
	HasAIAPIKey          bool     `json:"has_ai_api_key" nullable:"false" doc:"Whether a primary AI API key is configured."`
	HasAIBackupAPIKey    bool     `json:"has_ai_backup_api_key" nullable:"false" doc:"Whether a backup AI API key is configured."`
	HasPushplusToken     bool     `json:"has_pushplus_token" nullable:"false" doc:"Whether a PushPlus token is configured."`
	ID                   int64    `json:"id" nullable:"false" format:"int64" doc:"Settings row identifier."`
	Keywords             []string `json:"keywords" nullable:"false" doc:"Keyword preferences."`
	PushplusChannel      string   `json:"pushplus_channel" nullable:"false" doc:"PushPlus channel."`
	PushplusTemplate     string   `json:"pushplus_template" nullable:"false" doc:"PushPlus template."`
	PushplusTokenMask    string   `json:"pushplus_token_mask" nullable:"false" doc:"Fixed non-secret mask when a PushPlus token is configured."`
	PushplusTopic        string   `json:"pushplus_topic" nullable:"false" doc:"PushPlus topic."`
	SelectedDatabases    []string `json:"selected_databases" nullable:"false" doc:"Selected index database names."`
	SyncToTrackingFolder bool     `json:"sync_to_tracking_folder" nullable:"false" doc:"Whether PushPlus delivery also syncs to the tracking folder."`
	UpdatedAt            float64  `json:"updated_at" nullable:"false" format:"double" doc:"Last update timestamp."`
	UserID               UserID   `json:"user_id" nullable:"false" format:"int64" doc:"User row identifier."`
}

// NotificationSettingsUpdate describes notification settings update payload.
type NotificationSettingsUpdate struct {
	AIAPIKey             *string  `json:"ai_api_key,omitempty" nullable:"true" doc:"Primary AI API key update: omitted preserves, null clears, and non-empty replaces."`
	AIBackupAPIKey       *string  `json:"ai_backup_api_key,omitempty" nullable:"true" doc:"Backup AI API key update: omitted preserves, null clears, and non-empty replaces."`
	AIBackupBaseURL      string   `json:"ai_backup_base_url,omitempty" nullable:"false" maxLength:"2048" doc:"Backup AI endpoint base URL."`
	AIBackupModel        string   `json:"ai_backup_model,omitempty" nullable:"false" maxLength:"200" doc:"Backup AI model."`
	AIBackupSystemPrompt string   `json:"ai_backup_system_prompt,omitempty" nullable:"false" maxLength:"10000" doc:"Backup AI system prompt."`
	AIBaseURL            string   `json:"ai_base_url,omitempty" nullable:"false" maxLength:"2048" doc:"Primary AI endpoint base URL."`
	AIModel              string   `json:"ai_model,omitempty" nullable:"false" maxLength:"200" doc:"Primary AI model."`
	AIRetryAttempts      int64    `json:"ai_retry_attempts,omitempty" nullable:"false" minimum:"1" maximum:"10" format:"int64" doc:"Retry attempts per AI endpoint."`
	AISystemPrompt       string   `json:"ai_system_prompt,omitempty" nullable:"false" maxLength:"10000" doc:"Primary AI system prompt."`
	DeliveryMethod       string   `json:"delivery_method,omitempty" nullable:"false" doc:"Delivery method."`
	Directions           []string `json:"directions,omitempty" nullable:"false" maxItems:"100" doc:"Research direction preferences."`
	Enabled              bool     `json:"enabled,omitempty" nullable:"false" doc:"Whether recommendations are enabled."`
	Keywords             []string `json:"keywords,omitempty" nullable:"false" maxItems:"100" doc:"Keyword preferences."`
	PushplusChannel      string   `json:"pushplus_channel,omitempty" nullable:"false" maxLength:"64" doc:"PushPlus channel."`
	PushplusTemplate     string   `json:"pushplus_template,omitempty" nullable:"false" maxLength:"64" doc:"PushPlus template."`
	PushplusToken        *string  `json:"pushplus_token,omitempty" nullable:"true" doc:"PushPlus token update: omitted preserves, null clears, and non-empty replaces."`
	PushplusTopic        string   `json:"pushplus_topic,omitempty" nullable:"false" maxLength:"200" doc:"PushPlus topic."`
	SelectedDatabases    []string `json:"selected_databases,omitempty" nullable:"false" maxItems:"500" doc:"Selected index database names."`
	SyncToTrackingFolder bool     `json:"sync_to_tracking_folder,omitempty" nullable:"false" doc:"Whether PushPlus delivery also syncs to the tracking folder."`
}

// OkResponse describes boolean ok response.
type OkResponse struct {
	Ok bool `json:"ok" nullable:"false" doc:"Whether the operation succeeded."`
}

// PageMeta describes pagination metadata.
type PageMeta struct {
	HasMore    *bool   `json:"has_more,omitempty" nullable:"true" doc:"Whether another page may exist."`
	Limit      int64   `json:"limit" nullable:"false" format:"int64" doc:"Page size."`
	NextCursor *string `json:"next_cursor,omitempty" nullable:"true" doc:"Keyset cursor for the next page."`
	Offset     int64   `json:"offset" nullable:"false" format:"int64" doc:"Offset row count."`
	Total      *int64  `json:"total,omitempty" nullable:"true" format:"int64" doc:"Total rows when requested."`
}

// ProviderCapabilityInfo describes aggregated capabilities for one logical Provider.
type ProviderCapabilityInfo struct {
	ArticleAbstract bool   `json:"article_abstract" nullable:"false" doc:"Whether the Provider can resolve an online abstract page."`
	ArticleFullText bool   `json:"article_full_text" nullable:"false" doc:"Whether the Provider can resolve online full text."`
	IndexContent    bool   `json:"index_content" nullable:"false" doc:"Whether the Provider can build canonical index content."`
	Name            string `json:"name" nullable:"false" doc:"Stable lowercase runtime Provider name."`
}

// ProviderCatalogInfo describes safe catalog file metadata visible to administrators.
type ProviderCatalogInfo struct {
	CSVFilename      *string `json:"csv_filename,omitempty" nullable:"true" doc:"Metadata CSV filename when present."`
	DatabaseFilename *string `json:"database_filename,omitempty" nullable:"true" doc:"Content SQLite filename when present."`
	Stem             string  `json:"stem" nullable:"false" doc:"Canonical catalog stem shared by CSV and SQLite files."`
}

// ProviderCatalogResponse describes administrator Provider and catalog capability response.
type ProviderCatalogResponse struct {
	Catalogs  []ProviderCatalogInfo    `json:"catalogs" nullable:"false" doc:"Discovered metadata and content catalogs."`
	Providers []ProviderCapabilityInfo `json:"providers" nullable:"false" doc:"Logical Providers and their aggregate capabilities."`
}

// ProviderOrderConfiguration describes ordered online Provider configuration with optional catalog overrides.
type ProviderOrderConfiguration struct {
	Catalogs map[string][]string `json:"catalogs" nullable:"false" doc:"Complete per-catalog replacement orders keyed by canonical catalog stem."`
	Default  []string            `json:"default" nullable:"false" doc:"Default Provider order used when a catalog has no explicit override."`
}

// PushStats describes push-state statistics payload.
type PushStats struct {
	DBName         string         `json:"db_name" nullable:"false" doc:"State file stem."`
	DeliveredCount *int64         `json:"delivered_count,omitempty" nullable:"true" minimum:"0" doc:"Delivered article count."`
	LastCompleted  *string        `json:"last_completed,omitempty" nullable:"true" doc:"Last completed run timestamp."`
	Status         PushStatsState `json:"status" nullable:"false" enum:"idle,running,completed,failed,skipped,unknown,error" doc:"Push state status."`
	UserResults    *int64         `json:"user_results,omitempty" nullable:"true" minimum:"0" doc:"User result count."`
}

// PushStatsState describes application-owned state exposed for legacy push statistics.
type PushStatsState string

// RegisterRequest describes account registration request.
type RegisterRequest struct {
	InviteCode string  `json:"invite_code" nullable:"false" doc:"Invite code text required for every public registration."`
	Password   string  `json:"password" nullable:"false" doc:"Requested password."`
	Username   string  `json:"username" nullable:"false" doc:"Requested username."`
	Email      *string `json:"email,omitempty" nullable:"true" doc:"Normalized unique account email; null when unset."`
}

// RelationGraphFavoriteSummary describes RelationGraphFavoriteSummary JSON fields.
type RelationGraphFavoriteSummary struct {
	ID            int64   `json:"id" nullable:"false" format:"int64"`
	Title         string  `json:"title" nullable:"false"`
	DBName        string  `json:"db_name" nullable:"false"`
	RootArticleID int64   `json:"root_article_id" nullable:"false" format:"int64"`
	NodeCount     int64   `json:"node_count" nullable:"false"`
	CreatedAt     float64 `json:"created_at" nullable:"false" format:"double"`
	UpdatedAt     float64 `json:"updated_at" nullable:"false" format:"double"`
}

// RelationGraphFavorite describes RelationGraphFavorite JSON fields.
type RelationGraphFavorite struct {
	ID            int64          `json:"id" nullable:"false" format:"int64"`
	Title         string         `json:"title" nullable:"false"`
	DBName        string         `json:"db_name" nullable:"false"`
	RootArticleID int64          `json:"root_article_id" nullable:"false" format:"int64"`
	NodeCount     int64          `json:"node_count" nullable:"false"`
	CreatedAt     float64        `json:"created_at" nullable:"false" format:"double"`
	UpdatedAt     float64        `json:"updated_at" nullable:"false" format:"double"`
	Graph         map[string]any `json:"graph" nullable:"false" doc:"Verbatim JSON object snapshot, at most 2 MiB encoded."`
}

// RelationGraphFavoriteCreate describes RelationGraphFavoriteCreate JSON fields.
type RelationGraphFavoriteCreate struct {
	Title         string         `json:"title" nullable:"false" minLength:"1" maxLength:"200" doc:"Trimmed graph title."`
	DBName        string         `json:"db_name" nullable:"false" maxLength:"255"`
	RootArticleID int64          `json:"root_article_id" nullable:"false" minimum:"1" format:"int64"`
	NodeCount     int64          `json:"node_count" nullable:"false" minimum:"1" maximum:"500"`
	Graph         map[string]any `json:"graph" nullable:"false" doc:"Verbatim JSON object snapshot, at most 2 MiB encoded."`
}

// RelationGraphFavoriteUpdate describes RelationGraphFavoriteUpdate JSON fields.
type RelationGraphFavoriteUpdate struct {
	Title     string         `json:"title,omitempty" nullable:"false" minLength:"1" maxLength:"200" doc:"Trimmed graph title."`
	NodeCount int64          `json:"node_count,omitempty" nullable:"false" minimum:"1" maximum:"500"`
	Graph     map[string]any `json:"graph,omitempty" nullable:"false" doc:"Verbatim JSON object snapshot, at most 2 MiB encoded."`
}

// RelationGraphFavoritePage describes RelationGraphFavoritePage JSON fields.
type RelationGraphFavoritePage struct {
	Items      []RelationGraphFavoriteSummary `json:"items" nullable:"false"`
	NextCursor *string                        `json:"next_cursor" nullable:"true"`
}

// RuntimeSecretItemInfo describes individually manageable secret-pool item metadata.
type RuntimeSecretItemInfo struct {
	MaskedValue string `json:"masked_value" nullable:"false" doc:"Human-readable prefix mask that never contains the complete secret."`
	Reference   string `json:"reference" nullable:"false" doc:"Opaque authenticated reference accepted by a pool removal update."`
}

// RuntimeSecretPoolUpdate describes additions and removals applied to one secret runtime pool.
type RuntimeSecretPoolUpdate struct {
	Add    []string `json:"add,omitempty" nullable:"false" doc:"New plaintext entries to normalize and append."`
	Remove []string `json:"remove,omitempty" nullable:"false" doc:"Opaque item references to remove."`
}

// RuntimeSettingApplyMode describes lifecycle point at which a saved runtime setting becomes effective.
type RuntimeSettingApplyMode string

// RuntimeSettingControl describes frontend control requested for a runtime setting.
type RuntimeSettingControl string

// RuntimeSettingGroup describes functional section used to group runtime settings in administrator clients.
type RuntimeSettingGroup string

// RuntimeSettingInfo describes runtime setting response payload.
type RuntimeSettingInfo struct {
	AllowedValues []string                `json:"allowed_values" nullable:"false" doc:"Finite allowed values for selection controls."`
	ApplyMode     RuntimeSettingApplyMode `json:"apply_mode" nullable:"false" enum:"next_request,next_command,restart_required" doc:"Lifecycle point at which changes become effective."`
	Control       RuntimeSettingControl   `json:"control" nullable:"false" enum:"text,secret_pool,string_list,boolean,select,index_provider_routes,provider_order,provider_proxy_policy" doc:"Requested frontend control."`
	Description   string                  `json:"description" nullable:"false" doc:"Human-readable description."`
	Field         string                  `json:"field" nullable:"false" doc:"API field name."`
	Group         RuntimeSettingGroup     `json:"group" nullable:"false" enum:"source_access,provider_routing,server_security,single_sign_on,observability" doc:"Functional administrator configuration group."`
	HasValue      bool                    `json:"has_value" nullable:"false" doc:"Whether a credential value is configured."`
	InputType     string                  `json:"input_type" nullable:"false" doc:"Frontend input type."`
	IsSecret      bool                    `json:"is_secret" nullable:"false" doc:"Whether the value contains credentials."`
	Label         string                  `json:"label" nullable:"false" doc:"Human-readable label."`
	MaskedValue   string                  `json:"masked_value" nullable:"false" doc:"Fixed non-secret mask when a credential value is configured."`
	SecretItems   []RuntimeSecretItemInfo `json:"secret_items,omitempty" nullable:"false" doc:"Individually manageable masked entries for a secret pool."`
	Source        string                  `json:"source" nullable:"false" doc:"Effective setting source."`
	UpdatedAt     *float64                `json:"updated_at,omitempty" nullable:"true" format:"double" doc:"Database update timestamp."`
	Value         string                  `json:"value" nullable:"false" doc:"Effective setting value."`
}

// RuntimeSettingsUpdate describes runtime settings update payload.
type RuntimeSettingsUpdate struct {
	SecretPoolUpdates map[string]RuntimeSecretPoolUpdate `json:"secret_pool_updates,omitempty" nullable:"false" doc:"Incremental secret-pool mutations keyed by API field name."`
	Values            map[string]*string                 `json:"values,omitempty" nullable:"false" doc:"Values keyed by API field name; null clears and a blank secret preserves."`
}

// ScheduledDeliveryJob describes arguments accepted by a notification or push scheduled job.
type ScheduledDeliveryJob struct {
	Database      *string `json:"database,omitempty" nullable:"true" maxLength:"128" pattern:"^[A-Za-z0-9_-]+(?:\\.[A-Za-z0-9_-]+)*\\.sqlite$" doc:"Optional index database basename."`
	MaxCandidates *int64  `json:"max_candidates,omitempty" nullable:"true" minimum:"1" maximum:"1000" doc:"Optional upper bound for recommendation candidates."`
}

// ScheduledIndexJob describes arguments accepted by an index scheduled job.
type ScheduledIndexJob struct {
	MetadataFile *string `json:"metadata_file,omitempty" nullable:"true" maxLength:"128" pattern:"^[A-Za-z0-9_-]+(?:\\.[A-Za-z0-9_-]+)*\\.csv$" doc:"Optional metadata CSV basename."`
	Notify       bool    `json:"notify,omitempty" nullable:"false" doc:"Whether notification delivery runs after indexing succeeds."`
	Push         bool    `json:"push,omitempty" nullable:"false" doc:"Whether push delivery runs after indexing succeeds."`
}

// ScheduledTaskCreate describes scheduled task creation payload.
type ScheduledTaskCreate struct {
	Coalesce       bool             `json:"coalesce,omitempty" nullable:"false" doc:"Whether missed slots are collapsed to the latest slot."`
	Cron           string           `json:"cron" nullable:"false" doc:"Five-field cron expression."`
	Enabled        bool             `json:"enabled,omitempty" nullable:"false" doc:"Whether the task is enabled."`
	Job            ScheduledJobSpec `json:"job" nullable:"false" doc:"Validated job specification."`
	Name           string           `json:"name" nullable:"false" doc:"Display name."`
	TimeoutSeconds int64            `json:"timeout_seconds,omitempty" nullable:"false" minimum:"1" maximum:"86400" format:"int64" doc:"Maximum execution time in seconds."`
	Timezone       string           `json:"timezone,omitempty" nullable:"false" doc:"Explicit IANA time zone used for cron evaluation."`
}

// ScheduledTaskInfo describes scheduled task response payload.
type ScheduledTaskInfo struct {
	Coalesce       bool              `json:"coalesce" nullable:"false" doc:"Whether missed slots are collapsed to the latest slot."`
	CreatedAt      float64           `json:"created_at" nullable:"false" format:"double" doc:"Creation timestamp."`
	Cron           string            `json:"cron" nullable:"false" doc:"Five-field cron expression."`
	Enabled        bool              `json:"enabled" nullable:"false" doc:"Whether the task is enabled."`
	ID             int64             `json:"id" nullable:"false" format:"int64" doc:"Scheduled task row identifier."`
	Job            *ScheduledJobSpec `json:"job,omitempty" nullable:"false" doc:"Validated job specification, absent only for a migrated legacy row."`
	LastRunAt      *float64          `json:"last_run_at,omitempty" nullable:"true" format:"double" doc:"Last run timestamp."`
	LastStatus     SchedulerRunState `json:"last_status" nullable:"false" enum:"idle,pending,claimed,running,success,failed,timed_out,error,unknown,cancelled" doc:"Last run status."`
	LegacyCommand  *string           `json:"legacy_command,omitempty" nullable:"true" doc:"Read-only command text retained from a legacy row for administrator review."`
	Name           string            `json:"name" nullable:"false" doc:"Display name."`
	TimeoutSeconds int64             `json:"timeout_seconds" nullable:"false" minimum:"0" format:"int64" doc:"Maximum execution time in seconds."`
	Timezone       string            `json:"timezone" nullable:"false" doc:"Explicit IANA time zone used for cron evaluation."`
	UpdatedAt      float64           `json:"updated_at" nullable:"false" format:"double" doc:"Last update timestamp."`
}

// ScheduledTaskRunInfo describes persisted scheduled task run visible to administrators.
type ScheduledTaskRunInfo struct {
	ClaimedAt    *float64          `json:"claimed_at,omitempty" nullable:"true" format:"double" doc:"Claim timestamp."`
	FinishedAt   *float64          `json:"finished_at,omitempty" nullable:"true" format:"double" doc:"Terminal or unknown-state timestamp."`
	ID           int64             `json:"id" nullable:"false" format:"int64" doc:"Run row identifier."`
	ScheduledFor int64             `json:"scheduled_for" nullable:"false" format:"int64" doc:"UTC Unix timestamp of the scheduled minute or explicit manual request."`
	StartedAt    *float64          `json:"started_at,omitempty" nullable:"true" format:"double" doc:"Execution start timestamp."`
	Status       SchedulerRunState `json:"status" nullable:"false" enum:"idle,pending,claimed,running,success,failed,timed_out,error,unknown,cancelled" doc:"Durable run status."`
	TaskID       int64             `json:"task_id" nullable:"false" format:"int64" doc:"Scheduled task row identifier."`
	TaskName     string            `json:"task_name" nullable:"false" doc:"Task name captured when the slot was queued."`
	WorkerID     *string           `json:"worker_id,omitempty" nullable:"true" doc:"Worker identifier currently owning the run."`
}

// ScheduledTaskUpdate describes scheduled task update payload.
type ScheduledTaskUpdate struct {
	Coalesce       *bool             `json:"coalesce,omitempty" nullable:"true" doc:"Optional coalescing flag."`
	Cron           *string           `json:"cron,omitempty" nullable:"true" doc:"Optional replacement cron expression."`
	Enabled        *bool             `json:"enabled,omitempty" nullable:"true" doc:"Optional enabled flag."`
	Job            *ScheduledJobSpec `json:"job,omitempty" nullable:"false" doc:"Optional replacement job specification."`
	Name           *string           `json:"name,omitempty" nullable:"true" doc:"Optional replacement display name."`
	TimeoutSeconds *int64            `json:"timeout_seconds,omitempty" nullable:"true" minimum:"1" maximum:"86400" format:"int64" doc:"Optional replacement timeout in seconds."`
	Timezone       *string           `json:"timezone,omitempty" nullable:"true" doc:"Optional replacement IANA time zone."`
}

// SchedulerRunState describes application-owned durable scheduler run state.
type SchedulerRunState string

// SchedulerStatusResponse describes administrator scheduler status response.
type SchedulerStatusResponse struct {
	LastCheckedAt *float64               `json:"last_checked_at,omitempty" nullable:"true" format:"double" doc:"Last completed scheduler wall-clock check."`
	RecentRuns    []ScheduledTaskRunInfo `json:"recent_runs" nullable:"false" doc:"Recent durable task runs ordered from newest to oldest."`
	Workers       []SchedulerWorkerInfo  `json:"workers" nullable:"false" doc:"Known worker heartbeats ordered from newest to oldest."`
}

// SchedulerWorkerInfo describes persisted scheduler worker heartbeat visible to administrators.
type SchedulerWorkerInfo struct {
	HeartbeatAt float64 `json:"heartbeat_at" nullable:"false" format:"double" doc:"Most recent heartbeat timestamp."`
	IsHealthy   bool    `json:"is_healthy" nullable:"false" doc:"Whether the heartbeat is within the health threshold."`
	StartedAt   float64 `json:"started_at" nullable:"false" format:"double" doc:"Worker start timestamp."`
	WorkerID    string  `json:"worker_id" nullable:"false" doc:"Stable worker process identifier."`
}

// TokenCreateRequest describes access token creation request.
type TokenCreateRequest struct {
	Name string `json:"name,omitempty" nullable:"false" maxLength:"100" doc:"Token display name."`
	Ttl  int64  `json:"ttl,omitempty" nullable:"false" minimum:"3600" maximum:"31536000" format:"int64" doc:"Requested token TTL in seconds."`
}

// TokenCreateResponse describes access token creation response.
type TokenCreateResponse struct {
	ExpiresAt float64 `json:"expires_at" nullable:"false" format:"double" doc:"Token expiration timestamp."`
	ID        int64   `json:"id" nullable:"false" format:"int64" doc:"Token row identifier."`
	Name      string  `json:"name" nullable:"false" doc:"Token display name."`
	Token     string  `json:"token" nullable:"false" doc:"Raw token value returned only at creation time."`
}

// TokenInfo describes access token metadata response.
type TokenInfo struct {
	CreatedAt float64 `json:"created_at" nullable:"false" format:"double" doc:"Token creation timestamp."`
	ExpiresAt float64 `json:"expires_at" nullable:"false" format:"double" doc:"Token expiration timestamp."`
	ID        int64   `json:"id" nullable:"false" format:"int64" doc:"Token row identifier."`
	Name      string  `json:"name" nullable:"false" doc:"Token display name."`
}

// TrackingFolderSummary describes tracking folder summary for `/api/tracking/status`.
type TrackingFolderSummary struct {
	ID   int64  `json:"id" nullable:"false" format:"int64" doc:"Folder row identifier."`
	Name string `json:"name" nullable:"false" doc:"Folder display name."`
}

// TrackingSetRequest describes set-tracking-folder request payload.
type TrackingSetRequest struct {
	FolderID int64 `json:"folder_id" nullable:"false" format:"int64" doc:"Folder row identifier to mark as tracking."`
}

// TrackingStatusResponse describes tracking status response payload.
type TrackingStatusResponse struct {
	NotificationConfigured  bool                   `json:"notification_configured" nullable:"false" doc:"Whether notification settings exist for the user."`
	TotalFolders            int64                  `json:"total_folders" nullable:"false" minimum:"0" doc:"Total folder count."`
	TrackingFolder          *TrackingFolderSummary `json:"tracking_folder,omitempty" nullable:"false" doc:"Current tracking folder summary."`
	WeeklyArticlesAvailable int64                  `json:"weekly_articles_available" nullable:"false" minimum:"0" doc:"Number of weekly article ids currently available."`
}

// TransformSchema retains explicitly nullable object fields.
func (TrackingStatusResponse) TransformSchema(_ huma.Registry, schema *huma.Schema) *huma.Schema {
	schema.Properties["tracking_folder"] = nullableSchema(schema.Properties["tracking_folder"])
	return schema
}

// UserID describes auth user identifier stored as an ordinary SQLite integer.
type UserID int64

// UserResponse describes user profile returned by auth endpoints.
type UserResponse struct {
	ID           UserID  `json:"id" nullable:"false" format:"int64" doc:"User identifier."`
	IsAdmin      bool    `json:"is_admin" nullable:"false" doc:"Whether the user has admin privileges."`
	Username     string  `json:"username" nullable:"false" doc:"Login username."`
	Email        *string `json:"email" nullable:"true" doc:"Normalized unique account email; null when unset."`
	AuthKind     string  `json:"auth_kind" nullable:"false" enum:"local,oidc" doc:"Account credential kind."`
	IsOIDCLinked bool    `json:"is_oidc_linked" nullable:"false" doc:"Whether an external OIDC identity is linked."`
}

// ValueCount describes label/count tuple.
type ValueCount struct {
	Count int64  `json:"count" nullable:"false" format:"int64" doc:"Row count."`
	Value string `json:"value" nullable:"false" doc:"Label value."`
}

// WeeklyArticlePage describes cursor-paginated weekly articles for one database and journal.
type WeeklyArticlePage struct {
	Items []WeeklyArticleRecord `json:"items" nullable:"false" doc:"Weekly article records."`
	Page  PageMeta              `json:"page" nullable:"false" doc:"Descending date and article-id pagination metadata."`
}

// WeeklyArticleRecord describes weekly article record.
type WeeklyArticleRecord struct {
	Abstract        *string        `json:"abstract,omitempty" nullable:"true" doc:"Abstract text."`
	ArticleID       ArticleID      `json:"article_id" nullable:"false" doc:"Article identifier."`
	Authors         []string       `json:"authors" nullable:"false" doc:"Authors text."`
	Date            *string        `json:"date,omitempty" nullable:"true" doc:"Article date."`
	DatePrecision   *DatePrecision `json:"date_precision,omitempty" nullable:"true" enum:"year,month,day" doc:"Precision represented by the article date."`
	DOI             *string        `json:"doi,omitempty" nullable:"true" doc:"DOI."`
	InPress         *bool          `json:"in_press,omitempty" nullable:"true" doc:"In-press flag."`
	IssueID         *int64         `json:"issue_id,omitempty" nullable:"true" format:"int64" doc:"Issue identifier."`
	JournalID       JournalID      `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	JournalTitle    string         `json:"journal_title" nullable:"false" doc:"Journal title."`
	Number          *string        `json:"number,omitempty" nullable:"true" doc:"Issue number."`
	OpenAccess      *bool          `json:"open_access,omitempty" nullable:"true" doc:"Open-access flag."`
	PublicationYear *int64         `json:"publication_year,omitempty" nullable:"true" format:"int64" doc:"Publication year."`
	Title           string         `json:"title" nullable:"false" doc:"Article title."`
	Volume          *string        `json:"volume,omitempty" nullable:"true" doc:"Issue volume."`
}

// WeeklyDatabaseSummary describes weekly article counts for one database without article bodies.
type WeeklyDatabaseSummary struct {
	DBName          string                 `json:"db_name" nullable:"false" doc:"Database filename."`
	GeneratedAt     string                 `json:"generated_at" nullable:"false" doc:"Generated timestamp from the newest included manifest."`
	Journals        []WeeklyJournalSummary `json:"journals" nullable:"false" doc:"Journal summaries ordered by count and label."`
	NewArticleCount int64                  `json:"new_article_count" nullable:"false" minimum:"0" doc:"New article count within the fixed window."`
	RunID           *string                `json:"run_id,omitempty" nullable:"true" doc:"Source run identifier from the newest included manifest."`
}

// WeeklyDatabaseUpdate describes weekly update summary for one database.
type WeeklyDatabaseUpdate struct {
	DBName          string                `json:"db_name" nullable:"false" doc:"Database filename."`
	GeneratedAt     string                `json:"generated_at" nullable:"false" doc:"Generated timestamp."`
	Journals        []WeeklyJournalUpdate `json:"journals" nullable:"false" doc:"Journal groups."`
	NewArticleCount int64                 `json:"new_article_count" nullable:"false" minimum:"0" doc:"New article count."`
	RunID           *string               `json:"run_id,omitempty" nullable:"true" doc:"Source run identifier."`
}

// WeeklyJournalSummary describes weekly article counts for one journal without article bodies.
type WeeklyJournalSummary struct {
	JournalID       JournalID `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	JournalTitle    *string   `json:"journal_title,omitempty" nullable:"true" doc:"Journal title."`
	NewArticleCount int64     `json:"new_article_count" nullable:"false" minimum:"0" doc:"New article count within the fixed window."`
}

// WeeklyJournalUpdate describes weekly update summary for one journal.
type WeeklyJournalUpdate struct {
	Articles        []WeeklyArticleRecord `json:"articles" nullable:"false" doc:"Weekly articles."`
	JournalID       JournalID             `json:"journal_id" nullable:"false" doc:"Journal identifier."`
	JournalTitle    *string               `json:"journal_title,omitempty" nullable:"true" doc:"Journal title."`
	NewArticleCount int64                 `json:"new_article_count" nullable:"false" minimum:"0" doc:"New article count."`
}

// WeeklyUpdatesResponse describes weekly updates response.
type WeeklyUpdatesResponse struct {
	Databases   []WeeklyDatabaseUpdate `json:"databases" nullable:"false" doc:"Database update groups."`
	GeneratedAt string                 `json:"generated_at" nullable:"false" doc:"Response generated timestamp."`
	WindowEnd   string                 `json:"window_end" nullable:"false" doc:"Window end timestamp."`
	WindowStart string                 `json:"window_start" nullable:"false" doc:"Window start timestamp."`
}

// WeeklyUpdatesSummaryResponse describes bounded weekly update summary without article bodies.
type WeeklyUpdatesSummaryResponse struct {
	Databases   []WeeklyDatabaseSummary `json:"databases" nullable:"false" doc:"Database update summaries."`
	GeneratedAt string                  `json:"generated_at" nullable:"false" doc:"Response generated timestamp."`
	WindowEnd   string                  `json:"window_end" nullable:"false" doc:"Fixed window end timestamp for article-page requests."`
	WindowStart string                  `json:"window_start" nullable:"false" doc:"Window start timestamp."`
}

// YearSummary describes publication year summary.
type YearSummary struct {
	IssueCount   int64 `json:"issue_count" nullable:"false" format:"int64" doc:"Issue count."`
	JournalCount int64 `json:"journal_count" nullable:"false" format:"int64" doc:"Journal count."`
	Year         int64 `json:"year" nullable:"false" format:"int64" doc:"Publication year."`
}

// EmailUpdateRequest describes EmailUpdateRequest JSON fields.
type EmailUpdateRequest struct {
	Email *string `json:"email" nullable:"true" doc:"Normalized unique account email; null when unset."`
}

// AuthProvidersResponse describes AuthProvidersResponse JSON fields.
type AuthProvidersResponse struct {
	OIDCEnabled     bool   `json:"oidc_enabled" nullable:"false"`
	OIDCDisplayName string `json:"oidc_display_name" nullable:"false"`
}

// OIDCLinkResponse describes OIDCLinkResponse JSON fields.
type OIDCLinkResponse struct {
	AuthorizationURL string `json:"authorization_url" nullable:"false"`
}

// TelemetrySeries describes TelemetrySeries JSON fields.
type TelemetrySeries struct {
	Kind          string  `json:"kind" nullable:"false" enum:"http,mcp,search"`
	Name          string  `json:"name" nullable:"false"`
	Method        string  `json:"method,omitempty" nullable:"false"`
	StatusClass   string  `json:"status_class,omitempty" nullable:"false" enum:"2xx,3xx,4xx,5xx"`
	Mode          string  `json:"mode,omitempty" nullable:"false" enum:"simple,advanced"`
	Database      string  `json:"database,omitempty" nullable:"false"`
	TermCount     string  `json:"term_count,omitempty" nullable:"false" enum:"0,1,2-4,5-9,10+"`
	Hits          string  `json:"hits,omitempty" nullable:"false" enum:"0,1-9,10-99,100-999,1000+"`
	IsError       bool    `json:"is_error" nullable:"false"`
	UsesPrefix    bool    `json:"uses_prefix" nullable:"false"`
	UsesBoolean   bool    `json:"uses_boolean" nullable:"false"`
	Calls         int64   `json:"calls" nullable:"false" minimum:"0" format:"int64"`
	Errors        int64   `json:"errors" nullable:"false" minimum:"0" format:"int64"`
	ErrorRate     float64 `json:"error_rate" nullable:"false" minimum:"0" maximum:"1" format:"double"`
	P50Ms         float64 `json:"p50_ms" nullable:"false" minimum:"0" format:"double"`
	P95Ms         float64 `json:"p95_ms" nullable:"false" minimum:"0" format:"double"`
	P99Ms         float64 `json:"p99_ms" nullable:"false" minimum:"0" format:"double"`
	HitCountBasis string  `json:"hit_count_basis,omitempty" nullable:"false" enum:"total,returned,candidates"`
}

// TelemetryHour describes TelemetryHour JSON fields.
type TelemetryHour struct {
	Hour   string `json:"hour" nullable:"false" format:"date-time"`
	Kind   string `json:"kind" nullable:"false" enum:"http,mcp,search"`
	Calls  int64  `json:"calls" nullable:"false" minimum:"0" format:"int64"`
	Errors int64  `json:"errors" nullable:"false" minimum:"0" format:"int64"`
}

// TelemetrySearchShape describes TelemetrySearchShape JSON fields.
type TelemetrySearchShape struct {
	Feature string `json:"feature" nullable:"false" enum:"endpoint,mode,database,term_count,uses_prefix,uses_boolean,hits,hit_count_basis"`
	Value   string `json:"value" nullable:"false"`
	Calls   int64  `json:"calls" nullable:"false" minimum:"0" format:"int64"`
}

// TelemetryResponse describes TelemetryResponse JSON fields.
type TelemetryResponse struct {
	Enabled      bool                   `json:"enabled" nullable:"false"`
	From         string                 `json:"from" nullable:"false" format:"date-time"`
	To           string                 `json:"to" nullable:"false" format:"date-time"`
	Series       []TelemetrySeries      `json:"series" nullable:"false"`
	Hourly       []TelemetryHour        `json:"hourly" nullable:"false"`
	SearchShapes []TelemetrySearchShape `json:"search_shapes" nullable:"false"`
}

// ScheduledJobSpec describes a scheduler command and its variant arguments.
type ScheduledJobSpec map[string]any
