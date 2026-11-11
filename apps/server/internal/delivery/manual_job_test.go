package delivery

import (
	"context"
	"errors"
	"math"
	"runtime"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	storageconfig "github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	store "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
)

func enqueueManual(t *testing.T, repository *store.Repository, created, deadline float64) *store.RunRecord {
	t.Helper()
	run, err := repository.EnqueueRun(context.Background(), store.RunCreate{ExternalID: "manual-attempt", Workflow: store.WorkflowPush, ScopeKey: "manual:1", TriggerKind: store.TriggerKindManual, Mode: store.RunModeExecute, UserID: new(int64(1)), DeadlineAt: &deadline, CreatedAt: created})
	if err != nil {
		t.Fatal(err)
	}
	return run
}

func TestManualJobCompletesDisabledSettingsAndReusesTerminalRecord(t *testing.T) {
	repository, database, config := durableFixture(t)
	now := unixNow()
	run := enqueueManual(t, repository, now, now+60)
	storage := storageconfig.FromProjectRoot(t.TempDir()).WithAuthDBPath(config.AuthDBPath)
	terminal, err := RunManualDeliveryJob(context.Background(), storage, nil, run.ID, "owner")
	if err != nil || terminal.Status != store.RunStatusCompleted || terminal.OwnerID != nil || terminal.ResultJSON == nil || !strings.Contains(*terminal.ResultJSON, "Recommendation settings are not enabled; skipped push") {
		t.Fatal("manual completion", err)
	}
	replay, err := RunManualDeliveryJob(context.Background(), storage, nil, run.ID, "")
	if err != nil || replay.Revision != terminal.Revision || countRows(t, database, "delivery_checkpoints") != 0 || countRows(t, database, "delivery_run_items") != 0 {
		t.Fatal("top-level job touched child state", err)
	}
}

func TestManualJobTimesOutQueuedAndReclaimsExpiredOwner(t *testing.T) {
	for _, expired := range []bool{false, true} {
		t.Run(map[bool]string{false: "restart", true: "expired"}[expired], func(t *testing.T) {
			repository, _, config := durableFixture(t)
			now := unixNow()
			deadline := now + 60
			if expired {
				deadline = now - 1
			}
			run := enqueueManual(t, repository, now-10, deadline)
			if !expired {
				claim, err := repository.ClaimRun(context.Background(), run.ID, "crashed", run.Revision, now-10, 1)
				if err != nil {
					t.Fatal(err)
				}
				if _, err = repository.StartRun(context.Background(), run.ID, "crashed", claim.Run.Revision, now-10); err != nil {
					t.Fatal(err)
				}
			}
			terminal, err := RunManualDeliveryJob(context.Background(), storageconfig.FromProjectRoot(t.TempDir()).WithAuthDBPath(config.AuthDBPath), nil, run.ID, "replacement")
			want := store.RunStatusCompleted
			if expired {
				want = store.RunStatusTimedOut
			}
			if err != nil || terminal.Status != want || terminal.OwnerID != nil {
				t.Fatal("recovery", err)
			}
		})
	}
}

func TestManualJobContextValidationPrecedesCancellationAndDeadline(t *testing.T) {
	for _, invalid := range []bool{false, true} {
		t.Run(map[bool]string{false: "cancelled", true: "invalid"}[invalid], func(t *testing.T) {
			repository, database, config := durableFixture(t)
			now := unixNow()
			run := enqueueManual(t, repository, now-10, now-1)
			if invalid {
				if _, err := database.Exec("UPDATE delivery_runs SET mode='dry_run',cancellation_requested=1 WHERE id=?", run.ID); err != nil {
					t.Fatal(err)
				}
			} else {
				if _, err := database.Exec("UPDATE delivery_runs SET cancellation_requested=1 WHERE id=?", run.ID); err != nil {
					t.Fatal(err)
				}
			}
			terminal, err := RunManualDeliveryJob(context.Background(), storageconfig.FromProjectRoot(t.TempDir()).WithAuthDBPath(config.AuthDBPath), nil, run.ID, "owner")
			want, code := store.RunStatusCancelled, "cancelled"
			if invalid {
				want, code = store.RunStatusFailed, "invalid_job_context"
			}
			if err != nil || terminal.Status != want || terminal.ErrorCode == nil || *terminal.ErrorCode != code {
				t.Fatal("validation precedence", err)
			}
		})
	}
	run := store.RunRecord{TriggerKind: store.TriggerKindManual, Mode: store.RunModeExecute, UserID: new(int64(1)), Workflow: store.WorkflowNotify, CreatedAt: 100, DeadlineAt: new(701.0)}
	if !isValidManualJob(run) {
		t.Fatal("valid 601-second context rejected")
	}
	run.DeadlineAt = new(math.Nextafter(701, math.Inf(1)))
	if isValidManualJob(run) {
		t.Fatal("oversized deadline accepted")
	}
}

