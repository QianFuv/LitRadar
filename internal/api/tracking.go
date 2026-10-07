package api

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/domain/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/platform/httpwire"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

type trackingHandlers struct {
	storage       config.Config
	repository    *storage.Repository
	favorites     *favorites.Repository
	settings      *settings.Repository
	codec         *secrets.Codec
	authenticator *Authenticator
	pool          *executor.Pool
}

func (handlers *trackingHandlers) routes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/tracking/status", Id: "status"},
		{Method: "POST", Path: "/api/tracking/push-weekly", Id: "push_weekly_to_tracking"},
		{Method: "GET", Path: "/api/tracking/push-weekly/status", Id: "get_push_weekly_status"},
		{Method: "GET", Path: "/api/tracking/push-weekly/runs/{run_id}", Id: "get_push_weekly_run"},
		{Method: "POST", Path: "/api/tracking/push-weekly/runs/{run_id}/cancel", Id: "cancel_push_weekly_run"},
		{Method: "POST", Path: "/api/tracking/push-weekly/runs/{run_id}/acknowledge", Id: "acknowledge_unknown_push_weekly_run"},
		{Method: "GET", Path: "/api/tracking/notification-settings", Id: "get_notification_settings"},
		{Method: "PUT", Path: "/api/tracking/notification-settings", Id: "update_notification_settings"},
		{Method: "GET", Path: "/api/tracking/ai-endpoints", Id: "get_ai_endpoints"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handle(writer, request, operation.Id)
		}})
	}
	return routes
}

// handle retains extraction before authentication and business checks afterward.
func (handlers *trackingHandlers) handle(writer http.ResponseWriter, request *http.Request, operation string) {
	runId := request.PathValue("run_id")
	if !utf8.ValidString(runId) {
		(&apiError{status: 400, detail: "Invalid URL: Invalid UTF-8 in `run_id`", isPlain: true}).write(writer)
		return
	}
	var update domain.NotificationSettingsUpdate
	if operation == "update_notification_settings" {
		decoded, failure := extractBody(request, notificationBody(), false)
		if failure != nil {
			failure.write(writer)
			return
		}
		update = notificationUpdate(decoded.(map[string]any))
	}
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		failure.write(writer)
		return
	}
	if operation == "update_notification_settings" {
		handlers.respondNotificationUpdate(writer, request, current, update)
		return
	}
	if failure = validateManualRunId(operation, runId); failure != nil {
		failure.write(writer)
		return
	}
	type result struct {
		payload any
		failure *apiError
	}
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		payload, failure := handlers.perform(context.WithoutCancel(request.Context()), operation, current.authorization.User, runId, requestId(request))
		return result{payload, failure}, nil
	})
	if err != nil {
		mapExecutorError(err).write(writer)
		return
	}
	if value.failure != nil {
		value.failure.write(writer)
		return
	}
	_ = httpwire.JSON(writer, trackingResponseStatus(operation), value.payload)
}

// trackingResponseStatus reports durable admission and acknowledgement as accepted.
func trackingResponseStatus(operation string) int {
	if operation == "push_weekly_to_tracking" || operation == "acknowledge_unknown_push_weekly_run" {
		return 202
	}
	return 200
}

func manualRunMissing() *apiError { return &apiError{status: 404, detail: "Manual push job not found"} }
func newManualRun(user int64) (storage.RunCreate, error) {
	var entropy [16]byte
	if _, err := rand.Read(entropy[:]); err != nil {
		return storage.RunCreate{}, err
	}
	now := currentTimestamp()
	deadline := now + 600
	return storage.RunCreate{ExternalId: hex.EncodeToString(entropy[:]), Workflow: storage.WorkflowPush, ScopeKey: fmt.Sprintf("manual-user-%d", user), TriggerKind: storage.TriggerKindManual, Mode: storage.RunModeExecute, UserId: &user, DeadlineAt: &deadline, CreatedAt: now}, nil
}

