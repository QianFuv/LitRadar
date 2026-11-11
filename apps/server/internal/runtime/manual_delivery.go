package runtime

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"strconv"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/process"
	"github.com/QianFuv/LitRadar/apps/server/internal/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
)

const cooperativeGrace = 250 * time.Millisecond

type manualChild struct {
	runID       int64
	ownerID     string
	child       *process.Child
	stopKind    string
	stopStarted time.Time
}

type manualDispatcher struct {
	configuration Config
	repository    *delivery.Repository
	children      []*manualChild
}

func (prepared *Prepared) runManualDispatcher(ctx context.Context) (result error) {
	concurrency, err := settings.New(prepared.services.Auth, prepared.services.Codec).DeliveryWorkerConcurrency(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return nil
		}
		return errors.New("delivery dispatcher setting load failed")
	}
	dispatcher := manualDispatcher{configuration: prepared.configuration, repository: prepared.services.Delivery}
	defer func() { result = errors.Join(result, dispatcher.shutdown()) }()
	for ctx.Err() == nil {
		if err := dispatcher.reap(); err != nil {
			return err
		}
		if err := dispatcher.enforceStops(); err != nil {
			return err
		}
		if err := dispatcher.dispatch(concurrency); err != nil {
			return err
		}
		if !waitDuration(ctx, 100*time.Millisecond) {
			return nil
		}
	}
	return nil
}

func (dispatcher *manualDispatcher) reap() error {
	for index := 0; index < len(dispatcher.children); {
		active := dispatcher.children[index]
		isDone, exitError := active.child.Poll()
		if !isDone {
			index++
			continue
		}
		if err := active.child.Close(); err != nil {
			return errors.New("delivery child wait failed")
		}
		outcome := "success"
		if exitError != nil {
			outcome = "failure"
		}
		slog.Info("delivery.dispatcher.child_completed", "event", "delivery.dispatcher.child_completed", "component", "delivery_dispatcher", "outcome", outcome, "delivery_run_id", active.runID, "exit_success", exitError == nil)
		dispatcher.remove(index)
	}
	return nil
}

func (dispatcher *manualDispatcher) remove(index int) {
	last := len(dispatcher.children) - 1
	dispatcher.children[index] = dispatcher.children[last]
	dispatcher.children[last] = nil
	dispatcher.children = dispatcher.children[:last]
}

func (dispatcher *manualDispatcher) dispatch(concurrency int) error {
	available := max(0, concurrency-len(dispatcher.children))
	if available == 0 {
		return nil
	}
	now := unixTime()
	candidates, err := dispatcher.repository.ListDispatchableManualRuns(context.Background(), now, available)
	if err != nil {
		return errors.New("delivery dispatch query failed")
	}
	for _, candidate := range candidates {
		isActive := dispatcher.hasActiveManualRun(candidate.ID)
		if isActive {
			continue
		}
		isExpired, err := dispatcher.expireQueuedManualCandidate(candidate, now)
		if err != nil {
			return err
		}
		if isExpired {
			continue
		}
		active, err := dispatcher.spawn(candidate.ID)
		if err == nil {
			dispatcher.children = append(dispatcher.children, active)
			continue
		}
		slog.Error("delivery.dispatcher.spawn_failed", "event", "delivery.dispatcher.spawn_failed", "component", "delivery_dispatcher", "outcome", "failure", "error_kind", "spawn_or_assign_failed", "delivery_run_id", candidate.ID)
		if candidate.Status == delivery.RunStatusQueued {
			code := "spawn_or_assign_failed"
			if _, err := dispatcher.repository.FinalizeQueuedRun(context.Background(), candidate.ID, candidate.Revision, delivery.RunStatusFailed, nil, &code, unixTime()); err != nil && !errors.Is(err, delivery.ErrConflict) {
				return errors.New("delivery terminal state persistence failed")
			}
		}
	}
	return nil
}

func (dispatcher *manualDispatcher) spawn(runID int64) (*manualChild, error) {
	var random [16]byte
	if _, err := rand.Read(random[:]); err != nil {
		return nil, err
	}
	owner := "manual-worker-" + hex.EncodeToString(random[:])
	configuration := dispatcher.configuration
	arguments := []string{"delivery-run", "--project-root", configuration.Storage.ProjectRoot, "--auth-db", configuration.Storage.AuthDBPath, "--secret-key-file", configuration.SecretKeyFile, "--run-id", strconv.FormatInt(runID, 10), "--owner-id", owner, scheduler.ParentRunIDArgument, fmt.Sprintf("manual-delivery-%d", runID)}
	child, err := process.Start(context.Background(), process.Config{Path: configuration.Executable, Args: arguments, InheritStdin: true, InheritStdout: true, InheritStderr: true})
	if err != nil {
		return nil, err
	}
	return &manualChild{runID: runID, ownerID: owner, child: child}, nil
}

func (dispatcher *manualDispatcher) enforceStops() error {
	now := unixTime()
	for index := 0; index < len(dispatcher.children); {
		active := dispatcher.children[index]
		run, err := dispatcher.repository.LoadRun(context.Background(), active.runID)
		if err != nil {
			return errors.New("delivery cancellation query failed")
		}
		beginManualStop(active, run, now)
		if active.stopKind == "" || active.stopKind != "deadline" && time.Since(active.stopStarted) < cooperativeGrace {
			index++
			continue
		}
		if err := dispatcher.terminateManualChild(active); err != nil {
			return err
		}
		dispatcher.remove(index)
	}
	return nil
}

