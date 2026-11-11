package api

import (
	"encoding/json"

	delivery "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	scheduled "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/danielgtaylor/huma/v2"
)

// AdminInviteCodeCreate describes optional administrator overrides for a newly issued invite code.
type AdminInviteCodeCreate struct {
	// ExpiresAt describes absolute expiration timestamp; omitted values use the seven-day default.
	ExpiresAt *float64 `json:"expires_at,omitempty" nullable:"true" format:"double" doc:"Absolute expiration timestamp; omitted values use the seven-day default."`
	// MaxUses describes registration quota; omitted values use one redemption.
	MaxUses *int64 `json:"max_uses,omitempty" nullable:"true" minimum:"1" maximum:"1000" format:"int64" doc:"Registration quota; omitted values use one redemption."`
}

// AdminResetPassword describes admin password reset request payload.
type AdminResetPassword struct {
	// NewPassword describes replacement password.
	NewPassword string `json:"new_password" nullable:"false" doc:"Replacement password."`
}

// AdminSetAdmin describes admin grant/revoke request payload.
type AdminSetAdmin struct {
	// IsAdmin describes replacement admin flag.
	IsAdmin bool `json:"is_admin" nullable:"false" doc:"Replacement admin flag."`
}

// AnnouncementCreate describes announcement creation payload.
type AnnouncementCreate struct {
	// Enabled reports whether the announcement is visible.
	Enabled *bool `json:"enabled,omitempty" nullable:"false" doc:"Whether the announcement is visible." default:"true"`
	// Message describes announcement message body.
	Message string `json:"message" nullable:"false" minLength:"1" maxLength:"10000" doc:"Announcement message body."`
	// Priority describes priority label.
	Priority string `json:"priority,omitempty" nullable:"false" enum:"high,normal,low" minLength:"1" maxLength:"16" doc:"Priority label." default:"normal"`
	// Title describes announcement title.
	Title string `json:"title" nullable:"false" minLength:"1" maxLength:"200" doc:"Announcement title."`
}

// AnnouncementUpdate describes announcement update payload.
type AnnouncementUpdate struct {
	// Enabled describes optional enabled flag.
	Enabled *bool `json:"enabled,omitempty" nullable:"true" doc:"Optional enabled flag."`
	// Message describes optional replacement message.
	Message *string `json:"message,omitempty" nullable:"true" minLength:"1" maxLength:"10000" doc:"Optional replacement message."`
	// Priority describes optional replacement priority.
	Priority *string `json:"priority,omitempty" nullable:"true" enum:"high,normal,low" minLength:"1" maxLength:"16" doc:"Optional replacement priority."`
	// Title describes optional replacement title.
	Title *string `json:"title,omitempty" nullable:"true" minLength:"1" maxLength:"200" doc:"Optional replacement title."`
}

// ChangePasswordRequest describes password change request.
type ChangePasswordRequest struct {
	// NewPassword describes replacement password.
	NewPassword string `json:"new_password" nullable:"false" doc:"Replacement password."`
	// OldPassword describes current password.
	OldPassword string `json:"old_password" nullable:"false" doc:"Current password."`
}

// CNKILoginPollRequest describes zhejiang Library QR login polling parameters.
type CNKILoginPollRequest struct {
	// IntervalSeconds describes poll interval in seconds.
	IntervalSeconds *float64 `json:"interval_seconds,omitempty" nullable:"false" minimum:"0.1" maximum:"10" format:"double" doc:"Poll interval in seconds." default:"2"`
	// TimeoutSeconds describes poll timeout in seconds.
	TimeoutSeconds *int64 `json:"timeout_seconds,omitempty" nullable:"false" minimum:"1" maximum:"600" format:"int64" doc:"Poll timeout in seconds." default:"180"`
}

// FavoriteAdd describes favorite creation request payload.
type FavoriteAdd struct {
	// ArticleID describes article identifier.
	ArticleID identity.ID `json:"article_id" nullable:"false" pattern:"^[1-9][0-9]*$" doc:"Article identifier."`
	// DBName describes source index database name.
	DBName string `json:"db_name,omitempty" nullable:"false" maxLength:"255" doc:"Source index database name."`
	// Note describes user note text.
	Note string `json:"note,omitempty" nullable:"false" maxLength:"2000" doc:"User note text."`
}

