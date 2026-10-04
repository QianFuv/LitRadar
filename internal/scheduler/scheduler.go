// Package scheduler coordinates durable cron admission and supervised typed application jobs.
package scheduler

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"os"
	"time"

	"github.com/QianFuv/LitRadar/internal/runtime/observability"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/platform/cron"
	store "github.com/QianFuv/LitRadar/internal/storage/scheduler"
)

const HealthWindowSeconds = 90.0
const runLeaseSeconds = 90.0

// Mode preserves dry-run inspection and actual execution as separate operations.
type Mode string

const (
	DryRun  Mode = "dry_run"
	Execute Mode = "execute"
)

// Job is validated runnable metadata with the task-wide single-instance bound.
type Job struct {
	Id             int64      `json:"id"`
	JobId          string     `json:"job_id"`
	Name           string     `json:"name"`
	Job            domain.Job `json:"job"`
	Cron           string     `json:"cron"`
	Timezone       string     `json:"timezone"`
	TimeoutSeconds uint64     `json:"timeout_seconds"`
	Coalesce       bool       `json:"coalesce"`
	MaxInstances   int64      `json:"max_instances"`
}

// SkippedTask explains why an enabled task cannot be admitted.
type SkippedTask struct {
	Id     int64  `json:"id"`
	Name   string `json:"name"`
	Cron   string `json:"cron"`
	Reason string `json:"reason"`
}

// LoadResult separates runnable definitions from enabled invalid definitions.
type LoadResult struct {
	Jobs    []Job         `json:"jobs"`
	Skipped []SkippedTask `json:"skipped"`
}

// TaskExecution reports a durable task result after child cleanup.
type TaskExecution struct {
	TaskId int64        `json:"task_id"`
	JobId  string       `json:"job_id"`
	Name   string       `json:"name"`
	Status domain.State `json:"status"`
}

// TickResult reports the evaluated interval and each distinct admission counter.
type TickResult struct {
	Mode            Mode            `json:"mode"`
	Status          domain.State    `json:"status"`
	MinuteEpoch     int64           `json:"minute_epoch"`
	CheckedFrom     float64         `json:"checked_from"`
	CheckedTo       float64         `json:"checked_to"`
	Jobs            int             `json:"jobs"`
	Skipped         []SkippedTask   `json:"skipped"`
	Due             int             `json:"due"`
	AlreadyExecuted int             `json:"already_executed"`
	Queued          int             `json:"queued"`
	Claimed         int             `json:"claimed"`
	Executed        []TaskExecution `json:"executed"`
}

// ManualOutcome distinguishes inspection, missing tasks, contention and actual execution.
type ManualOutcome struct {
	Found      bool          `json:"found"`
	DidExecute bool          `json:"did_execute"`
	Status     *domain.State `json:"status"`
	Message    *string       `json:"message"`
}

var ErrHeartbeatLost = errors.New("Scheduled run heartbeat was lost")

func currentTime() float64  { return float64(time.Now().UnixNano()) / 1e9 }
func jobId(id int64) string { return fmt.Sprintf("scheduled-task-%d", id) }
func validateTask(task domain.Task) error {
	if _, err := cron.Parse(task.Cron); err != nil {
		return err
	}
	if task.Job == nil {
		return errors.New("Legacy task requires a typed job")
	}
	if err := task.Job.Validate(); err != nil {
		return err
	}
	return domain.ValidateTiming(task.Timezone, task.TimeoutSeconds)
}

// LoadJobs validates enabled task definitions without creating runs or launching children.
func LoadJobs(ctx context.Context, repository *store.Repository) (LoadResult, error) {
	result := LoadResult{Jobs: []Job{}, Skipped: []SkippedTask{}}
	tasks, err := repository.List(ctx)
	if err != nil {
		return result, err
	}
	for _, task := range tasks {
		if !task.Enabled {
			continue
		}
		if err := validateTask(task); err != nil {
			result.Skipped = append(result.Skipped, SkippedTask{task.Id, task.Name, task.Cron, err.Error()})
			continue
		}
		result.Jobs = append(result.Jobs, Job{task.Id, jobId(task.Id), task.Name, *task.Job, task.Cron, task.Timezone, task.TimeoutSeconds, task.Coalesce, 1})
	}
	return result, nil
}

