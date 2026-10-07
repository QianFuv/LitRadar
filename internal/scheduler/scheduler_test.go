package scheduler

import (
	"context"
	"errors"
	"path/filepath"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	store "github.com/QianFuv/LitRadar/internal/storage/scheduler"
)

func schedulerFixture(t *testing.T) (*store.Repository, string) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := store.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	return repository, filename
}
func fixtureSql(t *testing.T, filename, statement string, args ...any) {
	t.Helper()
	database, err := platform.Open(platform.Config{Filename: filename, Mode: "rw", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec(statement, args...); err != nil {
		t.Fatal(err)
	}
}
func createTask(t *testing.T, repository *store.Repository) domain.Task {
	t.Helper()
	task, err := repository.Create(context.Background(), domain.Create{Name: "fixture", Job: domain.Job{Kind: "index"}, Cron: "* * * * *", Timezone: "UTC", TimeoutSeconds: 60, Coalesce: true, Enabled: true}, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	return *task
}

func TestSchedulerTickCatchupCapacityAndClockRollback(t *testing.T) {
	repository, filename := schedulerFixture(t)
	ctx := context.Background()
	task := createTask(t, repository)
	fixtureSql(t, filename, "UPDATE scheduled_tasks SET created_at=0")
	assertInitialSchedulerCatchup(t, ctx, repository)
	claim := assertPendingSchedulerClaim(t, ctx, repository)
	if _, err := repository.FinishRun(ctx, claim, domain.Success, "", 121); err != nil {
		t.Fatal(err)
	}
	assertSchedulerClockRollback(t, ctx, repository)
	assertNoncoalescedSchedulerCatchup(t, ctx, repository, filename, task.Id)
}

func TestSchedulerLoadAndManualDryRunDoNotMutate(t *testing.T) {
	repository, filename := schedulerFixture(t)
	ctx := context.Background()
	first := createTask(t, repository)
	second := createTask(t, repository)
	fixtureSql(t, filename, "UPDATE scheduled_tasks SET cron='invalid',enabled=0 WHERE id=?", first.Id)
	fixtureSql(t, filename, "UPDATE scheduled_tasks SET cron='invalid' WHERE id=?", second.Id)
	loaded, err := LoadJobs(ctx, repository)
	if err != nil || len(loaded.Jobs) != 0 || len(loaded.Skipped) != 1 {
		t.Fatalf("%#v %v", loaded, err)
	}
	executed := false
	runner := func(context.Context, domain.Task, store.Claim, func() bool) processResult {
		executed = true
		return processResult{domain.Success, ""}
	}
	assertManualSchedulerInspection(t, ctx, repository, first.Id, runner, &executed)
	status, err := repository.Status(ctx, 0, 90, 10)
	if err != nil || len(status.RecentRuns) != 0 {
		t.Fatal("inspection admitted a run")
	}
}

func TestSchedulerClaimOwnershipAndHeartbeatFailures(t *testing.T) {
	for _, scenario := range []string{"unavailable", "heartbeat-lost", "heartbeat-error", "finish-lost", "cancelled"} {
		t.Run(scenario, func(t *testing.T) {
			repository, filename := schedulerFixture(t)
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			task := createTask(t, repository)
			admission, err := repository.ClaimManual(ctx, task.Id, "owner", currentTime(), 90)
			if err != nil {
				t.Fatal(err)
			}
			claim := *admission.Claim
			if scenario == "unavailable" {
				claim.WorkerId = "competitor"
			}
			executions := 0
			runner := func(ctx context.Context, task domain.Task, claim store.Claim, heartbeat func() bool) processResult {
				executions++
				status, err := repository.Status(context.Background(), currentTime(), 90, 10)
				if err != nil || status.RecentRuns[0].Status != domain.Running {
					t.Fatal("child started before durable running")
				}
				switch scenario {
				case "heartbeat-lost", "finish-lost":
					fixtureSql(t, filename, "UPDATE scheduled_task_runs SET worker_id='replacement'")
				case "heartbeat-error":
					fixtureSql(t, filename, "CREATE TRIGGER heartbeat_failure BEFORE INSERT ON scheduler_workers BEGIN SELECT RAISE(ABORT,'synthetic error');END")
				case "cancelled":
					cancel()
					return processResult{domain.Cancelled, "cancelled"}
				}
				if scenario == "heartbeat-lost" || scenario == "heartbeat-error" {
					if heartbeat() {
						t.Fatal("heartbeat failure did not stop runner")
					}
					return processResult{domain.Unknown, "heartbeat lost"}
				}
				return processResult{domain.Success, "success"}
			}
			result, didStart, err := runClaim(ctx, repository, claim, runner)
			switch scenario {
			case "unavailable":
				if err != nil || didStart || executions != 0 || result.Status != domain.Unknown {
					t.Fatalf("%#v %v %v", result, didStart, err)
				}
			case "heartbeat-lost", "finish-lost":
				if !errors.Is(err, ErrHeartbeatLost) {
					t.Fatal(err)
				}
			case "heartbeat-error":
				if err == nil || errors.Is(err, ErrHeartbeatLost) {
					t.Fatal(err)
				}
			case "cancelled":
				if err != nil || result.Status != domain.Cancelled || !didStart {
					t.Fatalf("%#v %v", result, err)
				}
			}
			status, err := repository.Status(context.Background(), currentTime(), 90, 10)
			if err != nil {
				t.Fatal(err)
			}
			if scenario == "heartbeat-lost" || scenario == "heartbeat-error" || scenario == "finish-lost" {
				if status.RecentRuns[0].FinishedAt != nil {
					t.Fatal("lost claim was finalized")
				}
			}
		})
	}
}

// assertInitialSchedulerCatchup retains its complete ordered admission or recovery assertion phase.
func assertInitialSchedulerCatchup(t *testing.T, ctx context.Context, repository *store.Repository) {
	t.Helper()
	result, claims, err := PrepareRunsAt(ctx, repository, "first", 0, 120)
	if err != nil {
		t.Fatal(err)
	}
	if result.Due != 3 || result.Queued != 1 || result.Claimed != 0 || len(claims) != 0 || result.CheckedFrom != -86280 {
		t.Fatalf("first tick %#v %v", result, claims)
	}
}

// assertPendingSchedulerClaim retains its complete ordered admission or recovery assertion phase.
func assertPendingSchedulerClaim(t *testing.T, ctx context.Context, repository *store.Repository) store.Claim {
	t.Helper()
	result, claims, err := PrepareRunsAt(ctx, repository, "second", 1, 120)
	if err != nil || result.Due != 0 || result.Claimed != 1 || len(claims) != 1 || claims[0].ScheduledFor != 120 {
		t.Fatalf("pending tick %#v %v %v", result, claims, err)
	}
	return claims[0]
}

// assertSchedulerClockRollback retains its complete ordered admission or recovery assertion phase.
func assertSchedulerClockRollback(t *testing.T, ctx context.Context, repository *store.Repository) {
	t.Helper()
	result, claims, err := PrepareRunsAt(ctx, repository, "rollback", 1, 60)
	if err != nil || result.Queued != 0 || result.AlreadyExecuted != 1 || len(claims) != 0 {
		t.Fatalf("rollback tick %#v %v %v", result, claims, err)
	}
	cursor, err := repository.LastCheckedAt(ctx)
	if err != nil || cursor == nil || *cursor != 120 {
		t.Fatal("cursor moved backward", cursor, err)
	}
}

// assertNoncoalescedSchedulerCatchup retains its complete ordered admission or recovery assertion phase.
func assertNoncoalescedSchedulerCatchup(t *testing.T, ctx context.Context, repository *store.Repository, filename string, taskId int64) {
	t.Helper()
	fixtureSql(t, filename, "UPDATE scheduled_tasks SET coalesce=0 WHERE id=?", taskId)
	fixtureSql(t, filename, "UPDATE scheduler_state SET last_checked_at=0")
	result, _, err := PrepareRunsAt(ctx, repository, "catchup", 0, 200000)
	if err != nil || result.CheckedFrom != 113600 || result.Due != 1440 || result.Queued != 1440 {
		t.Fatalf("catchup %#v %v", result, err)
	}
}

// assertManualSchedulerInspection checks dry-run, missing-task and invalid execution without changing the shared runner flag.
func assertManualSchedulerInspection(t *testing.T, ctx context.Context, repository *store.Repository, taskId int64, runner jobRunner, executed *bool) {
	t.Helper()
	outcome, err := runTaskNow(ctx, repository, taskId, DryRun, runner)
	if err != nil || !outcome.Found || outcome.DidExecute || *executed {
		t.Fatalf("%#v %v", outcome, err)
	}
	outcome, err = runTaskNow(ctx, repository, 999, Execute, runner)
	if err != nil || outcome.Found || *executed {
		t.Fatalf("%#v %v", outcome, err)
	}
	if _, err = runTaskNow(ctx, repository, taskId, Execute, runner); err == nil || *executed {
		t.Fatal("manual execution skipped validation")
	}
}