func (dispatcher *manualDispatcher) finalizeForced(active *manualChild) error {
	run, err := dispatcher.repository.LoadRun(context.Background(), active.runID)
	if err != nil {
		return errors.New("delivery forced-stop query failed")
	}
	if run == nil || run.Status.IsTerminal() {
		return nil
	}
	if run.Status == delivery.RunStatusQueued {
		if active.stopKind == "deadline" {
			code := "deadline_exceeded"
			if _, err := dispatcher.repository.FinalizeQueuedRun(context.Background(), run.ID, run.Revision, delivery.RunStatusTimedOut, nil, &code, unixTime()); err != nil && !errors.Is(err, delivery.ErrConflict) {
				return errors.New("delivery terminal state persistence failed")
			}
		}
		return nil
	}
	if run.OwnerID == nil || *run.OwnerID != active.ownerID {
		return nil
	}
	kind := active.stopKind
	if kind == "" {
		kind = "termination"
	}
	code := "forced_" + kind + "_unknown"
	if _, err := dispatcher.repository.FinalizeRun(context.Background(), run.ID, active.ownerID, run.Revision, delivery.RunStatusUnknown, nil, &code, unixTime()); err != nil && !errors.Is(err, delivery.ErrConflict) {
		return errors.New("delivery terminal state persistence failed")
	}
	return nil
}

func (dispatcher *manualDispatcher) shutdown() error {
	dispatcher.requestManualShutdown()
	deadline := time.Now().Add(cooperativeGrace)
	result := dispatcher.reapManualShutdown(deadline)
	for _, active := range dispatcher.children {
		if _, err := active.child.Terminate(cooperativeGrace); err != nil {
			result = errors.Join(result, errors.New("delivery child termination failed"))
			continue
		}
		result = errors.Join(result, dispatcher.finalizeForced(active))
	}
	dispatcher.children = nil
	return result
}

// hasActiveManualRun retains a full scan of current children before spawning a candidate.
func (dispatcher *manualDispatcher) hasActiveManualRun(runID int64) bool {
	isActive := false
	for _, active := range dispatcher.children {
		isActive = isActive || active.runID == runID
	}
	return isActive
}

// expireQueuedManualCandidate reports persistence failures while allowing concurrent ownership changes.
func (dispatcher *manualDispatcher) expireQueuedManualCandidate(candidate delivery.RunRecord, now float64) (bool, error) {
	if candidate.Status == delivery.RunStatusQueued && candidate.DeadlineAt != nil && *candidate.DeadlineAt <= now {
		code := "deadline_exceeded"
		if _, err := dispatcher.repository.FinalizeQueuedRun(context.Background(), candidate.ID, candidate.Revision, delivery.RunStatusTimedOut, nil, &code, now); err != nil && !errors.Is(err, delivery.ErrConflict) {
			return true, errors.New("delivery terminal state persistence failed")
		}
		return true, nil
	}
	return false, nil
}

// beginManualStop preserves deadline priority and the original stop-start timestamp updates.
func beginManualStop(active *manualChild, run *delivery.RunRecord, now float64) {
	if run != nil && active.stopKind == "" {
		switch {
		case run.DeadlineAt != nil && *run.DeadlineAt <= now:
			active.stopKind = "deadline"
		case run.CancellationRequested:
			active.stopKind = "cancellation"
		case run.Status.IsTerminal():
			active.stopKind = "termination"
		}
		active.stopStarted = time.Now()
	}
}

// terminateManualChild joins termination before reloading fenced durable finalization authority.
func (dispatcher *manualDispatcher) terminateManualChild(active *manualChild) error {
	grace := cooperativeGrace
	if active.stopKind == "deadline" {
		grace = 0
	}
	if _, err := active.child.Terminate(grace); err != nil {
		return errors.New("delivery child termination failed")
	}
	if err := dispatcher.finalizeForced(active); err != nil {
		return err
	}
	return nil
}

// requestManualShutdown requests cancellation for every owned child before any wait or reap.
func (dispatcher *manualDispatcher) requestManualShutdown() {
	for _, active := range dispatcher.children {
		run, err := dispatcher.repository.LoadRun(context.Background(), active.runID)
		if err == nil && run != nil && !run.Status.IsTerminal() && run.OwnerID != nil && *run.OwnerID == active.ownerID {
			_, _ = dispatcher.repository.CancelRun(context.Background(), run.ID, run.Revision, unixTime())
		}
		active.stopKind, active.stopStarted = "shutdown", time.Now()
	}
}

// reapManualShutdown shares one cooperative deadline across all tracked children.
func (dispatcher *manualDispatcher) reapManualShutdown(deadline time.Time) error {
	var result error
	for len(dispatcher.children) > 0 && time.Now().Before(deadline) {
		if err := dispatcher.reap(); err != nil {
			result = errors.Join(result, err)
			break
		}
		if len(dispatcher.children) > 0 {
			time.Sleep(10 * time.Millisecond)
		}
	}
	return result
}