// PrepareRuns evaluates at most one day of persisted catch-up before claiming available capacity.
func PrepareRuns(ctx context.Context, repository *store.Repository, worker string, capacity uint64) (TickResult, []store.Claim, error) {
	return PrepareRunsAt(ctx, repository, worker, capacity, currentTime())
}

// PrepareRunsAt exposes the wall clock explicitly for recovery and deterministic service integration.
func PrepareRunsAt(ctx context.Context, repository *store.Repository, worker string, capacity uint64, now float64) (TickResult, []store.Claim, error) {
	ctx = observability.StartSpan(ctx, "litradar_worker::scheduler", "scheduler.tick", map[string]any{"component": "scheduler", "worker_id": worker})
	result := TickResult{Mode: Execute, Status: domain.Running, MinuteEpoch: int64(math.Floor(math.Trunc(now) / 60)), CheckedTo: now, Skipped: []SkippedTask{}, Executed: []TaskExecution{}}
	if err := repository.RecordHeartbeat(ctx, worker, now); err != nil {
		return result, nil, err
	}
	previous, err := repository.LastCheckedAt(ctx)
	if err != nil {
		return result, nil, err
	}
	result.CheckedFrom = now - 86400
	if previous != nil && *previous <= now {
		result.CheckedFrom = max(result.CheckedFrom, *previous)
	}
	tasks, err := repository.List(ctx)
	if err != nil {
		return result, nil, err
	}
	for _, task := range tasks {
		if !task.Enabled {
			continue
		}
		if err := validateTask(task); err != nil {
			result.Skipped = append(result.Skipped, SkippedTask{task.Id, task.Name, task.Cron, err.Error()})
			continue
		}
		result.Jobs++
		schedule, err := cron.Parse(task.Cron)
		if err != nil {
			return result, nil, err
		}
		slots, err := schedule.Slots(task.Timezone, max(result.CheckedFrom, task.CreatedAt-0.001), now)
		if err != nil {
			return result, nil, err
		}
		result.Due += len(slots)
		represented := len(slots)
		if task.Coalesce && represented > 0 {
			represented = 1
		}
		inserted, err := repository.Enqueue(ctx, task, slots)
		if err != nil {
			return result, nil, err
		}
		result.Queued += inserted
		result.AlreadyExecuted += max(0, represented-inserted)
	}
	if err := repository.RecordCheck(ctx, now); err != nil {
		return result, nil, err
	}
	claims, err := repository.ClaimReady(ctx, worker, now, runLeaseSeconds, capacity)
	if err != nil {
		return result, nil, err
	}
	result.Claimed = len(claims)
	level := slog.LevelInfo
	if result.Due == 0 && result.Queued == 0 && result.Claimed == 0 && len(result.Skipped) == 0 {
		level = slog.LevelDebug
	}
	slog.Log(ctx, level, "scheduler.tick.prepared", "event", "scheduler.tick.prepared", "component", "scheduler", "worker_id", worker, "outcome", "success", "jobs", result.Jobs, "skipped", len(result.Skipped), "due", result.Due, "already_executed", result.AlreadyExecuted, "queued", result.Queued, "claimed", result.Claimed)
	return result, claims, nil
}

type processResult struct {
	status  domain.State
	summary string
}
type jobRunner func(context.Context, domain.Task, store.Claim, func() bool) processResult

// RunClaim starts a durable claim before spawning, renews ownership and persists only an owned result.
func RunClaim(ctx context.Context, repository *store.Repository, config ProcessConfig, claim store.Claim) (TaskExecution, error) {
	result, _, err := runClaim(ctx, repository, claim, config.run)
	return result, err
}

