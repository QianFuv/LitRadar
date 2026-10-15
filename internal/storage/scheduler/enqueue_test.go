package scheduler

import (
	"context"
	"database/sql"
	"fmt"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
)

// TestEnqueueSlotIdentityAndCoalescing preserves duplicates, ordered history, and the scheduled-only watermark.
func TestEnqueueSlotIdentityAndCoalescing(t *testing.T) {
	repository, _ := fixture(t)
	ctx := context.Background()
	input := taskInput()
	input.Coalesce = false
	task, err := repository.Create(ctx, input, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		slots []int64
		count int
	}{{[]int64{60, 120, 120, 180}, 3}, {[]int64{60, 120, 180}, 0}, {[]int64{120, 240}, 1}} {
		if inserted, err := repository.Enqueue(ctx, *task, scenario.slots); err != nil || inserted != scenario.count {
			t.Fatal("slot identity changed", inserted, scenario.count, err)
		}
	}
	status, err := repository.Status(ctx, 100, 90, 10)
	if err != nil || len(status.RecentRuns) != 4 {
		t.Fatal(status, err)
	}
	for index, expected := range []int64{240, 180, 120, 60} {
		if run := status.RecentRuns[index]; run.ScheduledFor != expected || run.TaskName != task.Name || run.Status != domain.Pending {
			t.Fatal("scheduled history changed", run, expected)
		}
	}
	task.Coalesce = true
	if err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "UPDATE scheduled_tasks SET coalesce=1 WHERE id=?", task.Id)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if admission, err := repository.ClaimManual(ctx, task.Id, "manual", 1000, 90); err != nil || admission.Claim == nil {
		t.Fatal(admission, err)
	}
	if inserted, err := repository.Enqueue(ctx, *task, []int64{300, 360}); err != nil || inserted != 1 {
		t.Fatal("manual history advanced scheduled watermark", inserted, err)
	}
	if inserted, err := repository.Enqueue(ctx, *task, []int64{300}); err != nil || inserted != 0 {
		t.Fatal("older catchup replaced the latest scheduled slot", inserted, err)
	}
	status, err = repository.Status(ctx, 1001, 90, 10)
	if err != nil || len(status.RecentRuns) != 2 || status.RecentRuns[0].ScheduledFor != 1000 || status.RecentRuns[0].Status != domain.Claimed || status.RecentRuns[1].ScheduledFor != 360 || status.RecentRuns[1].Status != domain.Pending {
		t.Fatal("coalescing lost manual or latest scheduled history", status, err)
	}
}

// TestEnqueueFailureRollsBackEarlierSlots rejects partial publication when a later insert fails.
func TestEnqueueFailureRollsBackEarlierSlots(t *testing.T) {
	repository, _ := fixture(t)
	ctx := context.Background()
	input := taskInput()
	input.Coalesce = false
	task, err := repository.Create(ctx, input, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "CREATE TRIGGER reject_slot BEFORE INSERT ON scheduled_task_runs WHEN NEW.scheduled_for=120 BEGIN SELECT RAISE(ABORT,'slot rejected'); END")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	before := snapshot(t, repository)
	if _, err := repository.Enqueue(ctx, *task, []int64{60, 120, 180}); err == nil {
		t.Fatal("later slot failure was ignored")
	}
	if after := snapshot(t, repository); !reflect.DeepEqual(before, after) {
		t.Fatal("failed enqueue published partial durable state")
	}
}

// BenchmarkEnqueueSlots measures actual transactions while removing copied rows outside the timer.
func BenchmarkEnqueueSlots(b *testing.B) {
	for _, scenario := range []struct {
		count    int
		coalesce bool
	}{{1, false}, {64, false}, {1440, false}, {1440, true}} {
		b.Run(fmt.Sprintf("slots=%d/coalesce=%t", scenario.count, scenario.coalesce), func(b *testing.B) {
			repository, _ := fixture(b)
			ctx := context.Background()
			input := taskInput()
			input.Coalesce = scenario.coalesce
			task, err := repository.Create(ctx, input, nil, nil)
			if err != nil {
				b.Fatal(err)
			}
			slots := make([]int64, scenario.count)
			for index := range slots {
				slots[index] = int64(index+1) * 60
			}
			expected := scenario.count
			if scenario.coalesce {
				expected = 1
			}
			b.ReportAllocs()
			for b.Loop() {
				if count, err := repository.Enqueue(ctx, *task, slots); err != nil || count != expected {
					b.Fatal("catchup enqueue changed", count, expected, err)
				}
				b.StopTimer()
				if err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
					_, err := connection.ExecContext(ctx, "DELETE FROM scheduled_task_runs WHERE task_id=?", task.Id)
					return err
				}); err != nil {
					b.Fatal(err)
				}
				b.StartTimer()
			}
		})
	}
}
