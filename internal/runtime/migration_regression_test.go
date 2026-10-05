package runtime

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"log/slog"
	"net"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/delivery"
)

type migrationLogBuffer struct {
	mutex  sync.Mutex
	buffer bytes.Buffer
}

func (buffer *migrationLogBuffer) Write(value []byte) (int, error) {
	buffer.mutex.Lock()
	defer buffer.mutex.Unlock()
	return buffer.buffer.Write(value)
}

func (buffer *migrationLogBuffer) String() string {
	buffer.mutex.Lock()
	defer buffer.mutex.Unlock()
	return buffer.buffer.String()
}

func TestAuditRetentionImmediateBacklogErrorAndDailyDelay(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		prepared, err := prepareResources(context.Background(), runtimeConfiguration(t))
		if err != nil {
			t.Fatal(err)
		}
		defer prepared.Close()
		var output migrationLogBuffer
		previous := slog.Default()
		slog.SetDefault(slog.New(slog.NewJSONHandler(&output, &slog.HandlerOptions{Level: slog.LevelDebug})))
		defer slog.SetDefault(previous)
		execute := func(statement string) {
			t.Helper()
			if err := prepared.services.Auth.Immediate(context.Background(), false, func(connection *sql.Conn) error {
				_, err := connection.ExecContext(context.Background(), statement)
				return err
			}); err != nil {
				t.Fatal(err)
			}
		}
		execute("WITH RECURSIVE sequence(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM sequence WHERE value<10005) INSERT INTO security_audit_events(action,outcome,occurred_at) SELECT 'login','completed',1 FROM sequence")
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()
		finished := make(chan error, 1)
		go func() { finished <- prepared.runAuditRetention(ctx) }()
		synctest.Wait()
		if !strings.Contains(output.String(), `"deleted_count":10000`) {
			t.Fatal(output.String())
		}
		execute("CREATE TRIGGER retention_failure BEFORE DELETE ON security_audit_events BEGIN SELECT RAISE(ABORT,'PRIVATE_DATABASE_SENTINEL'); END")
		time.Sleep(time.Minute)
		synctest.Wait()
		if !strings.Contains(output.String(), `"event":"audit.retention.failed"`) {
			t.Fatal(output.String())
		}
		execute("DROP TRIGGER retention_failure")
		time.Sleep(time.Minute)
		synctest.Wait()
		if !strings.Contains(output.String(), `"deleted_count":5`) {
			t.Fatal(output.String())
		}
		beforeDaily := output.String()
		time.Sleep(24*time.Hour - time.Nanosecond)
		synctest.Wait()
		if output.String() != beforeDaily {
			t.Fatal("daily cleanup ran early")
		}
		time.Sleep(time.Nanosecond)
		synctest.Wait()
		cancel()
		synctest.Wait()
		if err := <-finished; err != nil {
			t.Fatal(err)
		}
		var timestamps []time.Time
		for _, line := range strings.Split(strings.TrimSpace(output.String()), "\n") {
			var event struct {
				Time  time.Time
				Event string
			}
			if err := json.Unmarshal([]byte(line), &event); err != nil {
				t.Fatal(err)
			}
			if strings.HasPrefix(event.Event, "audit.retention.") {
				timestamps = append(timestamps, event.Time)
			}
		}
		if len(timestamps) != 4 {
			t.Fatal(output.String())
		}
		for ordinal, expected := range []time.Duration{time.Minute, time.Minute, 24 * time.Hour} {
			if actual := timestamps[ordinal+1].Sub(timestamps[ordinal]); actual != expected {
				t.Fatalf("delay %d: %v", ordinal, actual)
			}
		}
		if strings.Contains(output.String(), "PRIVATE_DATABASE_SENTINEL") {
			t.Fatal("retention failure leaked database detail")
		}
	})
}

