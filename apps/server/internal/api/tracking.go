package api

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
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

func manualRunMissing() *apiError { return &apiError{status: 404, detail: "Manual push job not found"} }
func newManualRun(user int64) (storage.RunCreate, error) {
	var entropy [16]byte
	if _, err := rand.Read(entropy[:]); err != nil {
		return storage.RunCreate{}, err
	}
	now := currentTimestamp()
	deadline := now + 600
	return storage.RunCreate{ExternalID: hex.EncodeToString(entropy[:]), Workflow: storage.WorkflowPush, ScopeKey: fmt.Sprintf("manual-user-%d", user), TriggerKind: storage.TriggerKindManual, Mode: storage.RunModeExecute, UserID: &user, DeadlineAt: &deadline, CreatedAt: now}, nil
}

// trackingStatus loads folders and settings before computing available weekly articles.
func (handlers *trackingHandlers) trackingStatus(ctx context.Context, user auth.User) (trackingStatusResponse, *apiError) {
	userID := int64(user.ID)
	folder, err := handlers.favorites.TrackingFolder(ctx, user.ID)
	if err != nil {
		return trackingStatusResponse{}, internalError()
	}
	folders, err := handlers.favorites.ListFolders(ctx, user.ID)
	if err != nil {
		return trackingStatusResponse{}, internalError()
	}
	settings, err := handlers.repository.GetNotificationSettings(ctx, handlers.codec, userID)
	if err != nil {
		return trackingStatusResponse{}, internalError()
	}
	selected := []string{}
	if settings != nil {
		selected = settings.SelectedDatabases
	}
	count, err := weekly.CountAvailable(ctx, handlers.storage, weekly.FromTime(time.Now()), selected)
	if err != nil {
		return trackingStatusResponse{}, internalError()
	}

	var tracking *folderSummary
	if folder != nil {
		tracking = &folderSummary{folder.ID, folder.Name}
	}
	return trackingStatusResponse{tracking, len(folders), count, settings != nil}, nil
}

// admitManualRun creates the replacement identity before acknowledgement or regular admission.
func (handlers *trackingHandlers) admitManualRun(ctx context.Context, operation string, userID int64, externalID, requestID string) (manualPushStatus, *apiError) {
	run, err := newManualRun(userID)
	if err != nil {
		return manualPushStatus{}, internalError()
	}
	if operation == "acknowledge_unknown_push_weekly_run" {
		return handlers.acknowledgeManualRun(ctx, userID, externalID, requestID, run)
	}
	outcome, err := handlers.repository.AdmitManualRun(ctx, run)
	if err != nil {
		return manualPushStatus{}, internalError()
	}
	if outcome.Kind == "blocked_unknown" {
		return manualPushStatus{}, &apiError{status: 409, detail: "Manual push outcome is unknown; review delivery state before retrying"}
	}
	return manualStatus(outcome.Run), nil
}

// acknowledgeManualRun keeps audit identity and replacement admission atomic in storage.
func (handlers *trackingHandlers) acknowledgeManualRun(ctx context.Context, userID int64, externalID, requestID string, run storage.RunCreate) (manualPushStatus, *apiError) {
	event := auth.AuditEvent{Action: "manual_push_unknown_acknowledge", Outcome: "completed", RequestID: requestID, OccurredAt: currentTimestamp()}
	record, err := handlers.repository.AcknowledgeUnknownManualRun(ctx, userID, externalID, run, &event)
	if errors.Is(err, storage.ErrNotFound) {
		return manualPushStatus{}, manualRunMissing()
	}
	if errors.Is(err, storage.ErrConflict) {
		return manualPushStatus{}, &apiError{status: 409, detail: "Manual push is no longer the latest unknown outcome"}
	}
	if err != nil {
		return manualPushStatus{}, internalError()
	}
	return manualStatus(record), nil
}

// performManualRun uses admin visibility only for inspection and cancellation.
func (handlers *trackingHandlers) performManualRun(ctx context.Context, operation string, user auth.User, externalID string) (manualPushStatus, *apiError) {
	userID := int64(user.ID)
	var record *storage.RunRecord
	var err error
	if user.IsAdmin {
		record, err = handlers.repository.LoadManualRunForAdmin(ctx, externalID)
	} else {
		record, err = handlers.repository.LoadManualRun(ctx, userID, externalID)
	}
	if err != nil {
		return manualPushStatus{}, internalError()
	}
	if record == nil {
		return manualPushStatus{}, manualRunMissing()
	}
	if operation == "cancel_push_weekly_run" && !record.Status.IsTerminal() && !record.CancellationRequested {

		changed, failure := handlers.cancelManualRun(ctx, record)
		if failure != nil {
			return manualPushStatus{}, failure
		}
		record = changed

	}
	return manualStatus(record), nil
}

// cancelManualRun reloads the actual record after an optimistic concurrency conflict.
func (handlers *trackingHandlers) cancelManualRun(ctx context.Context, record *storage.RunRecord) (*storage.RunRecord, *apiError) {
	changed, err := handlers.repository.CancelRun(ctx, record.ID, record.Revision, currentTimestamp())
	if errors.Is(err, storage.ErrConflict) {
		changed, err = handlers.repository.LoadRun(ctx, record.ID)
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
		return observed.value, requestFailure(ctx, err, mapExecutorError)
	}
	if observed.err != nil {
		return observed.value, requestFailure(ctx, observed.err, func(error) *apiError { return internalError() })
	}
	return observed.value, nil
}
