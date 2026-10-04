package scheduler

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/platform/process"
	store "github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const ParentRunIdArgument = "--litradar-parent-run-id"
const processHeartbeatInterval = 5 * time.Second
const processPollInterval = 25 * time.Millisecond
const processTerminationGrace = 250 * time.Millisecond

// ProcessConfig selects the installed application and deployment paths passed explicitly to children.
type ProcessConfig struct{ ProjectRoot, AuthDatabase, Executable, SecretKeyFile string }
type scheduledProcess struct {
	command, path string
	arguments     []string
}

func (config ProcessConfig) processes(job domain.Job) ([]scheduledProcess, error) {
	if err := job.Validate(); err != nil {
		return nil, err
	}
	auth := []string{"--project-root", config.ProjectRoot, "--auth-db", config.AuthDatabase, "--secret-key-file", config.SecretKeyFile}
	delivery := func(command string, job domain.Job) scheduledProcess {
		args := append([]string{command}, auth...)
		args = append(args, "--no-dry-run")
		if job.Database != nil {
			args = append(args, "--db", *job.Database)
		}
		if job.MaxCandidates != nil {
			args = append(args, "--max-candidates", strconv.FormatUint(*job.MaxCandidates, 10))
		}
		return scheduledProcess{command, config.Executable, args}
	}
	if job.Kind != "index" {
		return []scheduledProcess{delivery(job.Kind, job)}, nil
	}
	args := append([]string{"index"}, auth...)
	args = append(args, "--update")
	if job.MetadataFile != nil {
		args = append(args, "--file", *job.MetadataFile)
	}
	result := []scheduledProcess{{"index", config.Executable, args}}
	if job.Notify {
		result = append(result, delivery("notify", domain.Job{}))
	}
	if job.Push {
		result = append(result, delivery("push", domain.Job{}))
	}
	return result, nil
}

func (config ProcessConfig) run(ctx context.Context, task domain.Task, claim store.Claim, heartbeat func() bool) processResult {
	if task.Job == nil {
		return processResult{domain.Error, "Legacy task requires a typed job"}
	}
	started := time.Now()
	fields := []any{"worker_id", claim.WorkerId, "task_id", task.Id, "run_id", fmt.Sprint(claim.RunId), "job_id", jobId(task.Id), "job_kind", task.Job.Kind}
	slog.InfoContext(ctx, "scheduler.run.started", append(fields, "event", "scheduler.run.started", "component", "scheduler", "outcome", "started")...)
	execution := config.runProcesses(ctx, task, claim, heartbeat)
	emitTerminal(ctx, "scheduler.run", execution.status, started, fields)
	return execution
}

func (config ProcessConfig) runProcesses(ctx context.Context, task domain.Task, claim store.Claim, heartbeat func() bool) processResult {
	commands, err := config.processes(*task.Job)
	if err != nil {
		return processResult{domain.Error, "scheduler: invalid job"}
	}
	deadline := time.Now().Add(time.Duration(task.TimeoutSeconds) * time.Second)
	summaries := []string{}
	for ordinal, command := range commands {
		if ctx.Err() != nil {
			summaries = append(summaries, "scheduler: cancelled")
			return processResult{domain.Cancelled, boundedSummary(strings.Join(summaries, "\n"))}
		}
		execution := executeProcess(ctx, command, claim, ordinal+1, deadline, processHeartbeatInterval, heartbeat)
		if execution.summary != "" {
			summaries = append(summaries, execution.summary)
		}
		if execution.status != domain.Success {
			return processResult{execution.status, boundedSummary(strings.Join(summaries, "\n"))}
		}
	}
	return processResult{domain.Success, boundedSummary(strings.Join(summaries, "\n"))}
}

