package api

import (
	"context"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	scheduled "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/announcements"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"net/http"
	"time"
)

type providerCatalogResponse struct {
	// Providers describes logical Providers and their aggregate capabilities.
	Providers []sources.CapabilityInfo `json:"providers" nullable:"false"`
	// Catalogs describes discovered metadata and content catalogs.
	Catalogs []storage.ProviderCatalog `json:"catalogs" nullable:"false"`
}

func adminOperation[Body any](handlers *adminHandlers, request *http.Request, name string, target int64, body map[string]any) (*output[Body], error) {
	current, failure := handlers.auth.authenticator.requireAdmin(request)
	if failure != nil {
		return nil, failure.problem()
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
	defer logAdminCompletion(request, action, actorNumber, &target, &isCompleted, started)
	reject := func(reason string, failure *apiError) error {
		rejection := event
		rejection.Outcome, rejection.Reason = "rejected", reason
		if persistence := handlers.auth.persistAudit(request, rejection); persistence != nil {
			failure = persistence
		}
		return failure.problem()
	}

	reason, failure := validateAdminSelf(name, target, actorNumber, body)
	if failure != nil {
		return nil, reject(reason, failure)
	}
	if failure = validateAdminOperationBody(name, body); failure != nil {
		return nil, reject("validation_failed", failure)
	}
	payload, reason, failure := handlers.executeAdminOperation(request, name, actor, target, body, event)
	if failure != nil {
		if reason != "" {
			return nil, reject(reason, failure)
		} else {
			return nil, failure.problem()
		}
	}
	payload, target, failure = normalizeAdminPayload(name, payload, target)
	if failure != nil {
		return nil, reject("target_not_found", failure)
	}
	isCompleted = true
	value, ok := payload.(Body)
	if !ok {
		return nil, internalError().problem()
	}
	return &output[Body]{Body: value}, nil
}

// logAdminCompletion observes the final target and completion state at handler return.
func (handlers *adminHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/telemetry", ID: "telemetry"}, handlers.loadTelemetry),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/users", ID: "list_users"}, func(ctx context.Context, input *ListUsersInput) (*output[[]adminUserResponse], error) {
			body := map[string]any{}
			return adminOperation[[]adminUserResponse](handlers, input.request, "list_users", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/users/{user_id}/admin", ID: "set_admin"}, func(ctx context.Context, input *SetAdminInput) (*output[okResponse], error) {
			body := map[string]any{}
			body["is_admin"] = input.Body.IsAdmin
			return adminOperation[okResponse](handlers, input.request, "set_admin", input.UserID, body)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/users/{user_id}/reset-password", ID: "reset_password"}, func(ctx context.Context, input *ResetPasswordInput) (*output[okResponse], error) {
			body := map[string]any{}
			body["new_password"] = input.Body.NewPassword
			return adminOperation[okResponse](handlers, input.request, "reset_password", input.UserID, body)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/users/{user_id}", ID: "delete_user"}, func(ctx context.Context, input *DeleteUserInput) (*output[okResponse], error) {
			body := map[string]any{}
			return adminOperation[okResponse](handlers, input.request, "delete_user", input.UserID, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/invite-codes", ID: "list_invite_codes"}, func(ctx context.Context, input *ListInviteCodesInput) (*output[[]adminInviteResponse], error) {
			body := map[string]any{}
			return adminOperation[[]adminInviteResponse](handlers, input.request, "list_invite_codes", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/invite-codes", ID: "create_invite_code"}, func(ctx context.Context, input *CreateInviteCodeInput) (*output[adminInviteResponse], error) {
			body := map[string]any{}
			if input.Body != nil {
				if input.Body.ExpiresAt != nil {
					body["expires_at"] = *input.Body.ExpiresAt
				}
				if input.Body.MaxUses != nil {
					body["max_uses"] = *input.Body.MaxUses
				}
			}
			return adminOperation[adminInviteResponse](handlers, input.request, "create_invite_code", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/invite-codes/{code_id}", ID: "revoke_admin_invite_code"}, func(ctx context.Context, input *RevokeAdminInviteCodeInput) (*output[okResponse], error) {
			body := map[string]any{}
			return adminOperation[okResponse](handlers, input.request, "revoke_admin_invite_code", input.CodeID, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/stats", ID: "stats"}, func(ctx context.Context, input *StatsInput) (*output[adminStats], error) {
			body := map[string]any{}
			return adminOperation[adminStats](handlers, input.request, "stats", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/announcements", ID: "list_announcements"}, func(ctx context.Context, input *ListAnnouncementsInput) (*output[[]announcements.Announcement], error) {
			body := map[string]any{}
			return adminOperation[[]announcements.Announcement](handlers, input.request, "list_announcements", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/announcements", ID: "create_announcement"}, func(ctx context.Context, input *CreateAnnouncementInput) (*output[announcements.Announcement], error) {
			body := map[string]any{}
			body["enabled"] = *input.Body.Enabled
			body["message"] = input.Body.Message
			body["priority"] = input.Body.Priority
			body["title"] = input.Body.Title
			return adminOperation[announcements.Announcement](handlers, input.request, "create_announcement", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/announcements/{announcement_id}", ID: "update_announcement"}, func(ctx context.Context, input *UpdateAnnouncementInput) (*output[*announcements.Announcement], error) {
			body := map[string]any{}
			if input.Body.Enabled != nil {
				body["enabled"] = *input.Body.Enabled
			}
			if input.Body.Message != nil {
				body["message"] = *input.Body.Message
			}
			if input.Body.Priority != nil {
				body["priority"] = *input.Body.Priority
			}
			if input.Body.Title != nil {
				body["title"] = *input.Body.Title
			}
			return adminOperation[*announcements.Announcement](handlers, input.request, "update_announcement", input.AnnouncementID, body)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/announcements/{announcement_id}", ID: "delete_announcement"}, func(ctx context.Context, input *DeleteAnnouncementInput) (*output[okResponse], error) {
			body := map[string]any{}
			return adminOperation[okResponse](handlers, input.request, "delete_announcement", input.AnnouncementID, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/scheduled-tasks", ID: "list_scheduled_tasks"}, func(ctx context.Context, input *ListScheduledTasksInput) (*output[[]scheduled.Task], error) {
			body := map[string]any{}
			return adminOperation[[]scheduled.Task](handlers, input.request, "list_scheduled_tasks", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/scheduled-tasks", ID: "create_scheduled_task"}, func(ctx context.Context, input *CreateScheduledTaskInput) (*output[*scheduled.Task], error) {
			body := map[string]any{}
			body["coalesce"] = *input.Body.Coalesce
			body["cron"] = input.Body.Cron
			body["enabled"] = *input.Body.Enabled
			body["job"] = input.Body.Job
			body["name"] = input.Body.Name
			body["timeout_seconds"] = uint64(*input.Body.TimeoutSeconds)
			body["timezone"] = input.Body.Timezone
			return adminOperation[*scheduled.Task](handlers, input.request, "create_scheduled_task", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/scheduled-tasks/{task_id}", ID: "update_scheduled_task"}, func(ctx context.Context, input *UpdateScheduledTaskInput) (*output[*scheduled.Task], error) {
			body := map[string]any{}
			if input.Body.Coalesce != nil {
				body["coalesce"] = *input.Body.Coalesce
			}
			if input.Body.Cron != nil {
				body["cron"] = *input.Body.Cron
			}
			if input.Body.Enabled != nil {
				body["enabled"] = *input.Body.Enabled
			}
			if input.Body.Job != nil {
				body["job"] = *input.Body.Job
			}
			if input.Body.Name != nil {
				body["name"] = *input.Body.Name
			}
			if input.Body.TimeoutSeconds != nil {
				body["timeout_seconds"] = uint64(*input.Body.TimeoutSeconds)
			}
			if input.Body.Timezone != nil {
				body["timezone"] = *input.Body.Timezone
			}
			return adminOperation[*scheduled.Task](handlers, input.request, "update_scheduled_task", input.TaskID, body)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/scheduled-tasks/{task_id}", ID: "delete_scheduled_task"}, func(ctx context.Context, input *DeleteScheduledTaskInput) (*output[okResponse], error) {
			body := map[string]any{}
			return adminOperation[okResponse](handlers, input.request, "delete_scheduled_task", input.TaskID, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/scheduler/status", ID: "scheduler_status"}, func(ctx context.Context, input *SchedulerStatusInput) (*output[scheduled.Status], error) {
			body := map[string]any{}
			return adminOperation[scheduled.Status](handlers, input.request, "scheduler_status", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/runtime-settings", ID: "list_runtime_settings"}, func(ctx context.Context, input *ListRuntimeSettingsInput) (*output[[]settings.Info], error) {
			body := map[string]any{}
			return adminOperation[[]settings.Info](handlers, input.request, "list_runtime_settings", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/runtime-settings", ID: "update_runtime_settings"}, func(ctx context.Context, input *UpdateRuntimeSettingsInput) (*output[[]settings.Info], error) {
			body := map[string]any{}
			body["secret_pool_updates"] = input.Body.SecretPoolUpdates
			body["values"] = input.Body.Values
			return adminOperation[[]settings.Info](handlers, input.request, "update_runtime_settings", 0, body)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/provider-catalog", ID: "get_provider_catalog"}, func(ctx context.Context, input *GetProviderCatalogInput) (*output[providerCatalogResponse], error) {
			body := map[string]any{}
			return adminOperation[providerCatalogResponse](handlers, input.request, "get_provider_catalog", 0, body)
		}),
	}
}
