package runtime

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"os"
	"time"

	"github.com/QianFuv/LitRadar/internal/runtime/observability"

	"github.com/QianFuv/LitRadar/internal/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	store "github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func (prepared *Prepared) runScheduler(ctx context.Context) (result error) {
	worker := fmt.Sprintf("worker-%d-%d", os.Getpid(), time.Now().UnixNano())
	ctx = observability.StartSpan(ctx, "litradar::runtime", "scheduler.loop", map[string]any{"component": "scheduler", "worker_id": worker})
	processes := scheduler.ProcessConfig{ProjectRoot: prepared.configuration.Storage.ProjectRoot, AuthDatabase: prepared.configuration.Storage.AuthDbPath, Executable: prepared.configuration.Executable, SecretKeyFile: prepared.configuration.SecretKeyFile}
	claimContext, cancel := context.WithCancel(ctx)
	defer cancel()
	completed := make(chan error, 4)
	active := 0
	executed := 0
	defer func() {
		cancel()
		for active > 0 {
			result = errors.Join(result, <-completed)
			active--
		}
	}()
	for ctx.Err() == nil {
		started := time.Now()
		tick, claims, err := scheduler.PrepareRuns(observability.CaptureCurrent(ctx), prepared.services.Scheduler, worker, uint64(4-active))
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			slog.ErrorContext(ctx, "scheduler.tick.failed", "event", "scheduler.tick.failed", "component", "scheduler", "worker_id", worker, "outcome", "failure", "error_kind", "execution_error", "duration_ms", time.Since(started).Milliseconds())
			return err
		}
		for _, claim := range claims {
			active++
			go prepared.runSchedulerClaim(claimContext, processes, claim, completed)
		}
		reportSchedulerTick(ctx, tick, worker, executed, started)
		executed = 0
		if err := prepared.waitSchedulerTick(ctx, completed, worker, started, &active, &executed); err != nil {
			return err
		}
	}
	return nil
}

func waitSeconds(ctx context.Context, seconds uint64) {
	const maximumSeconds = uint64(math.MaxInt64 / int64(time.Second))
	for seconds > 0 {
		chunk := min(seconds, maximumSeconds)
		if !waitDuration(ctx, time.Duration(chunk)*time.Second) {
			return
		}
		seconds -= chunk
	}
}

func (prepared *Prepared) runAuditRetention(ctx context.Context) error {
	repository := settings.New(prepared.services.Auth, prepared.services.Codec)
	hasBacklog := false
	for ctx.Err() == nil {
		started := time.Now()
		days, err := repository.AuditRetentionDays(context.Background())
		errorKind := "setting_error"
		if err != nil {
			auth.ReportAuditFailure("retention_setting")
		} else {
			result, cleanupError := prepared.services.Auth.CleanupAudit(context.Background(), days, unixTime())
			err, errorKind = cleanupError, "persistence_error"
			if err == nil {
				hasBacklog = result.HasMoreExpired
				reportAuditRetention(ctx, result, started)
			}
		}
		if err != nil {
			slog.ErrorContext(ctx, "audit.retention.failed", "event", "audit.retention.failed", "component", "security", "outcome", "failure", "error_kind", errorKind, "duration_ms", time.Since(started).Milliseconds())
		}
		interval := 24 * time.Hour
		if hasBacklog {
			interval = time.Minute
		}
		if !waitDuration(ctx, interval) {
			return nil
		}
	}
	return nil
}

// runSchedulerClaim reports exactly one result after execution or a redacted claim panic.
func (prepared *Prepared) runSchedulerClaim(claimContext context.Context, processes scheduler.ProcessConfig, claim store.Claim, completed chan<- error) {
	var result error
	defer func() {
		if recover() != nil {
			observability.ReportPanic(observability.CaptureCurrent(claimContext))
			result = errors.New("scheduler claim task failed")
		}
		completed <- result
	}()
	_, result = scheduler.RunClaim(observability.CaptureCurrent(claimContext), prepared.services.Scheduler, processes, claim)
}

// reportSchedulerTick preserves empty-tick log level and all evaluated counters.
func reportSchedulerTick(ctx context.Context, tick scheduler.TickResult, worker string, executed int, started time.Time) {
	level := slog.LevelInfo
	if tick.Due == 0 && len(tick.Skipped) == 0 && tick.Claimed == 0 {
		level = slog.LevelDebug
	}
	slog.Log(ctx, level, "scheduler.tick.completed", "event", "scheduler.tick.completed", "component", "scheduler", "worker_id", worker, "outcome", "success", "minute_epoch", tick.MinuteEpoch, "jobs", tick.Jobs, "skipped", len(tick.Skipped), "due", tick.Due, "already_executed", tick.AlreadyExecuted, "queued", tick.Queued, "claimed", tick.Claimed, "executed", executed, "duration_ms", time.Since(started).Milliseconds())
}

// waitSchedulerTick updates caller-owned counters and joins its delay task before every early return.
func (prepared *Prepared) waitSchedulerTick(ctx context.Context, completed <-chan error, worker string, started time.Time, active, executed *int) error {
	delayContext, cancelDelay := context.WithCancel(ctx)
	nextTick := make(chan struct{})
	go func() { waitSeconds(delayContext, prepared.configuration.SchedulerIntervalSeconds); close(nextTick) }()
	isDue := false
	for !isDue {
		select {
		case <-ctx.Done():
			cancelDelay()
			<-nextTick
			return nil
		case err := <-completed:
			(*active)--
			(*executed)++
			if err != nil {
				slog.ErrorContext(ctx, "scheduler.tick.failed", "event", "scheduler.tick.failed", "component", "scheduler", "worker_id", worker, "outcome", "failure", "error_kind", "execution_error", "duration_ms", time.Since(started).Milliseconds())
				cancelDelay()
				<-nextTick
				return err
			}
		case <-nextTick:
			isDue = true
		}
	}
	cancelDelay()
	return nil
}

// reportAuditRetention preserves completed versus daily-window-skipped fields after successful cleanup.
func reportAuditRetention(ctx context.Context, result auth.RetentionResult, started time.Time) {
	if result.DidRun {
		slog.InfoContext(ctx, "audit.retention.completed", "event", "audit.retention.completed", "component", "security", "outcome", "success", "deleted_count", result.DeletedCount, "has_more_expired", result.HasMoreExpired, "cutoff", result.Cutoff, "duration_ms", time.Since(started).Milliseconds())
	} else {
		slog.DebugContext(ctx, "audit.retention.skipped", "event", "audit.retention.skipped", "component", "security", "outcome", "success", "reason", "daily_window_not_due", "cutoff", result.Cutoff, "duration_ms", time.Since(started).Milliseconds())
	}
}
