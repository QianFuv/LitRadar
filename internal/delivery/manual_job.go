package delivery

import (
	"context"
	"errors"
	"math"
	"runtime"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storageconfig "github.com/QianFuv/LitRadar/internal/storage/config"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// RunManualDeliveryJob claims an existing dispatcher job and persists its terminal result using current ownership.
func RunManualDeliveryJob(ctx context.Context, config storageconfig.Config, codec *secrets.Codec, runId int64, owner string) (*store.RunRecord, error) {
	repository, err := store.Open(config.AuthDbPath)
	if err != nil {
		return nil, err
	}
	defer repository.Close()
	return runManualDeliveryJob(ctx, repository, config, codec, runId, owner, RunManualWeeklyPush)
}

func runManualDeliveryJob(ctx context.Context, repository *store.Repository, config storageconfig.Config, codec *secrets.Codec, runId int64, owner string, execute func(context.Context, ManualWeeklyPushConfig) (ManualWeeklyPushOutcome, error)) (*store.RunRecord, error) {
	load := func(readContext context.Context) (*store.RunRecord, error) {
		record, err := repository.LoadRun(readContext, runId)
		if err == nil && record == nil {
			err = store.ErrNotFound
		}
		return record, err
	}
	candidate, err := load(ctx)
	if err != nil {
		return nil, err
	}
	if candidate.Status.IsTerminal() {
		return candidate, nil
	}
	now := unixNow()
	deadline := now
	if candidate.DeadlineAt != nil {
		deadline = *candidate.DeadlineAt
	}
	lease := math.Min(math.Max(deadline-now, 1), domain.ManualJobDeadlineSeconds) + 30
	claim, err := repository.ClaimRun(ctx, candidate.Id, owner, candidate.Revision, now, lease)
	if errors.Is(err, store.ErrConflict) {
		return load(ctx)
	}
	if err != nil {
		return nil, err
	}
	if claim.Kind == "unavailable" {
		return claim.Run, nil
	}
	if claim.Kind == "busy" {
		return nil, ErrBusy
	}
	claimed := claim.Run
	finish := func(record *store.RunRecord, status store.RunStatus, result, code *string) (*store.RunRecord, error) {
		return repository.FinalizeRun(context.WithoutCancel(ctx), record.Id, owner, record.Revision, status, result, code, unixNow())
	}
	if !isValidManualJob(*claimed) {
		return finish(claimed, store.RunStatusFailed, nil, pointer("invalid_job_context"))
	}
	if claimed.CancellationRequested {
		return finish(claimed, store.RunStatusCancelled, nil, pointer("cancelled"))
	}
	if now >= deadline {
		return finish(claimed, store.RunStatusTimedOut, nil, pointer("deadline_exceeded"))
	}
	running, err := repository.StartRun(ctx, claimed.Id, owner, claimed.Revision, now)
	if errors.Is(err, store.ErrConflict) {
		current, readError := load(ctx)
		if readError != nil {
			return nil, readError
		}
		if current.OwnerId != nil && *current.OwnerId == owner && current.CancellationRequested {
			return finish(current, store.RunStatusCancelled, nil, pointer("cancelled"))
		}
		return nil, err
	}
	if err != nil {
		return nil, err
	}
	control := domain.NewExecutionControl(deadline, domain.ManualAiRequestBudget, func() (bool, error) {
		record, err := repository.LoadRun(context.WithoutCancel(ctx), runId)
		return record == nil || record.CancellationRequested || record.Status.IsTerminal(), err
	})
	stamp, err := manualCreationTime(running.CreatedAt)
	if err != nil {
		return nil, err
	}
	result, executionError := execute(ctx, ManualWeeklyPushConfig{WindowEnd: stamp, StorageConfig: config, SecretCodec: codec, UserId: *running.UserId, AttemptId: running.ExternalId, TimeoutSeconds: 120, RetryAttempts: 3, DedupeRetentionDays: 60, ExecutionControl: control})
	current, err := load(context.WithoutCancel(ctx))
	if err != nil {
		return nil, err
	}
	if current.Status.IsTerminal() {
		return current, nil
	}
	if current.OwnerId == nil || *current.OwnerId != owner {
		return nil, store.ErrConflict
	}
	if executionError != nil {
		status, code := manualErrorTerminal(executionError)
		return finish(current, status, nil, &code)
	}
	status, code := manualOutcomeTerminal(result.Status)
	encoded, err := resultJson(result)
	if err != nil {
		return nil, errors.New("Manual delivery result serialization failed")
	}
	return finish(current, status, &encoded, code)
}

func isValidManualJob(run store.RunRecord) bool {
	return run.TriggerKind == store.TriggerKindManual && run.Mode == store.RunModeExecute && run.UserId != nil && *run.UserId > 0 && run.DbName == nil && run.DeadlineAt != nil && !math.IsNaN(*run.DeadlineAt) && !math.IsInf(*run.DeadlineAt, 0) && *run.DeadlineAt > run.CreatedAt && *run.DeadlineAt-run.CreatedAt <= domain.ManualJobDeadlineSeconds+1
}

func manualCreationTime(value float64) (weekly.Timestamp, error) {
	if math.IsNaN(value) || math.IsInf(value, 0) || value < 0 || value >= float64(math.MaxInt64) {
		return weekly.Timestamp{}, errors.New("Manual job creation time is invalid")
	}
	seconds, fraction := math.Modf(value)
	nanoseconds := math.RoundToEven(fraction * 1e9)
	if nanoseconds == 1e9 {
		seconds++
		nanoseconds = 0
	}
	if runtime.GOOS == "windows" {
		nanoseconds = math.Floor(nanoseconds/100) * 100
	}
	return weekly.Timestamp{Seconds: int64(seconds), Nanoseconds: uint32(nanoseconds)}, nil
}

func manualOutcomeTerminal(status string) (store.RunStatus, *string) {
	switch status {
	case "completed":
		return store.RunStatusCompleted, nil
	case "unknown":
		return store.RunStatusUnknown, pointer("ambiguous_delivery")
	case "cancelled":
		return store.RunStatusCancelled, nil
	case "timed_out":
		return store.RunStatusTimedOut, nil
	default:
		return store.RunStatusFailed, pointer("delivery_failed")
	}
}

func manualErrorTerminal(err error) (store.RunStatus, string) {
	kind := deliveryErrorKind(err)
	switch kind {
	case "cancelled":
		return store.RunStatusCancelled, kind
	case "deadline_exceeded":
		return store.RunStatusTimedOut, kind
	case "cancellation_state_unavailable", "ai_request_budget_exhausted":
		return store.RunStatusFailed, kind
	case "busy":
		return store.RunStatusFailed, "workflow_busy"
	default:
		return store.RunStatusFailed, kind + "_failed"
	}
}
