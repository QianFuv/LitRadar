package api

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"strings"
	"time"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	scheduled "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cryptography"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
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

// perform routes configuration before administrator operation families.

func (handlers *adminHandlers) perform(ctx context.Context, name string, actor identity.Id, target int64, body map[string]any, event domain.AuditEvent) (any, error) {
	if strings.Contains(name, "scheduled_task") || name == "scheduler_status" || strings.Contains(name, "runtime_settings") || name == "get_provider_catalog" {
		return handlers.performConfiguration(ctx, name, actor, target, body, event)
	}

	switch name {
	case "list_users", "set_admin", "delete_user", "reset_password":
		return handlers.performAdminUsers(ctx, name, actor, target, body, event)
	case "list_invite_codes", "create_invite_code", "revoke_admin_invite_code":
		return handlers.performAdminInvites(ctx, name, actor, target, body, event)
	case "stats":
		return readAdminStats(ctx, handlers.auth.repository, handlers.storage)
	default:
		return handlers.performAdminAnnouncements(ctx, name, actor, target, body, event)
	}
}

// validateAdminSelf checks protected self mutations before other business validation.
func validateAdminSelf(name string, target, actorNumber int64, body map[string]any) (string, *apiError) {
	if name == "set_admin" && target == actorNumber && !body["is_admin"].(bool) {
		return "self_revocation_forbidden", badRequest("Cannot revoke own admin status")
	}
	if name == "delete_user" && target == actorNumber {
		return "self_delete_forbidden", badRequest("Cannot delete yourself")
	}
	if name == "reset_password" && !cryptography.ValidNewPassword(body["new_password"].(string)) {
		return "password_policy_failed", mapAuthError(domain.ErrPasswordShort)
	}

	return "", nil
}

// validateAdminOperationBody preserves operation-specific normalization before executor admission.
func validateAdminOperationBody(name string, body map[string]any) *apiError {
	switch name {
	case "create_announcement", "update_announcement":
		return validateAnnouncementBody(body)
	case "create_scheduled_task", "update_scheduled_task":
		return validateScheduledBody(body)
	case "update_runtime_settings":
		return validateRuntimeBody(body)
	}
	return nil
}

// executeAdminOperation retains KDF admission and avoids repeating an already failed audit write.
func (handlers *adminHandlers) executeAdminOperation(request *http.Request, name string, actor identity.Id, target int64, body map[string]any, event domain.AuditEvent) (any, string, *apiError) {
	pool := handlers.auth.pool
	if name == "reset_password" {
		pool = handlers.auth.kdfPool
	}
	type result struct {
		payload any
		err     error
	}
	observed, err := executor.Run(request.Context(), pool, func() (result, error) {
		value, err := handlers.perform(context.WithoutCancel(request.Context()), name, actor, target, body, event)
		return result{value, err}, nil
	})

	if err != nil {
		if event.Action != "" {
			return nil, "executor_failed", mapExecutorError(err)
		}
		return nil, "", mapExecutorError(err)
	}
	if observed.err != nil {
		failure := mapAdminError(observed.err)
		if event.Action != "" && !errors.Is(observed.err, domain.ErrAudit) {
			reason := adminRejectionReason(observed.err)
			if name == "reset_password" {
				reason = "operation_failed"
			}
			return nil, reason, failure
		}
		return nil, "", failure
	}
	return observed.payload, "", nil
}

// normalizeAdminPayload retains missing-target details and returned audit target identity.
func normalizeAdminPayload(name string, payload any, target int64) (any, int64, *apiError) {
	switch value := payload.(type) {
	case bool:
		if !value {
			return payload, target, &apiError{status: 404, detail: adminMissingTargetDetail(name)}
		}
		payload = okResponse{OK: true}
	case adminInviteResponse:
		target = value.Id
	case *announcements.Announcement:
		if value == nil {
			return payload, target, &apiError{status: 404, detail: "Announcement not found"}
		}
		target = value.Id
	case announcements.Announcement:
		target = value.Id
	case *scheduled.Task:
		if value == nil {
			return payload, target, &apiError{status: 404, detail: "Scheduled task not found"}
		}
		target = value.Id
	}
	return payload, target, nil
}

// adminMissingTargetDetail distinguishes user, invite, announcement and task mutations.
func adminMissingTargetDetail(name string) string {
	switch name {
	case "revoke_admin_invite_code":
		return "Code not found or already revoked"
	case "delete_announcement":
		return "Announcement not found"
	case "delete_scheduled_task":
		return "Scheduled task not found"
	default:
		return "User not found"
	}
}