func TestManualDeadlineAndCancellationReapChildBeforeTerminalState(t *testing.T) {
	for _, scenario := range []string{"queued-deadline", "running-deadline", "running-cancellation"} {
		t.Run(scenario, func(t *testing.T) {
			prepared, run := manualFixture(t)
			ctx := context.Background()
			owner := "stop-owner"
			if scenario != "queued-deadline" {
				claimed, err := prepared.services.Delivery.ClaimRun(ctx, run.Id, owner, run.Revision, unixTime(), 300)
				if err != nil {
					t.Fatal(err)
				}
				run, err = prepared.services.Delivery.StartRun(ctx, run.Id, owner, claimed.Run.Revision, unixTime())
				if err != nil {
					t.Fatal(err)
				}
			}
			child, address := runtimeChild(t)
			dispatcher := manualDispatcher{repository: prepared.services.Delivery, children: []*manualChild{{runId: run.Id, ownerId: owner, child: child}}}
			if scenario == "running-cancellation" {
				if _, err := prepared.services.Delivery.CancelRun(ctx, run.Id, run.Revision, unixTime()); err != nil {
					t.Fatal(err)
				}
				if err := dispatcher.enforceStops(); err != nil {
					t.Fatal(err)
				}
				if len(dispatcher.children) != 1 {
					t.Fatal("cooperative grace was skipped")
				}
				if done, _ := child.Poll(); done {
					t.Fatal("child killed before grace")
				}
				time.Sleep(cooperativeGrace + 20*time.Millisecond)
			} else {
				if err := prepared.services.Auth.Immediate(ctx, false, func(connection *sql.Conn) error {
					_, err := connection.ExecContext(ctx, "UPDATE delivery_runs SET created_at=?,deadline_at=? WHERE id=?", unixTime()-10, unixTime()-1, run.Id)
					return err
				}); err != nil {
					t.Fatal(err)
				}
			}
			started := time.Now()
			if err := dispatcher.enforceStops(); err != nil {
				t.Fatal(err)
			}
			if time.Since(started) > 2*time.Second || len(dispatcher.children) != 0 {
				t.Fatal("stop did not reap promptly")
			}
			if connection, err := net.DialTimeout("tcp", address, 100*time.Millisecond); err == nil {
				connection.Close()
				t.Fatal("terminal child remains alive")
			}
			stored, err := dispatcher.repository.LoadRun(ctx, run.Id)
			status, code := delivery.RunStatusUnknown, "forced_deadline_unknown"
			if scenario == "queued-deadline" {
				status, code = delivery.RunStatusTimedOut, "deadline_exceeded"
			}
			if scenario == "running-cancellation" {
				code = "forced_cancellation_unknown"
			}
			if err != nil || stored.Status != status || stored.ErrorCode == nil || *stored.ErrorCode != code {
				t.Fatal(stored, err)
			}
		})
	}
}

func TestSchedulerImmediateTickAndInterruptibleLongDelay(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		configuration := runtimeConfiguration(t)
		configuration.SchedulerIntervalSeconds = 86400
		prepared, err := prepareResources(context.Background(), configuration)
		if err != nil {
			t.Fatal(err)
		}
		defer prepared.Close()
		var output migrationLogBuffer
		previous := slog.Default()
		slog.SetDefault(slog.New(slog.NewJSONHandler(&output, &slog.HandlerOptions{Level: slog.LevelDebug})))
		defer slog.SetDefault(previous)
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()
		finished := make(chan error, 1)
		started := time.Now()
		go func() { finished <- prepared.runScheduler(ctx) }()
		synctest.Wait()
		if count := strings.Count(output.String(), `"event":"scheduler.tick.completed"`); count != 1 {
			t.Fatal(output.String())
		}
		if time.Now() != started {
			t.Fatal("first scheduler tick was delayed")
		}
		cancel()
		synctest.Wait()
		if err := <-finished; err != nil {
			t.Fatal(err)
		}
		if time.Now() != started {
			t.Fatal("shutdown waited for next tick")
		}
	})
}