// FavoriteArticleRef describes favorite article reference used by bulk operations.
type FavoriteArticleRef struct {
	// ArticleID describes article identifier.
	ArticleID identity.ID `json:"article_id" nullable:"false" pattern:"^[1-9][0-9]*$" doc:"Article identifier."`
	// DBName describes source index database name.
	DBName string `json:"db_name,omitempty" nullable:"false" maxLength:"255" doc:"Source index database name."`
}

// FavoriteBatchCheckRequest describes batch favorite check request.
type FavoriteBatchCheckRequest struct {
	// ArticleIDs describes article identifiers to check.
	ArticleIDs []identity.ID `json:"article_ids" nullable:"false" maxItems:"500" doc:"Article identifiers to check."`
	// DBName describes source index database name.
	DBName string `json:"db_name,omitempty" nullable:"false" maxLength:"255" doc:"Source index database name."`
}

// FavoriteBulkAdd describes bulk favorite add request.
type FavoriteBulkAdd struct {
	// Articles describes favorite articles to add.
	Articles []FavoriteAdd `json:"articles" nullable:"false" maxItems:"500" doc:"Favorite articles to add."`
}

// FavoriteBulkMove describes bulk favorite move request.
type FavoriteBulkMove struct {
	// Articles describes favorite articles to move.
	Articles []FavoriteArticleRef `json:"articles" nullable:"false" maxItems:"500" doc:"Favorite articles to move."`
	// TargetFolderID describes target folder row identifier.
	TargetFolderID int64 `json:"target_folder_id" nullable:"false" minimum:"1" format:"int64" doc:"Target folder row identifier."`
}

// FavoriteBulkRemove describes bulk favorite remove request.
type FavoriteBulkRemove struct {
	// Articles describes favorite articles to remove.
	Articles []FavoriteArticleRef `json:"articles" nullable:"false" maxItems:"500" doc:"Favorite articles to remove."`
}

// FolderCreate describes create-folder request payload.
type FolderCreate struct {
	// IsTracking reports whether the folder should become the user's tracking folder.
	IsTracking bool `json:"is_tracking,omitempty" nullable:"false" doc:"Whether the folder should become the user's tracking folder."`
	// Name describes folder display name.
	Name string `json:"name" nullable:"false" minLength:"1" maxLength:"100" doc:"Folder display name."`
}

// FolderRename describes rename-folder request payload.
type FolderRename struct {
	// Name describes replacement folder display name.
	Name string `json:"name" nullable:"false" minLength:"1" maxLength:"100" doc:"Replacement folder display name."`
}

// NativeOIDCExchangeRequest describes native OIDC authorization code exchange with client PKCE. No session cookie is issued.
type NativeOIDCExchangeRequest struct {
	// Code describes one-time unpadded base64url code (32 random bytes); TTL 120 seconds.
	Code string `json:"code" nullable:"false" doc:"One-time unpadded base64url code (32 random bytes); TTL 120 seconds."`
	// CodeVerifier contains the code verifier value.
	CodeVerifier string `json:"code_verifier" nullable:"false" minLength:"43" maxLength:"128" pattern:"^[A-Za-z0-9._~-]{43,128}$"`
	// RedirectURI describes exact native redirect_uri supplied at login.
	RedirectURI string `json:"redirect_uri" nullable:"false" doc:"Exact native redirect_uri supplied at login."`
	// DeviceName describes device label; surrounding whitespace is trimmed, blank text and control characters are rejected. The raw input is limited to 93 Unicode code points.
	DeviceName string `json:"device_name" nullable:"false" minLength:"1" maxLength:"93" doc:"Device label; surrounding whitespace is trimmed, blank text and control characters are rejected. The raw input is limited to 93 Unicode code points."`
}

