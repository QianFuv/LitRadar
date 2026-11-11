package api

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	scheduled "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cron"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cryptography"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/announcements"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
)

type providerCatalogResponse struct {
	// Providers describes logical Providers and their aggregate capabilities.
	Providers []sources.CapabilityInfo `json:"providers" nullable:"false"`
	// Catalogs describes discovered metadata and content catalogs.
	Catalogs []storage.ProviderCatalog `json:"catalogs" nullable:"false"`
}

// adminOperation retains typed results across authorization, audit and bounded execution.
func adminOperation[Body any](handlers *adminHandlers, request *http.Request, action string, target int64, validate func(identity.ID) (string, *apiError), work func(context.Context, identity.ID, domain.AuditEvent) (Body, int64, error)) (*output[Body], error) {
	current, failure := handlers.auth.authenticator.requireAdmin(request)
	if failure != nil {
		return nil, failure.problem()
	}
	actor := current.authorization.User.ID
	actorNumber := int64(actor)
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestID: requestID(request), OccurredAt: currentTimestamp(), ActorID: &actorNumber}
	if target > 0 {
		event.TargetID = &target
	}
	started := time.Now()
	isCompleted := false
	defer logAdminCompletion(request, action, actorNumber, &target, &isCompleted, started)
	reject := func(reason string, failure *apiError) error {
		if action == "" {
			return failure.problem()
		}
		rejection := event
		rejection.Outcome, rejection.Reason = "rejected", reason
		if persistence := handlers.auth.persistAudit(request, rejection); persistence != nil {
			failure = persistence
		}
		return failure.problem()
	}
	if validate != nil {
		if reason, failure := validate(actor); failure != nil {
			return nil, reject(reason, failure)
		}
	}
	pool := handlers.auth.pool
	if action == "user_password_reset" {
		pool = handlers.auth.kdfPool
	}
	type result struct {
		body   Body
		target int64
		err    error
	}
	observed, err := executor.Run(request.Context(), pool, func() (result, error) {
		body, returnedTarget, failure := work(context.WithoutCancel(request.Context()), actor, event)
		return result{body, returnedTarget, failure}, nil
	})
	if err != nil {
		return nil, reject("executor_failed", mapExecutorError(err))
	}
	if observed.err != nil {
		failure := mapAdminError(observed.err)
		var problem *apiError
		if errors.As(observed.err, &problem) {
			failure = problem
		}
		if errors.Is(observed.err, domain.ErrAudit) {
			return nil, failure.problem()
		}
		reason := adminRejectionReason(observed.err)
		if failure.status == http.StatusNotFound {
			reason = "target_not_found"
		}
		if action == "user_password_reset" && problem == nil {
			reason = "operation_failed"
		}
		return nil, reject(reason, failure)
	}
	target = observed.target
	isCompleted = true
	return &output[Body]{Body: observed.body}, nil
}

// routes binds administrator operations directly to their typed handlers.
func (handlers *adminHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/telemetry", ID: "telemetry"}, handlers.loadTelemetry),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/users", ID: "list_users"}, handlers.listUsers),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/users/{user_id}/admin", ID: "set_admin"}, handlers.setAdministrator),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/users/{user_id}/reset-password", ID: "reset_password"}, handlers.resetUserPassword),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/users/{user_id}", ID: "delete_user"}, handlers.deleteUser),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/invite-codes", ID: "list_invite_codes"}, handlers.listInviteCodes),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/invite-codes", ID: "create_invite_code"}, handlers.createInviteCode),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/invite-codes/{code_id}", ID: "revoke_admin_invite_code"}, handlers.revokeInviteCode),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/stats", ID: "stats"}, handlers.stats),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/announcements", ID: "list_announcements"}, handlers.listAnnouncements),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/announcements", ID: "create_announcement"}, handlers.createAnnouncement),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/announcements/{announcement_id}", ID: "update_announcement"}, handlers.updateAnnouncement),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/announcements/{announcement_id}", ID: "delete_announcement"}, handlers.deleteAnnouncement),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/scheduled-tasks", ID: "list_scheduled_tasks"}, handlers.listScheduledTasks),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/admin/scheduled-tasks", ID: "create_scheduled_task"}, handlers.createScheduledTask),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/scheduled-tasks/{task_id}", ID: "update_scheduled_task"}, handlers.updateScheduledTask),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/admin/scheduled-tasks/{task_id}", ID: "delete_scheduled_task"}, handlers.deleteScheduledTask),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/scheduler/status", ID: "scheduler_status"}, handlers.schedulerStatus),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/runtime-settings", ID: "list_runtime_settings"}, handlers.listRuntimeSettings),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/admin/runtime-settings", ID: "update_runtime_settings"}, handlers.updateRuntimeSettings),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/admin/provider-catalog", ID: "get_provider_catalog"}, handlers.providerCatalog),
	}
}

