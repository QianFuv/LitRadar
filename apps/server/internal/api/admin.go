package api

import (
	"errors"
	"log/slog"
	"net/http"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	scheduled "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/announcements"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
)

type adminHandlers struct {
	telemetry *telemetry.Collector
	auth      *authHandlers
	storage   config.Config
	scheduler *scheduler.Repository
	settings  *settings.Repository
}
type adminUserResponse struct {
	// Email contains the account email, or null when unset.
	Email *string `json:"email" nullable:"true"`
	// AuthKind identifies a local or OIDC account.
	AuthKind string `json:"auth_kind" nullable:"false" enum:"local,oidc"`
	// IsOIDCLinked reports whether the account has a linked OIDC identity.
	IsOIDCLinked  bool    `json:"is_oidc_linked" nullable:"false"`
	Id            int64   `json:"id" nullable:"false" format:"int64"`
	Username      string  `json:"username" nullable:"false"`
	IsAdmin       bool    `json:"is_admin" nullable:"false"`
	CreatedAt     float64 `json:"created_at" nullable:"false" format:"double"`
	UpdatedAt     float64 `json:"updated_at" nullable:"false" format:"double"`
	FolderCount   int64   `json:"folder_count" nullable:"false" format:"int64"`
	FavoriteCount int64   `json:"favorite_count" nullable:"false" format:"int64"`
	NotifyEnabled bool    `json:"notify_enabled" nullable:"false"`
}
type adminInviteResponse struct {
	Id            int64    `json:"id" nullable:"false" format:"int64"`
	Code          string   `json:"code" nullable:"false"`
	CreatedBy     *int64   `json:"created_by" required:"false" nullable:"true" format:"int64"`
	CreatedByName *string  `json:"created_by_name" required:"false" nullable:"true"`
	UsedBy        *int64   `json:"used_by" required:"false" nullable:"true" format:"int64"`
	UsedByName    *string  `json:"used_by_name" required:"false" nullable:"true"`
	UsedAt        *float64 `json:"used_at" required:"false" nullable:"true" format:"double"`
	Status        string   `json:"status" nullable:"false" enum:"active,expired,revoked,exhausted"`
	ExpiresAt     float64  `json:"expires_at" nullable:"false" format:"double"`
	RevokedAt     *float64 `json:"revoked_at" required:"false" nullable:"true" format:"double"`
	MaxUses       int64    `json:"max_uses" nullable:"false" format:"int64"`
	UseCount      int64    `json:"use_count" nullable:"false" format:"int64"`
	CreatedAt     float64  `json:"created_at" nullable:"false" format:"double"`
}

func adminInvite(value storage.AdminInviteInfo) adminInviteResponse {
	var created, used *int64
	if value.CreatedBy != nil {
		number := int64(*value.CreatedBy)
		created = &number
	}
	if value.UsedBy != nil {
		number := int64(*value.UsedBy)
		used = &number
	}
	return adminInviteResponse{value.Id, value.Code, created, value.CreatedByName, used, value.UsedByName, value.UsedAt, value.Status, value.ExpiresAt, value.RevokedAt, value.MaxUses, value.UseCount, value.CreatedAt}
}

func mapAdminError(err error) *apiError {
	var invalid announcements.InvalidInput
	var timing *scheduled.ValidationError
	var setting settings.Error
	switch {
	case errors.As(err, &timing), errors.As(err, &setting):
		return badRequest(err.Error())
	case errors.As(err, &invalid):
		return badRequest(err.Error())
	case errors.Is(err, storage.ErrUserNotFound):
		return &apiError{status: 404, detail: err.Error()}
	case errors.Is(err, storage.ErrLastAdministrator):
		return &apiError{status: 409, detail: err.Error()}
	case errors.Is(err, storage.ErrAdminInvitePolicy):
		return badRequest(err.Error())
	default:
		return mapAuthError(err)
	}
}
func adminRejectionReason(err error) string {
	var invalid announcements.InvalidInput
	var timing *scheduled.ValidationError
	var setting settings.Error
	switch {
	case errors.As(err, &timing), errors.As(err, &setting):
		return "validation_failed"
	case errors.As(err, &invalid):
		return "validation_failed"
	case errors.Is(err, domain.ErrAdminForbidden):
		return "actor_forbidden"
	case errors.Is(err, storage.ErrUserNotFound):
		return "target_not_found"
	case errors.Is(err, storage.ErrLastAdministrator):
		return "administrator_invariant"
	case errors.Is(err, storage.ErrAdminInvitePolicy):
		return "validation_failed"
	default:
		return "operation_failed"
	}
}

// logAdminCompletion observes the final target and completion state at handler return.
func logAdminCompletion(request *http.Request, action string, actor int64, target *int64, isCompleted *bool, started time.Time) {
	if action == "" {
		return
	}
	name, level, outcome := "security.admin.rejected", slog.LevelWarn, "rejected"
	if *isCompleted {
		name, level, outcome = "security.admin.completed", slog.LevelInfo, "completed"
	}
	slog.Log(request.Context(), level, name, "event", name, "component", "security", "action", action, "outcome", outcome, "actor_id", actor, "target_id", *target, "reason", map[bool]string{true: "", false: "operation_failed"}[*isCompleted], "duration_ms", time.Since(started).Milliseconds())
}
