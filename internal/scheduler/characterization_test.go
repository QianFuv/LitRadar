package scheduler

import (
	"bytes"
	"context"
	"fmt"
	"log/slog"
	"path/filepath"
	"strings"
	"testing"
	"time"

	store "github.com/QianFuv/LitRadar/internal/storage/scheduler"
)

// TestSchedulerHeartbeatPanicRetainsObservedStateAndReapsChild checks caller-owned state during unwinding.
func TestSchedulerHeartbeatPanicRetainsObservedStateAndReapsChild(t *testing.T) {
	command, directory := processFixture(t, "tree")
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	address := ""
	recovered := func() (value any) {
		defer func() { value = recover() }()
		executeProcess(context.Background(), command, testClaim(), 1, time.Now().Add(8*time.Second), 0, func() bool {
			address = waitDescendant(t, directory)
			panic("fixture heartbeat panic")
		})
		return nil
	}()
	if recovered != "fixture heartbeat panic" || strings.Count(logs.String(), `"event":"scheduler.child.completed"`) != 1 || strings.Contains(logs.String(), `"event":"scheduler.child.failed"`) {
		t.Fatal(recovered, logs.String())
	}
	assertDescendantStopped(t, address)
}

// TestSchedulerCancelledAdmissionDoesNotSpawnOrRenew preserves cancellation ahead of supervision errors.
func TestSchedulerCancelledAdmissionDoesNotSpawnOrRenew(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	calls := 0
	result := executeProcess(ctx, scheduledProcess{command: "index", path: filepath.Join(t.TempDir(), "missing")}, testClaim(), 1, time.Now().Add(-time.Second), 0, func() bool { calls++; return false })
	if result.status != "cancelled" || result.summary != "index: cancelled" || calls != 0 {
		t.Fatal(result, calls)
	}
}

// assertEarlierScheduledAdmission checks durable counters before the failed tick advances its cursor or claims.
func assertEarlierScheduledAdmission(t *testing.T, repository *store.Repository, taskId int64) {
	t.Helper()
	status, err := repository.Status(context.Background(), 120, 90, 10)
	if err != nil || len(status.RecentRuns) != 1 || status.LastCheckedAt != nil {
		t.Fatal(status, err)
	}
	run := status.RecentRuns[0]
	if run.TaskId != taskId || run.Status != "pending" || run.ScheduledFor != 120 || run.WorkerId != nil {
		t.Fatal(run)
	}
}

// TestSchedulerRetainsPartialTickAndEarlierAdmissionOnLaterEnqueueFailure checks ordered durable task admission.
func TestSchedulerRetainsPartialTickAndEarlierAdmissionOnLaterEnqueueFailure(t *testing.T) {
	repository, filename := schedulerFixture(t)
	first := createTask(t, repository)
	second := createTask(t, repository)
	fixtureSql(t, filename, "UPDATE scheduled_tasks SET created_at=0")
	fixtureSql(t, filename, fmt.Sprintf("CREATE TRIGGER fail_later_admission BEFORE INSERT ON scheduled_task_runs WHEN NEW.task_id=%d BEGIN SELECT RAISE(ABORT,'later admission failed'); END", second.Id))
	result, claims, err := PrepareRunsAt(context.Background(), repository, "owner", 1, 120)
	if err == nil || result.Jobs != 2 || result.Due != 6 || result.Queued != 1 || result.Claimed != 0 || claims != nil {
		t.Fatal(result, claims, err)
	}
	assertEarlierScheduledAdmission(t, repository, first.Id)
}