// listUsers handles list_users with typed input and output.
func (handlers *adminHandlers) listUsers(ctx context.Context, input *ListUsersInput) (*output[[]adminUserResponse], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) ([]adminUserResponse, int64, error) {
		rows, err := handlers.auth.repository.ListUsers(ctx)
		if err != nil {
			return nil, 0, err
		}
		result := make([]adminUserResponse, len(rows))
		for index, row := range rows {
			result[index] = adminUserResponse{Email: row.Email, AuthKind: row.AuthKind, IsOIDCLinked: row.IsOidcLinked, ID: int64(row.ID), Username: row.Username, IsAdmin: row.IsAdmin, CreatedAt: row.CreatedAt, UpdatedAt: row.UpdatedAt, FolderCount: row.FolderCount, FavoriteCount: row.FavoriteCount, NotifyEnabled: row.NotifyEnabled}
		}
		return result, 0, nil
	})
}

// setAdministrator handles set_admin with typed input and output.
func (handlers *adminHandlers) setAdministrator(ctx context.Context, input *SetAdminInput) (*output[okResponse], error) {
	return adminOperation(handlers, input.request, "user_admin_update", input.UserID, func(actor identity.ID) (string, *apiError) {
		if input.UserID == int64(actor) && !input.Body.IsAdmin {
			return "self_revocation_forbidden", badRequest("Cannot revoke own admin status")
		}
		return "", nil
	}, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (okResponse, int64, error) {
		err := handlers.auth.repository.SetAdministrator(ctx, actor, identity.ID(input.UserID), input.Body.IsAdmin, &event)
		return okResponse{OK: true}, input.UserID, err
	})
}

// resetUserPassword handles reset_password with typed input and output.
func (handlers *adminHandlers) resetUserPassword(ctx context.Context, input *ResetPasswordInput) (*output[okResponse], error) {
	return adminOperation(handlers, input.request, "user_password_reset", input.UserID, func(identity.ID) (string, *apiError) {
		if !cryptography.ValidNewPassword(input.Body.NewPassword) {
			return "password_policy_failed", mapAuthError(domain.ErrPasswordShort)
		}
		return "", nil
	}, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (okResponse, int64, error) {
		changed, err := handlers.auth.service.ResetPassword(ctx, &actor, identity.ID(input.UserID), input.Body.NewPassword, &event)
		response, failure := changedResponse(changed, err, "User not found")
		return response, input.UserID, failure
	})
}

// deleteUser handles delete_user with typed input and output.
func (handlers *adminHandlers) deleteUser(ctx context.Context, input *DeleteUserInput) (*output[okResponse], error) {
	return adminOperation(handlers, input.request, "user_delete", input.UserID, func(actor identity.ID) (string, *apiError) {
		if input.UserID == int64(actor) {
			return "self_delete_forbidden", badRequest("Cannot delete yourself")
		}
		return "", nil
	}, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (okResponse, int64, error) {
		err := handlers.auth.repository.DeleteUser(ctx, actor, identity.ID(input.UserID), &event)
		return okResponse{OK: true}, input.UserID, err
	})
}

// listInviteCodes handles list_invite_codes with typed input and output.
func (handlers *adminHandlers) listInviteCodes(ctx context.Context, input *ListInviteCodesInput) (*output[[]adminInviteResponse], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) ([]adminInviteResponse, int64, error) {
		rows, err := handlers.auth.repository.ListInvites(ctx, currentTimestamp())
		if err != nil {
			return nil, 0, err
		}
		result := make([]adminInviteResponse, len(rows))
		for index, row := range rows {
			result[index] = adminInvite(row)
		}
		return result, 0, nil
	})
}

// createInviteCode handles create_invite_code with typed input and output.
func (handlers *adminHandlers) createInviteCode(ctx context.Context, input *CreateInviteCodeInput) (*output[adminInviteResponse], error) {
	return adminOperation(handlers, input.request, "invite_create", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (adminInviteResponse, int64, error) {
		body := input.Body
		if body == nil {
			body = &AdminInviteCodeCreate{}
		}
		invite, err := handlers.auth.repository.CreateAdministratorInvite(ctx, &actor, body.ExpiresAt, body.MaxUses, &event)
		return adminInvite(invite), invite.ID, err
	})
}