// perform dispatches authenticated tracking queries and durable manual-run operations.
func (handlers *trackingHandlers) perform(ctx context.Context, operation string, user auth.User, externalId, requestId string) (any, *apiError) {
	userId := int64(user.Id)
	switch operation {
	case "get_ai_endpoints", "get_notification_settings":
		return handlers.trackingSettings(ctx, operation, userId)
	case "status":
		return handlers.trackingStatus(ctx, user)
	case "push_weekly_to_tracking", "acknowledge_unknown_push_weekly_run":
		return handlers.admitManualRun(ctx, operation, userId, externalId, requestId)
	case "get_push_weekly_status":
		record, err := handlers.repository.LoadLatestManualRun(ctx, userId)
		if err != nil {
			return nil, internalError()
		}
		return manualStatus(record), nil
	}
	return handlers.performManualRun(ctx, operation, user, externalId)
}

// respondNotificationUpdate preserves domain validation before retry bounds and persistence.
func (handlers *trackingHandlers) respondNotificationUpdate(writer http.ResponseWriter, request *http.Request, current currentIdentity, update domain.NotificationSettingsUpdate) {
	if err := domain.ValidateNotificationSettings(update); err != nil {
		badRequest(err.Error()).write(writer)
		return
	}
	if update.AiRetryAttempts < 1 || update.AiRetryAttempts > 10 {
		badRequest("ai_retry_attempts must be between 1 and 10").write(writer)
		return
	}
	payload, failure := handlers.updateNotification(request.Context(), int64(current.authorization.User.Id), update)
	if failure != nil {
		failure.write(writer)
	} else {
		writeResponse(writer, payload)
	}
	return
}

// validateManualRunId checks only named run operations after authentication has succeeded.
func validateManualRunId(operation, runId string) *apiError {
	if strings.Contains(operation, "_weekly_run") || operation == "acknowledge_unknown_push_weekly_run" {
		if len(runId) != 32 || strings.ContainsFunc(runId, func(value rune) bool {
			return !(value >= '0' && value <= '9' || value >= 'a' && value <= 'f' || value >= 'A' && value <= 'F')
		}) {
			return manualRunMissing()
		}
	}

	return nil
}

// trackingSettings preserves nil notification settings and endpoint list projection.
func (handlers *trackingHandlers) trackingSettings(ctx context.Context, operation string, userId int64) (any, *apiError) {
	switch operation {
	case "get_ai_endpoints":
		values, err := handlers.settings.AiBaseUrls(ctx)
		if err != nil {
			return nil, internalError()
		}
		return values, nil
	case "get_notification_settings":
		value, err := handlers.repository.GetNotificationSettings(ctx, handlers.codec, userId)
		if err != nil {
			return nil, internalError()
		}
		if value == nil {
			return nil, nil
		}
		return value.Public(), nil

	}
	return nil, nil
}

// trackingStatus loads folders and settings before computing available weekly articles.
func (handlers *trackingHandlers) trackingStatus(ctx context.Context, user auth.User) (any, *apiError) {
	userId := int64(user.Id)
	folder, err := handlers.favorites.TrackingFolder(ctx, user.Id)
	if err != nil {
		return nil, internalError()
	}
	folders, err := handlers.favorites.ListFolders(ctx, user.Id)
	if err != nil {
		return nil, internalError()
	}
	settings, err := handlers.repository.GetNotificationSettings(ctx, handlers.codec, userId)
	if err != nil {
		return nil, internalError()
	}
	selected := []string{}
	if settings != nil {
		selected = settings.SelectedDatabases
	}
	count, err := weekly.CountAvailable(ctx, handlers.storage, weekly.FromTime(time.Now()), selected)
	if err != nil {
		return nil, internalError()
	}
	type folderSummary struct {
		Id   int64  `json:"id"`
		Name string `json:"name"`
	}
	var tracking *folderSummary
	if folder != nil {
		tracking = &folderSummary{folder.Id, folder.Name}
	}
	return struct {
		TrackingFolder          *folderSummary `json:"tracking_folder"`
		TotalFolders            int            `json:"total_folders"`
		WeeklyArticlesAvailable int            `json:"weekly_articles_available"`
		NotificationConfigured  bool           `json:"notification_configured"`
	}{tracking, len(folders), count, settings != nil}, nil
}