func TestManualJobSharesOriginalWindowBudgetAndLatestRevision(t *testing.T) {
	repository, _, config := durableFixture(t)
	now := unixNow()
	run := enqueueManual(t, repository, now, now+60)
	storage := storageconfig.FromProjectRoot(t.TempDir()).WithAuthDBPath(config.AuthDBPath)
	terminal, err := runManualDeliveryJob(context.Background(), repository, storage, nil, run.ID, "owner", func(ctx context.Context, job ManualWeeklyPushConfig) (ManualWeeklyPushOutcome, error) {
		if job.AttemptID != run.ExternalID || job.UserID != 1 || job.TimeoutSeconds != 120 || job.RetryAttempts != 3 || job.DedupeRetentionDays != 60 || job.ExecutionControl.Deadline() != *run.DeadlineAt {
			t.Fatal("manual context changed")
		}
		stamp, _ := manualCreationTime(run.CreatedAt)
		if stamp != job.WindowEnd {
			t.Fatal("window drifted")
		}
		for range 8 {
			if _, err := job.ExecutionControl.BeginAiRequest(time.Second); err != nil {
				t.Fatal(err)
			}
		}
		if _, err := job.ExecutionControl.BeginAiRequest(time.Second); !errors.Is(err, domain.ControlBudgetExhausted) {
			t.Fatal("budget replenished", err)
		}
		current, err := repository.LoadRun(ctx, run.ID)
		if err != nil {
			t.Fatal(err)
		}
		if _, err = repository.CancelRun(ctx, run.ID, current.Revision, unixNow()); err != nil {
			t.Fatal(err)
		}
		return ManualWeeklyPushOutcome{}, job.ExecutionControl.Check()
	})
	if err != nil || terminal.Status != store.RunStatusCancelled || terminal.ErrorCode == nil || *terminal.ErrorCode != "cancelled" {
		t.Fatal("latest revision was not finalized", err)
	}
}

func TestManualJobRespectsConcurrentTerminalAndOwnershipChanges(t *testing.T) {
	for _, isTerminal := range []bool{false, true} {
		t.Run(map[bool]string{false: "new-owner", true: "terminal"}[isTerminal], func(t *testing.T) {
			repository, database, config := durableFixture(t)
			now := unixNow()
			run := enqueueManual(t, repository, now, now+60)
			terminal, err := runManualDeliveryJob(context.Background(), repository, storageconfig.FromProjectRoot(t.TempDir()).WithAuthDBPath(config.AuthDBPath), nil, run.ID, "owner", func(ctx context.Context, _ ManualWeeklyPushConfig) (ManualWeeklyPushOutcome, error) {
				if isTerminal {
					current, err := repository.LoadRun(ctx, run.ID)
					if err != nil {
						t.Fatal(err)
					}
					if _, err = repository.FinalizeRun(ctx, run.ID, "owner", current.Revision, store.RunStatusUnknown, nil, new("ambiguous_delivery"), unixNow()); err != nil {
						t.Fatal(err)
					}
				} else {
					if _, err := database.Exec("UPDATE delivery_runs SET owner_id='replacement',revision=revision+1 WHERE id=?", run.ID); err != nil {
						t.Fatal(err)
					}
				}
				return ManualWeeklyPushOutcome{Status: "completed"}, nil
			})
			if isTerminal {
				if err != nil || terminal.Status != store.RunStatusUnknown {
					t.Fatal("terminal overwritten", err)
				}
			} else if !errors.Is(err, store.ErrConflict) {
				t.Fatal("new owner overwritten", err)
			}
		})
	}
}

func TestManualOutcomeAndControlErrorKeepDistinctCodes(t *testing.T) {
	for _, entry := range []struct {
		state   string
		failure error
		status  store.RunStatus
		code    string
	}{{"cancelled", domain.ControlCancelled, store.RunStatusCancelled, "cancelled"}, {"timed_out", domain.ControlTimedOut, store.RunStatusTimedOut, "deadline_exceeded"}} {
		status, code := manualOutcomeTerminal(entry.state)
		if status != entry.status || code != nil {
			t.Fatal("outcome gained an error code")
		}
		status, errorCode := manualErrorTerminal(entry.failure)
		if status != entry.status || errorCode != entry.code {
			t.Fatal("control classification lost")
		}
	}
}

func TestManualJobPersistsCancellationAfterExecutionContextEnds(t *testing.T) {
	repository, _, config := durableFixture(t)
	now := unixNow()
	run := enqueueManual(t, repository, now, now+60)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	terminal, err := runManualDeliveryJob(ctx, repository, storageconfig.FromProjectRoot(t.TempDir()).WithAuthDBPath(config.AuthDBPath), nil, run.ID, "owner", func(context.Context, ManualWeeklyPushConfig) (ManualWeeklyPushOutcome, error) {
		cancel()
		return ManualWeeklyPushOutcome{}, domain.ControlCancelled
	})
	if err != nil || terminal.Status != store.RunStatusCancelled || terminal.OwnerID != nil {
		t.Fatal("cancelled execution did not persist its terminal state", err)
	}
}

func TestManualCreationTimePreservesPlatformSystemTimePrecision(t *testing.T) {
	stamp, err := manualCreationTime(1800000000 + math.Ldexp(1, -22))
	want := uint32(238)
	if runtime.GOOS == "windows" {
		want = 200
	}
	if err != nil || stamp.Seconds != 1800000000 || stamp.Nanoseconds != want {
		t.Fatalf("window precision: %+v %v", stamp, err)
	}
}