// NativeLoginRequest describes native password login that issues an independent 90-day personal access token.
type NativeLoginRequest struct {
	// Password describes password.
	Password string `json:"password" nullable:"false" doc:"Password."`
	// Username describes username or case-insensitive email address.
	Username string `json:"username" nullable:"false" doc:"Username or case-insensitive email address."`
	// DeviceName describes device label; surrounding whitespace is trimmed, blank text and control characters are rejected. The raw input is limited to 93 Unicode code points.
	DeviceName string `json:"device_name" nullable:"false" minLength:"1" maxLength:"93" doc:"Device label; surrounding whitespace is trimmed, blank text and control characters are rejected. The raw input is limited to 93 Unicode code points."`
}

// LoginRequest describes login request.
type LoginRequest struct {
	// Password describes password.
	Password string `json:"password" nullable:"false" doc:"Password."`
	// Username describes username or case-insensitive email address.
	Username string `json:"username" nullable:"false" doc:"Username or case-insensitive email address."`
}

// RegisterRequest describes account registration request.
type RegisterRequest struct {
	// InviteCode describes invite code text required for every public registration.
	InviteCode string `json:"invite_code" nullable:"false" doc:"Invite code text required for every public registration."`
	// Password describes requested password.
	Password string `json:"password" nullable:"false" doc:"Requested password."`
	// Username describes requested username.
	Username string `json:"username" nullable:"false" doc:"Requested username."`
	// Email describes normalized unique account email; null when unset.
	Email *string `json:"email,omitempty" nullable:"true" doc:"Normalized unique account email; null when unset."`
}

// RelationGraphFavoriteCreate describes RelationGraphFavoriteCreate JSON fields.
type RelationGraphFavoriteCreate struct {
	// Title describes trimmed graph title.
	Title string `json:"title" nullable:"false" minLength:"1" maxLength:"200" doc:"Trimmed graph title."`
	// DBName describes source index database name.
	DBName string `json:"db_name" nullable:"false" maxLength:"255"`
	// RootArticleID contains the root article id value.
	RootArticleID identity.ID `json:"root_article_id" nullable:"false" pattern:"^[1-9][0-9]*$"`
	// NodeCount contains the node count value.
	NodeCount int64 `json:"node_count" nullable:"false" minimum:"1" maximum:"500"`
	// Graph describes verbatim JSON object snapshot, at most 2 MiB encoded.
	Graph json.RawMessage `json:"graph" nullable:"false" doc:"Verbatim JSON object snapshot, at most 2 MiB encoded."`
}

// RelationGraphFavoriteUpdate describes RelationGraphFavoriteUpdate JSON fields.
type RelationGraphFavoriteUpdate struct {
	// Title describes trimmed graph title.
	Title *string `json:"title,omitempty" nullable:"false" minLength:"1" maxLength:"200" doc:"Trimmed graph title."`
	// NodeCount contains the node count value.
	NodeCount *int64 `json:"node_count,omitempty" nullable:"false" minimum:"1" maximum:"500"`
	// Graph describes verbatim JSON object snapshot, at most 2 MiB encoded.
	Graph json.RawMessage `json:"graph,omitempty" nullable:"false" doc:"Verbatim JSON object snapshot, at most 2 MiB encoded."`
}

// RuntimeSecretPoolUpdate describes additions and removals applied to one secret runtime pool.
type RuntimeSecretPoolUpdate struct {
	// Add describes new plaintext entries to normalize and append.
	Add []string `json:"add,omitempty" nullable:"false" doc:"New plaintext entries to normalize and append."`
	// Remove describes opaque item references to remove.
	Remove []string `json:"remove,omitempty" nullable:"false" doc:"Opaque item references to remove."`
}

// RuntimeSettingsUpdate describes runtime settings update payload.
type RuntimeSettingsUpdate struct {
	// SecretPoolUpdates describes incremental secret-pool mutations keyed by API field name.
	SecretPoolUpdates map[string]RuntimeSecretPoolUpdate `json:"secret_pool_updates,omitempty" nullable:"false" doc:"Incremental secret-pool mutations keyed by API field name."`
	// Values describes values keyed by API field name; null clears and a blank secret preserves.
	Values map[string]*string `json:"values,omitempty" nullable:"false" doc:"Values keyed by API field name; null clears and a blank secret preserves."`
}

