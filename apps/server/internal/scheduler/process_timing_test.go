package scheduler

import (
	"context"
	"testing"
	"testing/synctest"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	store "github.com/QianFuv/LitRadar/apps/server/internal/storage/scheduler"
)

// TestSchedulerSharedDeadline uses virtual time to reject per-command timeout resets.
func TestSchedulerSharedDeadline(t *testing.T) {
	for _, scenario := range []struct {
		name     string
		timeout  uint64
		status   domain.State
		commands int
		elapsed  time.Duration
	}{
		{"success", 30, domain.Success, 3, 24 * time.Second},
		{"deadline", 15, domain.TimedOut, 2, 15 * time.Second},
		{"first-failed", 30, domain.Failed, 1, 8 * time.Second},
		{"cancelled", 30, domain.Cancelled, 1, 8 * time.Second},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				started := time.Now()
				deadline := started.Add(time.Duration(scenario.timeout) * time.Second)
				commands := []string{}
				runner := func(ctx context.Context, command scheduledProcess, claim store.Claim, ordinal int, receivedDeadline time.Time, heartbeatInterval time.Duration, heartbeat func() bool) processResult {
					commands = append(commands, command.command)
					if !receivedDeadline.Equal(deadline) || ordinal != len(commands) || claim.RunID != 42 || heartbeatInterval != processHeartbeatInterval || !heartbeat() {
						t.Fatal("command lost its shared execution context", receivedDeadline, deadline, ordinal)
					}
					done := time.NewTimer(8 * time.Second)
					defer done.Stop()
					expired := time.NewTimer(time.Until(receivedDeadline))
					defer expired.Stop()
					select {
					case <-ctx.Done():
						return processResult{domain.Cancelled, "cancelled"}
					case <-expired.C:
						return processResult{domain.TimedOut, "timed out"}
					case <-done.C:
						if scenario.name == "first-failed" {
							return processResult{domain.Failed, "failed"}
						}
						if scenario.name == "cancelled" {
							cancel()
						}
						return processResult{domain.Success, command.command}
					}
				}
				task := domain.Task{Job: &domain.Job{Kind: "index", Notify: true, Push: true}, TimeoutSeconds: scenario.timeout}
				result := (ProcessConfig{}).runProcessesWith(ctx, task, testClaim(), func() bool { return true }, runner)
				if result.status != scenario.status || len(commands) != scenario.commands || time.Since(started) != scenario.elapsed {
					t.Fatal(result, commands, time.Since(started))
				}
				for index, command := range commands {
					if command != []string{"index", "notify", "push"}[index] {
						t.Fatal("command order changed", commands)
					}
				}
			})
		})
	}
}