// revokeInviteCode handles revoke_admin_invite_code with typed input and output.
func (handlers *adminHandlers) revokeInviteCode(ctx context.Context, input *RevokeAdminInviteCodeInput) (*output[okResponse], error) {
	return adminOperation(handlers, input.request, "invite_revoke", input.CodeID, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (okResponse, int64, error) {
		changed, err := handlers.auth.repository.RevokeAdministratorInvite(ctx, &actor, input.CodeID, &event)
		response, failure := changedResponse(changed, err, "Code not found or already revoked")
		return response, input.CodeID, failure
	})
}

// stats handles stats with typed input and output.
func (handlers *adminHandlers) stats(ctx context.Context, input *StatsInput) (*output[adminStats], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (adminStats, int64, error) {
		result, err := readAdminStats(ctx, handlers.auth.repository, handlers.storage)
		return result, 0, err
	})
}

// listAnnouncements handles list_announcements with typed input and output.
func (handlers *adminHandlers) listAnnouncements(ctx context.Context, input *ListAnnouncementsInput) (*output[[]announcements.Announcement], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) ([]announcements.Announcement, int64, error) {
		rows, err := announcements.ListAll(ctx, handlers.auth.repository)
		return rows, 0, err
	})
}

// createAnnouncement handles create_announcement with typed input and output.
func (handlers *adminHandlers) createAnnouncement(ctx context.Context, input *CreateAnnouncementInput) (*output[announcements.Announcement], error) {
	return adminOperation(handlers, input.request, "announcement_create", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (announcements.Announcement, int64, error) {
		body := input.Body
		result, err := announcements.Create(ctx, handlers.auth.repository, &actor, body.Title, body.Message, body.Priority, *body.Enabled, &event)
		return result, result.ID, err
	})
}

// updateAnnouncement handles update_announcement with typed input and output.
func (handlers *adminHandlers) updateAnnouncement(ctx context.Context, input *UpdateAnnouncementInput) (*output[*announcements.Announcement], error) {
	return adminOperation(handlers, input.request, "announcement_update", input.AnnouncementID, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (*announcements.Announcement, int64, error) {
		body := input.Body
		result, err := announcements.Modify(ctx, handlers.auth.repository, &actor, input.AnnouncementID, announcements.Update{Title: body.Title, Message: body.Message, Priority: body.Priority, Enabled: body.Enabled}, &event)
		if err == nil && result == nil {
			err = &apiError{status: http.StatusNotFound, detail: "Announcement not found"}
		}
		return result, input.AnnouncementID, err
	})
}

// deleteAnnouncement handles delete_announcement with typed input and output.
func (handlers *adminHandlers) deleteAnnouncement(ctx context.Context, input *DeleteAnnouncementInput) (*output[okResponse], error) {
	return adminOperation(handlers, input.request, "announcement_delete", input.AnnouncementID, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (okResponse, int64, error) {
		changed, err := announcements.Delete(ctx, handlers.auth.repository, &actor, input.AnnouncementID, &event)
		response, failure := changedResponse(changed, err, "Announcement not found")
		return response, input.AnnouncementID, failure
	})
}

// listScheduledTasks handles list_scheduled_tasks with typed input and output.
func (handlers *adminHandlers) listScheduledTasks(ctx context.Context, input *ListScheduledTasksInput) (*output[[]scheduled.Task], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) ([]scheduled.Task, int64, error) {
		result, err := handlers.scheduler.List(ctx)
		return result, 0, err
	})
}

// createScheduledTask handles create_scheduled_task with typed input and output.
func (handlers *adminHandlers) createScheduledTask(ctx context.Context, input *CreateScheduledTaskInput) (*output[*scheduled.Task], error) {
	return adminOperation(handlers, input.request, "scheduled_task_create", 0, func(identity.ID) (string, *apiError) {
		return "validation_failed", validateScheduledFields(&input.Body.Name, &input.Body.Cron, &input.Body.Timezone)
	}, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (*scheduled.Task, int64, error) {
		body := input.Body
		result, err := handlers.scheduler.Create(ctx, scheduled.Create{Name: body.Name, Job: body.Job, Cron: body.Cron, Timezone: body.Timezone, TimeoutSeconds: uint64(*body.TimeoutSeconds), Coalesce: *body.Coalesce, Enabled: *body.Enabled}, &actor, &event)
		if err != nil {
			return nil, 0, err
		}
		return result, result.ID, nil
	})
}