// performAdminUsers keeps user mutations and their original audit arguments.
func (handlers *adminHandlers) performAdminUsers(ctx context.Context, name string, actor identity.Id, target int64, body map[string]any, event domain.AuditEvent) (any, error) {
	repository := handlers.auth.repository
	switch name {
	case "list_users":
		rows, err := repository.ListUsers(ctx)
		if err != nil {
			return nil, err
		}
		result := make([]adminUserResponse, len(rows))
		for index, row := range rows {
			result[index] = adminUserResponse{Id: int64(row.Id), Username: row.Username, IsAdmin: row.IsAdmin, CreatedAt: row.CreatedAt, UpdatedAt: row.UpdatedAt, FolderCount: row.FolderCount, FavoriteCount: row.FavoriteCount, NotifyEnabled: row.NotifyEnabled}
		}
		return result, nil
	case "set_admin":
		err := repository.SetAdministrator(ctx, actor, identity.Id(target), body["is_admin"].(bool), &event)
		return true, err
	case "delete_user":
		err := repository.DeleteUser(ctx, actor, identity.Id(target), &event)
		return true, err
	case "reset_password":
		return handlers.auth.service.ResetPassword(ctx, &actor, identity.Id(target), body["new_password"].(string), &event)

	}
	return nil, errors.New("unknown administrator operation")
}

// performAdminInvites preserves optional expiry and usage projections.
func (handlers *adminHandlers) performAdminInvites(ctx context.Context, name string, actor identity.Id, target int64, body map[string]any, event domain.AuditEvent) (any, error) {
	repository := handlers.auth.repository
	switch name {
	case "list_invite_codes":
		rows, err := repository.ListInvites(ctx, currentTimestamp())
		if err != nil {
			return nil, err
		}
		result := make([]adminInviteResponse, len(rows))
		for index, row := range rows {
			result[index] = adminInvite(row)
		}
		return result, nil
	case "create_invite_code":
		var expires *float64
		var maxUses *int64
		if value, ok := body["expires_at"].(float64); ok {
			expires = &value
		}
		if value, ok := body["max_uses"].(int64); ok {
			maxUses = &value
		}
		invite, err := repository.CreateAdministratorInvite(ctx, &actor, expires, maxUses, &event)
		return adminInvite(invite), err
	case "revoke_admin_invite_code":
		return repository.RevokeAdministratorInvite(ctx, &actor, target, &event)

	}
	return nil, errors.New("unknown administrator operation")
}

// performAdminAnnouncements preserves omitted update fields and administrator audit arguments.
func (handlers *adminHandlers) performAdminAnnouncements(ctx context.Context, name string, actor identity.Id, target int64, body map[string]any, event domain.AuditEvent) (any, error) {
	repository := handlers.auth.repository
	switch name {
	case "list_announcements":
		return announcements.ListAll(ctx, repository)
	case "create_announcement":
		return announcements.Create(ctx, repository, &actor, body["title"].(string), body["message"].(string), body["priority"].(string), body["enabled"].(bool), &event)
	case "update_announcement":
		input := announcements.Update{}
		if value, ok := body["title"].(string); ok {
			input.Title = &value
		}
		if value, ok := body["message"].(string); ok {
			input.Message = &value
		}
		if value, ok := body["priority"].(string); ok {
			input.Priority = &value
		}
		if value, ok := body["enabled"].(bool); ok {
			input.Enabled = &value
		}
		return announcements.Modify(ctx, repository, &actor, target, input, &event)
	case "delete_announcement":
		return announcements.Delete(ctx, repository, &actor, target, &event)

	}
	return nil, errors.New("unknown administrator operation")
}

func validateAnnouncementBody(body map[string]any) *apiError {
	for _, field := range []struct {
		name, label string
		maximum     int
	}{{"title", "Title", 200}, {"message", "Message", 10000}, {"priority", "Priority", 16}} {
		value, ok := body[field.name].(string)
		if !ok {
			continue
		}
		value = strings.TrimSpace(value)
		if field.name == "priority" {
			value = asciiLower(value)
		}
		if utf8.RuneCountInString(value) > field.maximum {
			return badRequest(fmt.Sprintf("%s must be at most %d characters", field.label, field.maximum))
		}
		if value == "" {
			return badRequest(fmt.Sprintf("%s must be 1-%d characters", field.label, field.maximum))
		}
		if field.name == "priority" && value != "high" && value != "normal" && value != "low" {
			return badRequest("Priority must be high, normal, or low")
		}
		body[field.name] = value
	}
	return nil
}