// admitManualRun creates the replacement identity before acknowledgement or regular admission.
func (handlers *trackingHandlers) admitManualRun(ctx context.Context, operation string, userId int64, externalId, requestId string) (any, *apiError) {
	run, err := newManualRun(userId)
	if err != nil {
		return nil, internalError()
	}
	if operation == "acknowledge_unknown_push_weekly_run" {
		return handlers.acknowledgeManualRun(ctx, userId, externalId, requestId, run)
	}
	outcome, err := handlers.repository.AdmitManualRun(ctx, run)
	if err != nil {
		return nil, internalError()
	}
	if outcome.Kind == "blocked_unknown" {
		return nil, &apiError{status: 409, detail: "Manual push outcome is unknown; review delivery state before retrying"}
	}
	return manualStatus(outcome.Run), nil
}

// acknowledgeManualRun keeps audit identity and replacement admission atomic in storage.
func (handlers *trackingHandlers) acknowledgeManualRun(ctx context.Context, userId int64, externalId, requestId string, run storage.RunCreate) (any, *apiError) {
	event := auth.AuditEvent{Action: "manual_push_unknown_acknowledge", Outcome: "completed", RequestId: requestId, OccurredAt: currentTimestamp()}
	record, err := handlers.repository.AcknowledgeUnknownManualRun(ctx, userId, externalId, run, &event)
	if errors.Is(err, storage.ErrNotFound) {
		return nil, manualRunMissing()
	}
	if errors.Is(err, storage.ErrConflict) {
		return nil, &apiError{status: 409, detail: "Manual push is no longer the latest unknown outcome"}
	}
	if err != nil {
		return nil, internalError()
	}
	return manualStatus(record), nil
}

// performManualRun uses admin visibility only for inspection and cancellation.
func (handlers *trackingHandlers) performManualRun(ctx context.Context, operation string, user auth.User, externalId string) (any, *apiError) {
	userId := int64(user.Id)
	var record *storage.RunRecord
	var err error
	if user.IsAdmin {
		record, err = handlers.repository.LoadManualRunForAdmin(ctx, externalId)
	} else {
		record, err = handlers.repository.LoadManualRun(ctx, userId, externalId)
	}
	if err != nil {
		return nil, internalError()
	}
	if record == nil {
		return nil, manualRunMissing()
	}
	if operation == "cancel_push_weekly_run" && !record.Status.IsTerminal() && !record.CancellationRequested {

		changed, failure := handlers.cancelManualRun(ctx, record)
		if failure != nil {
			return nil, failure
		}
		record = changed

	}
	return manualStatus(record), nil
}

// cancelManualRun reloads the actual record after an optimistic concurrency conflict.
func (handlers *trackingHandlers) cancelManualRun(ctx context.Context, record *storage.RunRecord) (*storage.RunRecord, *apiError) {
	changed, err := handlers.repository.CancelRun(ctx, record.Id, record.Revision, currentTimestamp())
	if errors.Is(err, storage.ErrConflict) {
		changed, err = handlers.repository.LoadRun(ctx, record.Id)
	}
	if err != nil {
		return nil, internalError()
	}
	if changed == nil {
		return nil, internalError()
	}
	return changed, nil
}

func trackingStorage[Value any](ctx context.Context, pool *executor.Pool, work func() (Value, error)) (Value, *apiError) {
	type result struct {
		value Value
		err   error
	}
	observed, err := executor.Run(ctx, pool, func() (result, error) { value, err := work(); return result{value, err}, nil })
	if err != nil {
		return observed.value, mapExecutorError(err)
	}
	if observed.err != nil {
		return observed.value, internalError()
	}
	return observed.value, nil
}