// updateScheduledTask handles update_scheduled_task with typed input and output.
func (handlers *adminHandlers) updateScheduledTask(ctx context.Context, input *UpdateScheduledTaskInput) (*output[*scheduled.Task], error) {
	return adminOperation(handlers, input.request, "scheduled_task_update", input.TaskID, func(identity.ID) (string, *apiError) {
		return "validation_failed", validateScheduledFields(input.Body.Name, input.Body.Cron, input.Body.Timezone)
	}, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (*scheduled.Task, int64, error) {
		body := input.Body
		var timeout *uint64
		if body.TimeoutSeconds != nil {
			timeout = new(uint64(*body.TimeoutSeconds))
		}
		update := scheduled.Update{TaskID: input.TaskID, Name: body.Name, Job: body.Job, Cron: body.Cron, Timezone: body.Timezone, TimeoutSeconds: timeout, Coalesce: body.Coalesce, Enabled: body.Enabled}
		result, err := handlers.scheduler.Update(ctx, update, &actor, &event)
		if err == nil && result == nil {
			err = &apiError{status: http.StatusNotFound, detail: "Scheduled task not found"}
		}
		return result, input.TaskID, err
	})
}

// deleteScheduledTask handles delete_scheduled_task with typed input and output.
func (handlers *adminHandlers) deleteScheduledTask(ctx context.Context, input *DeleteScheduledTaskInput) (*output[okResponse], error) {
	return adminOperation(handlers, input.request, "scheduled_task_delete", input.TaskID, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (okResponse, int64, error) {
		changed, err := handlers.scheduler.Delete(ctx, input.TaskID, &actor, &event)
		response, failure := changedResponse(changed, err, "Scheduled task not found")
		return response, input.TaskID, failure
	})
}

// schedulerStatus handles scheduler_status with typed input and output.
func (handlers *adminHandlers) schedulerStatus(ctx context.Context, input *SchedulerStatusInput) (*output[scheduled.Status], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (scheduled.Status, int64, error) {
		result, err := handlers.scheduler.Status(ctx, currentTimestamp(), 90, 20)
		return result, 0, err
	})
}

// listRuntimeSettings handles list_runtime_settings with typed input and output.
func (handlers *adminHandlers) listRuntimeSettings(ctx context.Context, input *ListRuntimeSettingsInput) (*output[[]settings.Info], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) ([]settings.Info, int64, error) {
		result, err := handlers.settings.List(ctx)
		return result, 0, err
	})
}

// updateRuntimeSettings handles update_runtime_settings with typed input and output.
func (handlers *adminHandlers) updateRuntimeSettings(ctx context.Context, input *UpdateRuntimeSettingsInput) (*output[[]settings.Info], error) {
	return adminOperation(handlers, input.request, "runtime_settings_update", 0, func(identity.ID) (string, *apiError) {
		return "validation_failed", validateRuntimeValues(input.Body.Values)
	}, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) ([]settings.Info, int64, error) {
		pools := make(map[string]settings.PoolUpdate, len(input.Body.SecretPoolUpdates))
		for field, update := range input.Body.SecretPoolUpdates {
			pools[field] = settings.PoolUpdate{Add: update.Add, Remove: update.Remove}
		}
		result, err := handlers.settings.Update(ctx, &actor, input.Body.Values, pools, &event)
		return result, 0, err
	})
}

// providerCatalog handles get_provider_catalog with typed input and output.
func (handlers *adminHandlers) providerCatalog(ctx context.Context, input *GetProviderCatalogInput) (*output[providerCatalogResponse], error) {
	return adminOperation(handlers, input.request, "", 0, nil, func(ctx context.Context, actor identity.ID, event domain.AuditEvent) (providerCatalogResponse, int64, error) {
		catalogs, err := handlers.storage.ListProviderCatalogs()
		return providerCatalogResponse{sources.BuiltInCapabilities(), catalogs}, 0, err
	})
}

// validateScheduledFields trims supplied labels and checks the domain cron grammar.
func validateScheduledFields(name, expression, timezone *string) *apiError {
	for _, field := range []struct {
		value *string
		label string
	}{{name, "Task name"}, {expression, "Cron"}, {timezone, "Timezone"}} {
		if field.value == nil {
			continue
		}
		*field.value = strings.TrimSpace(*field.value)
		if *field.value == "" {
			return badRequest(field.label + " must not be empty")
		}
	}
	if expression != nil {
		if _, err := cron.Parse(*expression); err != nil {
			return badRequest(err.Error())
		}
	}
	if timezone != nil {
		if err := scheduled.ValidateTiming(*timezone, 3600); err != nil {
			return badRequest(err.Error())
		}
	}
	return nil
}
