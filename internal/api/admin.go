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

	"github.com/QianFuv/LitRadar/internal/api/executor"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	scheduled "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/cryptography"
	"github.com/QianFuv/LitRadar/internal/storage/announcements"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type adminHandlers struct {
	auth      *authHandlers
	storage   config.Config
	scheduler *scheduler.Repository
	settings  *settings.Repository
}
type adminUserResponse struct {
	Id            int64   `json:"id"`
	Username      string  `json:"username"`
	IsAdmin       bool    `json:"is_admin"`
	CreatedAt     float64 `json:"created_at"`
	UpdatedAt     float64 `json:"updated_at"`
	FolderCount   int64   `json:"folder_count"`
	FavoriteCount int64   `json:"favorite_count"`
	NotifyEnabled bool    `json:"notify_enabled"`
}
type adminInviteResponse struct {
	Id            int64    `json:"id"`
	Code          string   `json:"code"`
	CreatedBy     *int64   `json:"created_by"`
	CreatedByName *string  `json:"created_by_name"`
	UsedBy        *int64   `json:"used_by"`
	UsedByName    *string  `json:"used_by_name"`
	UsedAt        *float64 `json:"used_at"`
	Status        string   `json:"status"`
	ExpiresAt     float64  `json:"expires_at"`
	RevokedAt     *float64 `json:"revoked_at"`
	MaxUses       int64    `json:"max_uses"`
	UseCount      int64    `json:"use_count"`
	CreatedAt     float64  `json:"created_at"`
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

func (handlers *adminHandlers) routes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/admin/users", Id: "list_users"},
		{Method: "PUT", Path: "/api/admin/users/{user_id}/admin", Id: "set_admin"},
		{Method: "POST", Path: "/api/admin/users/{user_id}/reset-password", Id: "reset_password"},
		{Method: "DELETE", Path: "/api/admin/users/{user_id}", Id: "delete_user"},
		{Method: "GET", Path: "/api/admin/invite-codes", Id: "list_invite_codes"},
		{Method: "POST", Path: "/api/admin/invite-codes", Id: "create_invite_code"},
		{Method: "DELETE", Path: "/api/admin/invite-codes/{code_id}", Id: "revoke_admin_invite_code"},
		{Method: "GET", Path: "/api/admin/stats", Id: "stats"},
		{Method: "GET", Path: "/api/admin/announcements", Id: "list_announcements"},
		{Method: "POST", Path: "/api/admin/announcements", Id: "create_announcement"},
		{Method: "PUT", Path: "/api/admin/announcements/{announcement_id}", Id: "update_announcement"},
		{Method: "DELETE", Path: "/api/admin/announcements/{announcement_id}", Id: "delete_announcement"},
		{Method: "GET", Path: "/api/admin/scheduled-tasks", Id: "list_scheduled_tasks"},
		{Method: "POST", Path: "/api/admin/scheduled-tasks", Id: "create_scheduled_task"},
		{Method: "PUT", Path: "/api/admin/scheduled-tasks/{task_id}", Id: "update_scheduled_task"},
		{Method: "DELETE", Path: "/api/admin/scheduled-tasks/{task_id}", Id: "delete_scheduled_task"},
		{Method: "GET", Path: "/api/admin/scheduler/status", Id: "scheduler_status"},
		{Method: "GET", Path: "/api/admin/runtime-settings", Id: "list_runtime_settings"},
		{Method: "PUT", Path: "/api/admin/runtime-settings", Id: "update_runtime_settings"},
		{Method: "GET", Path: "/api/admin/provider-catalog", Id: "get_provider_catalog"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handle(writer, request, operation.Id)
		}})
	}
	return routes
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