func executeProcess(ctx context.Context, command scheduledProcess, claim store.Claim, ordinal int, deadline time.Time, heartbeatInterval time.Duration, heartbeat func() bool) (result processResult) {
	started := time.Now()
	fields := []any{"component", "scheduler", "worker_id", claim.WorkerId, "task_id", claim.Task.Id, "run_id", fmt.Sprint(claim.RunId), "job_id", jobId(claim.Task.Id), "command", command.command, "process_number", ordinal}
	slog.InfoContext(ctx, "scheduler.child.started", append(fields, "event", "scheduler.child.started", "outcome", "started")...)
	errorKind := "none"
	var exitCode *int
	defer func() {
		event, outcome, level := "scheduler.child.completed", "success", slog.LevelInfo
		if result.status != domain.Success {
			event, outcome, level = "scheduler.child.failed", "failure", slog.LevelWarn
		}
		terminalFields := append(fields, "event", event, "outcome", outcome, "status", result.status, "duration_ms", time.Since(started).Milliseconds())
		if errorKind != "none" {
			terminalFields = append(terminalFields, "error_kind", errorKind)
		}
		if exitCode != nil {
			terminalFields = append(terminalFields, "exit_code", *exitCode)
		}
		slog.Log(ctx, level, event, terminalFields...)
	}()
	if ctx.Err() != nil {
		errorKind = "cancelled"
		return processResult{domain.Cancelled, command.command + ": cancelled"}
	}
	args := append(append([]string{}, command.arguments...), ParentRunIdArgument, strconv.FormatInt(claim.RunId, 10))
	child, err := process.Start(context.Background(), process.Config{Path: command.path, Args: args, OutputLimit: 2048, InheritStderr: true, InheritStdin: true})
	if err != nil {
		errorKind = "spawn_or_assign_failed"
		return processResult{domain.Error, command.command + ": process supervision failed (" + errorKind + ")"}
	}
	defer child.Close()
	lastHeartbeat := time.Now()
	result.status = domain.Success
	terminate := func(status domain.State, kind, suffix string) {
		result.status = status
		errorKind = kind
		result.summary = command.command + suffix
		if _, err := child.Terminate(processTerminationGrace); err != nil {
			result.status = domain.Error
			errorKind = process.ErrorKind(err)
			result.summary = command.command + ": process supervision failed (" + errorKind + ")"
		}
	}
	for {
		if ctx.Err() != nil {
			terminate(domain.Cancelled, "cancelled", ": cancelled")
			break
		}
		hasExited, waitError := child.Poll()
		if hasExited {
			if waitError != nil {
				var failure *exec.ExitError
				if errors.As(waitError, &failure) {
					result.status = domain.Failed
					errorKind = "nonzero_exit"
					code := failure.ExitCode()
					hasCode := code != -1 || runtime.GOOS == "windows"
					if runtime.GOOS == "windows" {
						code = int(int32(code))
					}
					if !hasCode {
						result.summary = command.command + ": process failed"
					} else {
						exitCode = &code
						result.summary = fmt.Sprintf("%s: exit code %d", command.command, code)
					}
				} else {
					result.status = domain.Error
					errorKind = "wait_failed"
					result.summary = command.command + ": process supervision failed (wait_failed)"
				}
			}
			break
		}
		if !time.Now().Before(deadline) {
			terminate(domain.TimedOut, "timeout", ": timed out")
			break
		}
		if time.Since(lastHeartbeat) >= heartbeatInterval {
			if !heartbeat() {
				terminate(domain.Unknown, "heartbeat_lost", ": heartbeat lost")
				break
			}
			lastHeartbeat = time.Now()
		}
		time.Sleep(processPollInterval)
	}
	if err := child.Close(); err != nil {
		result.status = domain.Error
		errorKind = process.ErrorKind(err)
		result.summary = command.command + ": process supervision failed (" + errorKind + ")"
	}
	stdout, _ := child.Output()
	if len(stdout) > 0 {
		if result.summary != "" {
			result.summary += "\n"
		}
		result.summary += "stdout: " + transport.LossyUtf8(stdout)
	}
	result.summary = boundedSummary(result.summary)
	return result
}

func boundedSummary(value string) string {
	characters := []rune(value)
	return string(characters[:min(len(characters), 4096)])
}