// TransformSchema documents JSON encoded in individual runtime setting strings.
func (RuntimeSettingsUpdate) TransformSchema(_ huma.Registry, schema *huma.Schema) *huma.Schema {
	openapi.DescribeRuntimeSettingValues(schema.Properties["values"])
	return schema
}

// ScheduledTaskCreate describes scheduled task creation payload.
type ScheduledTaskCreate struct {
	// Coalesce reports whether missed slots are collapsed to the latest slot.
	Coalesce *bool `json:"coalesce,omitempty" nullable:"false" doc:"Whether missed slots are collapsed to the latest slot." default:"true"`
	// Cron describes five-field cron expression.
	Cron string `json:"cron" nullable:"false" minLength:"1" doc:"Five-field cron expression."`
	// Enabled reports whether the task is enabled.
	Enabled *bool `json:"enabled,omitempty" nullable:"false" doc:"Whether the task is enabled." default:"true"`
	// Job describes validated job specification.
	Job scheduled.Job `json:"job" nullable:"false" doc:"Validated job specification."`
	// Name describes display name.
	Name string `json:"name" nullable:"false" minLength:"1" doc:"Display name."`
	// TimeoutSeconds describes maximum execution time in seconds.
	TimeoutSeconds *int64 `json:"timeout_seconds,omitempty" nullable:"false" minimum:"1" maximum:"86400" format:"int64" doc:"Maximum execution time in seconds." default:"3600"`
	// Timezone describes explicit IANA time zone used for cron evaluation.
	Timezone string `json:"timezone,omitempty" nullable:"false" minLength:"1" doc:"Explicit IANA time zone used for cron evaluation." default:"UTC"`
}

// ScheduledTaskUpdate describes scheduled task update payload.
type ScheduledTaskUpdate struct {
	// Coalesce describes optional coalescing flag.
	Coalesce *bool `json:"coalesce,omitempty" nullable:"true" doc:"Optional coalescing flag."`
	// Cron describes optional replacement cron expression.
	Cron *string `json:"cron,omitempty" nullable:"true" minLength:"1" doc:"Optional replacement cron expression."`
	// Enabled describes optional enabled flag.
	Enabled *bool `json:"enabled,omitempty" nullable:"true" doc:"Optional enabled flag."`
	// Job describes optional replacement job specification.
	Job *scheduled.Job `json:"job,omitempty" nullable:"false" doc:"Optional replacement job specification."`
	// Name describes optional replacement display name.
	Name *string `json:"name,omitempty" nullable:"true" minLength:"1" doc:"Optional replacement display name."`
	// TimeoutSeconds describes optional replacement timeout in seconds.
	TimeoutSeconds *int64 `json:"timeout_seconds,omitempty" nullable:"true" minimum:"1" maximum:"86400" format:"int64" doc:"Optional replacement timeout in seconds."`
	// Timezone describes optional replacement IANA time zone.
	Timezone *string `json:"timezone,omitempty" nullable:"true" minLength:"1" doc:"Optional replacement IANA time zone."`
}

// TokenCreateRequest describes access token creation request.
type TokenCreateRequest struct {
	// Name describes token display name.
	Name string `json:"name,omitempty" nullable:"false" maxLength:"100" doc:"Token display name."`
	// TTL describes requested token TTL in seconds.
	TTL int64 `json:"ttl,omitempty" default:"604800" nullable:"false" minimum:"3600" maximum:"31536000" format:"int64" doc:"Requested token TTL in seconds."`
}

// TrackingSetRequest describes set-tracking-folder request payload.
type TrackingSetRequest struct {
	// FolderID describes folder row identifier to mark as tracking.
	FolderID int64 `json:"folder_id" nullable:"false" minimum:"1" format:"int64" doc:"Folder row identifier to mark as tracking."`
}

// EmailUpdateRequest describes EmailUpdateRequest JSON fields.
type EmailUpdateRequest struct {
	// Email describes normalized unique account email; null when unset.
	Email *string `json:"email" nullable:"true" doc:"Normalized unique account email; null when unset."`
}