func (handlers *adminHandlers) handle(writer http.ResponseWriter, request *http.Request, name string) {
	target := int64(0)
	for _, key := range []string{"user_id", "code_id", "announcement_id", "task_id"} {
		if request.PathValue(key) != "" {
			value, failure := extractPathInteger(request, key)
			if failure != nil {
				failure.write(writer)
				return
			}
			target = value
		}
	}
	var body map[string]any
	var kind bodyType
	switch name {
	case "set_admin":
		kind = structBody("AdminSetAdmin", bodyFieldOf("is_admin", boolBody))
	case "reset_password":
		kind = structBody("AdminResetPassword", bodyFieldOf("new_password", stringBody))
	case "create_invite_code":
		kind = requestBodies["AdminInviteCodeCreate"]
	case "create_announcement":
		kind = structBody("AnnouncementCreate", bodyFieldOf("title", stringBody), bodyFieldOf("message", stringBody), defaultBodyField("priority", stringBody, "normal"), defaultBodyField("enabled", boolBody, true))
	case "update_announcement":
		kind = structBody("AnnouncementUpdate", bodyFieldOf("title", optionalBody(stringBody)), bodyFieldOf("message", optionalBody(stringBody)), bodyFieldOf("priority", optionalBody(stringBody)), bodyFieldOf("enabled", optionalBody(boolBody)))
	case "create_scheduled_task", "update_scheduled_task":
		kind = scheduledBody(name == "update_scheduled_task")
	case "update_runtime_settings":
		kind = runtimeSettingsBody()
	}
	if kind.kind != "" {
		decoded, failure := extractBody(request, kind, name == "create_invite_code")
		if failure != nil {
			failure.write(writer)
			return
		}
		if decoded != nil {
			body = decoded.(map[string]any)
		} else {
			body = map[string]any{}
		}
	}
	current, failure := handlers.auth.authenticator.requireAdmin(request)
	if failure != nil {
		failure.write(writer)
		return
	}
	actor := current.authorization.User.Id
	action := map[string]string{"set_admin": "user_admin_update", "reset_password": "user_password_reset", "delete_user": "user_delete", "create_invite_code": "invite_create", "revoke_admin_invite_code": "invite_revoke", "create_announcement": "announcement_create", "update_announcement": "announcement_update", "delete_announcement": "announcement_delete", "create_scheduled_task": "scheduled_task_create", "update_scheduled_task": "scheduled_task_update", "delete_scheduled_task": "scheduled_task_delete", "update_runtime_settings": "runtime_settings_update"}[name]
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestId: requestId(request), OccurredAt: currentTimestamp()}
	actorNumber := int64(actor)
	event.ActorId = &actorNumber
	if target > 0 {
		event.TargetId = &target
	}
	started := time.Now()
	isCompleted := false
	if action != "" {
		defer func() {
			name, level, outcome := "security.admin.rejected", slog.LevelWarn, "rejected"
			if isCompleted {
				name, level, outcome = "security.admin.completed", slog.LevelInfo, "completed"
			}
			slog.Log(request.Context(), level, name, "event", name, "component", "security", "action", action, "outcome", outcome, "actor_id", actorNumber, "target_id", target, "reason", map[bool]string{true: "", false: "operation_failed"}[isCompleted], "duration_ms", time.Since(started).Milliseconds())
		}()
	}
	reject := func(reason string, failure *apiError) {
		rejection := event
		rejection.Outcome, rejection.Reason = "rejected", reason
		if persistence := handlers.auth.persistAudit(request, rejection); persistence != nil {
			failure = persistence
		}
		failure.write(writer)
	}
	if name == "set_admin" && target == actorNumber && !body["is_admin"].(bool) {
		reject("self_revocation_forbidden", badRequest("Cannot revoke own admin status"))
		return
	}
	if name == "delete_user" && target == actorNumber {
		reject("self_delete_forbidden", badRequest("Cannot delete yourself"))
		return
	}
	if name == "reset_password" && !cryptography.ValidNewPassword(body["new_password"].(string)) {
		reject("password_policy_failed", mapAuthError(domain.ErrPasswordShort))
		return
	}
	if name == "create_announcement" || name == "update_announcement" {
		if failure := validateAnnouncementBody(body); failure != nil {
			reject("validation_failed", failure)
			return
		}
	}
	if name == "create_scheduled_task" || name == "update_scheduled_task" {
		if failure := validateScheduledBody(body); failure != nil {
			reject("validation_failed", failure)
			return
		}
	}
	if name == "update_runtime_settings" {
		if failure := validateRuntimeBody(body); failure != nil {
			reject("validation_failed", failure)
			return
		}
	}
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
		failure = mapExecutorError(err)
		if action != "" {
			reject("executor_failed", failure)
		} else {
			failure.write(writer)
		}
		return
	}
	if observed.err != nil {
		failure = mapAdminError(observed.err)
		if action != "" && !errors.Is(observed.err, domain.ErrAudit) {
			reason := adminRejectionReason(observed.err)
			if name == "reset_password" {
				reason = "operation_failed"
			}
			reject(reason, failure)
		} else {
			failure.write(writer)
		}
		return
	}
	if changed, ok := observed.payload.(bool); ok {
		if !changed {
			detail := "User not found"
			if name == "revoke_admin_invite_code" {
				detail = "Code not found or already revoked"
			}
			if name == "delete_announcement" {
				detail = "Announcement not found"
			}
			if name == "delete_scheduled_task" {
				detail = "Scheduled task not found"
			}
			reject("target_not_found", &apiError{status: 404, detail: detail})
			return
		}
		observed.payload = map[string]bool{"ok": true}
	}
	if invite, ok := observed.payload.(adminInviteResponse); ok {
		target = invite.Id
	}
	if announcement, ok := observed.payload.(*announcements.Announcement); ok {
		if announcement == nil {
			reject("target_not_found", &apiError{status: 404, detail: "Announcement not found"})
			return
		}
		target = announcement.Id
	}
	if announcement, ok := observed.payload.(announcements.Announcement); ok {
		target = announcement.Id
	}
	if task, ok := observed.payload.(*scheduled.Task); ok {
		if task == nil {
			reject("target_not_found", &apiError{status: 404, detail: "Scheduled task not found"})
			return
		}
		target = task.Id
	}
	isCompleted = true
	writeResponse(writer, observed.payload)
}

func (handlers *adminHandlers) perform(ctx context.Context, name string, actor identity.Id, target int64, body map[string]any, event domain.AuditEvent) (any, error) {
	if strings.Contains(name, "scheduled_task") || name == "scheduler_status" || strings.Contains(name, "runtime_settings") || name == "get_provider_catalog" {
		return handlers.performConfiguration(ctx, name, actor, target, body, event)
	}
	repository := handlers.auth.repository
	switch name {
	case "list_users":
		rows, err := repository.ListUsers(ctx)
		if err != nil {
			return nil, err
		}
		result := make([]adminUserResponse, len(rows))
		for index, row := range rows {
			result[index] = adminUserResponse{int64(row.Id), row.Username, row.IsAdmin, row.CreatedAt, row.UpdatedAt, row.FolderCount, row.FavoriteCount, row.NotifyEnabled}
		}
		return result, nil
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
	case "set_admin":
		err := repository.SetAdministrator(ctx, actor, identity.Id(target), body["is_admin"].(bool), &event)
		return true, err
	case "delete_user":
		err := repository.DeleteUser(ctx, actor, identity.Id(target), &event)
		return true, err
	case "reset_password":
		return handlers.auth.service.ResetPassword(ctx, &actor, identity.Id(target), body["new_password"].(string), &event)
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
	case "stats":
		return readAdminStats(ctx, repository, handlers.storage)
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