func runClaim(ctx context.Context, repository *store.Repository, claim store.Claim, runner jobRunner) (TaskExecution, bool, error) {
	ctx = observability.StartSpan(ctx, "litradar_worker::scheduler", "scheduler.claim", map[string]any{"component": "scheduler", "worker_id": claim.WorkerId, "task_id": claim.Task.Id, "run_id": fmt.Sprint(claim.RunId), "job_id": jobId(claim.Task.Id)})
	result := TaskExecution{claim.Task.Id, jobId(claim.Task.Id), claim.Task.Name, domain.Unknown}
	started := time.Now()
	fields := []any{"component", "scheduler", "worker_id", claim.WorkerId, "task_id", claim.Task.Id, "run_id", fmt.Sprint(claim.RunId), "job_id", result.JobId}
	slog.InfoContext(ctx, "scheduler.claim.started", append(fields, "event", "scheduler.claim.started", "outcome", "started")...)
	failure := func(kind string) {
		slog.ErrorContext(ctx, "scheduler.claim.failed", append(fields, "event", "scheduler.claim.failed", "outcome", "failure", "status", "error", "error_kind", kind, "duration_ms", time.Since(started).Milliseconds())...)
	}
	storageContext := context.WithoutCancel(ctx)
	didStart, err := repository.StartRun(storageContext, claim.RunId, claim.WorkerId, currentTime(), runLeaseSeconds)
	if err != nil {
		failure("storage_error")
		return result, false, err
	}
	if !didStart {
		slog.WarnContext(ctx, "scheduler.claim.completed", append(fields, "event", "scheduler.claim.completed", "outcome", "skipped", "status", "unknown", "reason", "claim_unavailable")...)
		return result, false, nil
	}
	var heartbeatError error
	isHeartbeatLost := false
	execution := runner(ctx, claim.Task, claim, func() bool {
		if heartbeatError != nil || isHeartbeatLost {
			return false
		}
		var didRenew bool
		didRenew, heartbeatError = repository.HeartbeatRun(storageContext, claim.RunId, claim.WorkerId, currentTime(), runLeaseSeconds)
		if heartbeatError == nil && !didRenew {
			isHeartbeatLost = true
		}
		return heartbeatError == nil && !isHeartbeatLost
	})
	if heartbeatError != nil {
		failure("heartbeat_error")
		return result, true, heartbeatError
	}
	if isHeartbeatLost {
		failure("heartbeat_lost")
		return result, true, ErrHeartbeatLost
	}
	didFinish, err := repository.FinishRun(storageContext, claim, execution.status, execution.summary, currentTime())
	if err != nil {
		failure("storage_error")
		return result, true, err
	}
	if !didFinish {
		failure("heartbeat_lost")
		return result, true, ErrHeartbeatLost
	}
	result.Status = execution.status
	emitTerminal(ctx, "scheduler.claim", execution.status, started, fields)
	return result, true, nil
}

// RunTaskNow permits explicit execution of disabled typed tasks without consuming a cron slot.
func RunTaskNow(ctx context.Context, repository *store.Repository, config ProcessConfig, id int64, mode Mode) (ManualOutcome, error) {
	return runTaskNow(ctx, repository, id, mode, config.run)
}
func runTaskNow(ctx context.Context, repository *store.Repository, id int64, mode Mode, runner jobRunner) (ManualOutcome, error) {
	task, err := repository.Get(ctx, id)
	if err != nil || task == nil {
		return ManualOutcome{}, err
	}
	outcome := ManualOutcome{Found: true}
	if mode != Execute {
		return outcome, nil
	}
	if task.Job == nil {
		status := domain.Error
		message := "Legacy task requires a typed job"
		outcome.Status = &status
		outcome.Message = &message
		return outcome, nil
	}
	if err := validateTask(*task); err != nil {
		return outcome, err
	}
	worker := fmt.Sprintf("worker-%d-%d", os.Getpid(), time.Now().UnixNano())
	admission, err := repository.ClaimManual(ctx, id, worker, currentTime(), runLeaseSeconds)
	if err != nil {
		return outcome, err
	}
	switch admission.Status {
	case "not_found":
		return ManualOutcome{}, nil
	case "busy":
		message := "Task already has an active run"
		outcome.Message = &message
		return outcome, nil
	}
	execution, didExecute, err := runClaim(ctx, repository, *admission.Claim, runner)
	if err != nil {
		return outcome, err
	}
	outcome.DidExecute = didExecute
	outcome.Status = &execution.Status
	return outcome, nil
}

func emitTerminal(ctx context.Context, prefix string, status domain.State, started time.Time, fields []any) {
	event, outcome, level := prefix+".completed", "success", slog.LevelInfo
	if status != domain.Success {
		event, outcome, level = prefix+".failed", "failure", slog.LevelWarn
		kind := "unknown"
		switch status {
		case domain.Cancelled:
			kind = "cancelled"
		case domain.TimedOut:
			kind = "timeout"
		case domain.Failed:
			kind = "child_failed"
		case domain.Error:
			kind = "execution_error"
		}
		fields = append(fields, "error_kind", kind)
	}
	slog.Log(ctx, level, event, append(fields, "event", event, "component", "scheduler", "outcome", outcome, "status", status, "duration_ms", time.Since(started).Milliseconds())...)
}