// NotificationSettingsUpdate describes notification settings update payload.
type NotificationSettingsUpdate struct {
	// AIAPIKey describes primary AI API key update: omitted preserves, null clears, and non-empty replaces.
	AIAPIKey delivery.SecretUpdate `json:"ai_api_key" required:"false" nullable:"true" doc:"Primary AI API key update: omitted preserves, null clears, and non-empty replaces."`
	// AIBackupAPIKey describes backup AI API key update: omitted preserves, null clears, and non-empty replaces.
	AIBackupAPIKey delivery.SecretUpdate `json:"ai_backup_api_key" required:"false" nullable:"true" doc:"Backup AI API key update: omitted preserves, null clears, and non-empty replaces."`
	// AIBackupBaseURL describes backup AI endpoint base URL.
	AIBackupBaseURL string `json:"ai_backup_base_url,omitempty" nullable:"false" maxLength:"2048" doc:"Backup AI endpoint base URL."`
	// AIBackupModel describes backup AI model.
	AIBackupModel string `json:"ai_backup_model,omitempty" nullable:"false" maxLength:"200" doc:"Backup AI model."`
	// AIBackupSystemPrompt describes backup AI system prompt.
	AIBackupSystemPrompt string `json:"ai_backup_system_prompt,omitempty" nullable:"false" maxLength:"10000" doc:"Backup AI system prompt."`
	// AIBaseURL describes primary AI endpoint base URL.
	AIBaseURL string `json:"ai_base_url,omitempty" nullable:"false" maxLength:"2048" doc:"Primary AI endpoint base URL."`
	// AIModel describes primary AI model.
	AIModel string `json:"ai_model,omitempty" nullable:"false" maxLength:"200" doc:"Primary AI model."`
	// AIRetryAttempts describes retry attempts per AI endpoint.
	AIRetryAttempts *int64 `json:"ai_retry_attempts,omitempty" nullable:"false" minimum:"1" maximum:"10" format:"int64" doc:"Retry attempts per AI endpoint." default:"3"`
	// AISystemPrompt describes primary AI system prompt.
	AISystemPrompt string `json:"ai_system_prompt,omitempty" nullable:"false" maxLength:"10000" doc:"Primary AI system prompt."`
	// DeliveryMethod describes delivery method.
	DeliveryMethod string `json:"delivery_method,omitempty" nullable:"false" doc:"Delivery method." default:"folder"`
	// Directions describes research direction preferences.
	Directions []string `json:"directions,omitempty" nullable:"false" maxItems:"100" doc:"Research direction preferences."`
	// Enabled reports whether recommendations are enabled.
	Enabled *bool `json:"enabled,omitempty" nullable:"false" doc:"Whether recommendations are enabled." default:"true"`
	// Keywords describes keyword preferences.
	Keywords []string `json:"keywords,omitempty" nullable:"false" maxItems:"100" doc:"Keyword preferences."`
	// PushplusChannel describes pushPlus channel.
	PushplusChannel string `json:"pushplus_channel,omitempty" nullable:"false" maxLength:"64" doc:"PushPlus channel." default:"wechat"`
	// PushplusTemplate describes pushPlus template.
	PushplusTemplate string `json:"pushplus_template,omitempty" nullable:"false" maxLength:"64" doc:"PushPlus template." default:"markdown"`
	// PushplusToken describes pushPlus token update: omitted preserves, null clears, and non-empty replaces.
	PushplusToken delivery.SecretUpdate `json:"pushplus_token" required:"false" nullable:"true" doc:"PushPlus token update: omitted preserves, null clears, and non-empty replaces."`
	// PushplusTopic describes pushPlus topic.
	PushplusTopic string `json:"pushplus_topic,omitempty" nullable:"false" maxLength:"200" doc:"PushPlus topic."`
	// SelectedDatabases describes selected index database names.
	SelectedDatabases []string `json:"selected_databases,omitempty" nullable:"false" maxItems:"500" doc:"Selected index database names."`
	// SyncToTrackingFolder reports whether PushPlus delivery also syncs to the tracking folder.
	SyncToTrackingFolder bool `json:"sync_to_tracking_folder,omitempty" nullable:"false" doc:"Whether PushPlus delivery also syncs to the tracking folder."`
}
