// Package process owns child process trees, pipe draining and explicit cleanup.
package process

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/exec"
	"sync"
	"time"
)

// Config supplies an executable directly without shell expansion or environment inheritance overrides.
type Config struct {
	Path          string
	Args          []string
	Directory     string
	Environment   []string
	OutputLimit   int
	StreamStdout  bool
	InheritStdout bool
	InheritStderr bool
	InheritStdin  bool
}

type retainedOutput struct {
	mu        sync.Mutex
	bytes     []byte
	limit     int
	truncated bool
}

func (output *retainedOutput) Write(data []byte) (int, error) {
	output.mu.Lock()
	defer output.mu.Unlock()
	remaining := output.limit - len(output.bytes)
	if len(data) > remaining {
		output.truncated = true
	}
	output.bytes = append(output.bytes, data[:min(len(data), remaining)]...)
	return len(data), nil
}

// Child owns a leader and its descendants; leader completion does not release tree ownership.
type Child struct {
	command     *exec.Cmd
	tree        *nativeTree
	Stdin       *os.File
	Stdout      *os.File
	readers     []*os.File
	output      retainedOutput
	diagnostics retainedOutput
	done        chan struct{}
	drained     chan struct{}
	waitError   error
	closeOnce   sync.Once
	closeError  error
	closed      chan struct{}
	termination Termination
}

// Termination distinguishes graceful shutdown, forceful tree cleanup and an already empty tree.
type Termination string

const (
	AlreadyExited Termination = "already_exited"
	Graceful      Termination = "graceful"
	Forced        Termination = "forced"
)

// Start publishes a child only after its native process group or Job contains it.
// Callers must Close even after Wait reports that the leader has exited.
func Start(ctx context.Context, config Config) (*Child, error) {
	return startWithHook(ctx, config, nil)
}

type startHook func(string, *exec.Cmd) error

// startWithHook publishes ownership after assignment and starts all completion observers.
func startWithHook(ctx context.Context, config Config, hook startHook) (*Child, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if config.OutputLimit < 0 {
		return nil, fmt.Errorf("invalid output limit")
	}
	if config.StreamStdout && config.InheritStdout {
		return nil, fmt.Errorf("stdout cannot be both inherited and streamed")
	}
	command := exec.Command(config.Path, config.Args...)
	command.Dir = config.Directory
	command.Env = config.Environment
	pipes, err := openChildPipes()
	if err != nil {
		return nil, err
	}
	configureChildPipes(command, config, pipes)
	tree, err := prepareTree(command)
	if err != nil {
		if err := pipes.inputReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.inputReader.Close", "error", err)
		}
		if err := pipes.inputWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.inputWriter.Close", "error", err)
		}
		if err := pipes.outputReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.outputReader.Close", "error", err)
		}
		if err := pipes.outputWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.outputWriter.Close", "error", err)
		}
		if err := pipes.errorReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.errorReader.Close", "error", err)
		}
		if err := pipes.errorWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.errorWriter.Close", "error", err)
		}
		return nil, fmt.Errorf("spawn_or_assign_failed")
	}
	if err = command.Start(); err == nil {
		err = tree.attach(command, hook)
	}
	if err := pipes.inputReader.Close(); err != nil {
		slog.Warn("cleanup.failed", "operation", "pipes.inputReader.Close", "error", err)
	}
	if err := pipes.outputWriter.Close(); err != nil {
		slog.Warn("cleanup.failed", "operation", "pipes.outputWriter.Close", "error", err)
	}
	if err := pipes.errorWriter.Close(); err != nil {
		slog.Warn("cleanup.failed", "operation", "pipes.errorWriter.Close", "error", err)
	}
	if err != nil {
		cleanupFailedStart(command, tree)
		if err := pipes.inputWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.inputWriter.Close", "error", err)
		}
		if err := pipes.outputReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.outputReader.Close", "error", err)
		}
		if err := pipes.errorReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "pipes.errorReader.Close", "error", err)
		}
		return nil, fmt.Errorf("spawn_or_assign_failed")
	}
	child := &Child{command: command, tree: tree, Stdin: pipes.inputWriter, readers: []*os.File{pipes.outputReader, pipes.errorReader},
		output: retainedOutput{limit: config.OutputLimit}, diagnostics: retainedOutput{limit: config.OutputLimit}, done: make(chan struct{}), drained: make(chan struct{}), closed: make(chan struct{})}
	child.startDraining(config)
	go func() { child.waitError = command.Wait(); close(child.done) }()
	go func() {
		select {
		case <-ctx.Done():
			if err := child.Close(); err != nil {
				slog.Warn("cleanup.failed", "operation", "child.Close", "error", err)
			}
		case <-child.closed:
		}
	}()
	return child, nil
}

// Pid identifies the leader for private lifecycle diagnostics.
func (child *Child) Pid() int { return child.command.Process.Pid }

// Wait waits only for the leader and never waits for descendant-held output handles.
func (child *Child) Wait(ctx context.Context) error {
	select {
	case <-child.done:
		return child.waitError
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Poll observes leader completion without blocking or racing cancellation against an already observed exit.
func (child *Child) Poll() (bool, error) {
	select {
	case <-child.done:
		return true, child.waitError
	default:
		return false, nil
	}
}

// Output returns retained stdout; excess bytes are drained rather than blocking children.
func (child *Child) Output() ([]byte, bool) {
	child.output.mu.Lock()
	defer child.output.mu.Unlock()
	return append([]byte(nil), child.output.bytes...), child.output.truncated
}

// Diagnostics returns retained stderr with its own independent retention limit.
func (child *Child) Diagnostics() ([]byte, bool) {
	child.diagnostics.mu.Lock()
	defer child.diagnostics.mu.Unlock()
	return append([]byte(nil), child.diagnostics.bytes...), child.diagnostics.truncated
}

// Close kills remaining descendants even after leader exit, reaps the leader and releases native handles.
func (child *Child) Close() error {
	_, err := child.Terminate(0)
	return err
}

// Terminate gives Unix descendants a bounded SIGTERM grace period before forced cleanup.
// Windows closes the complete Job forcefully. Concurrent calls share one cleanup result.
func (child *Child) Terminate(grace time.Duration) (Termination, error) {
	child.closeOnce.Do(func() {
		defer close(child.closed)
		if err := child.Stdin.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "child.Stdin.Close", "error", err)
		}
		mode, killError := child.tree.terminate(grace)
		child.termination = mode
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		waitError := child.Wait(ctx)
		var exitError *exec.ExitError
		if errors.As(waitError, &exitError) {
			waitError = nil
		}
		treeError := child.tree.waitEmpty(ctx)
		waitError = errors.Join(waitError, child.waitForDrain(ctx))
		child.closeError = errors.Join(killError, classifiedError("wait_failed", errors.Join(waitError, treeError)), classifiedError("kill_failed", child.tree.close()))
		if child.Stdout != nil {
			if err := child.Stdout.Close(); err != nil {
				slog.Warn("cleanup.failed", "operation", "child.Stdout.Close", "error", err)
			}
		}
	})
	return child.termination, child.closeError
}

// waitForDrain preserves completed cleanup when the deadline ties with output completion.
// Pending readers are closed and joined after the deadline expires.
func (child *Child) waitForDrain(ctx context.Context) error {
	select {
	case <-child.drained:
		return nil
	case <-ctx.Done():
		select {
		case <-child.drained:
			return nil
		default:
		}
		for _, reader := range child.readers {
			if err := reader.Close(); err != nil {
				slog.Warn("cleanup.failed", "operation", "reader.Close", "error", err)
			}
		}
		<-child.drained
		return ctx.Err()
	}
}

// childPipes keeps both endpoints owned until their corresponding startup phase completes.
type childPipes struct {
	inputReader, inputWriter   *os.File
	outputReader, outputWriter *os.File
	errorReader, errorWriter   *os.File
}

// openChildPipes creates stdin, stdout and stderr in order, closing prior endpoints on failure.
func openChildPipes() (*childPipes, error) {
	inputReader, inputWriter, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	outputReader, outputWriter, err := os.Pipe()
	if err != nil {
		if err := inputReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "inputReader.Close", "error", err)
		}
		if err := inputWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "inputWriter.Close", "error", err)
		}
		return nil, err
	}
	errorReader, errorWriter, err := os.Pipe()
	if err != nil {
		if err := inputReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "inputReader.Close", "error", err)
		}
		if err := inputWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "inputWriter.Close", "error", err)
		}
		if err := outputReader.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "outputReader.Close", "error", err)
		}
		if err := outputWriter.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "outputWriter.Close", "error", err)
		}
		return nil, err
	}
	return &childPipes{inputReader: inputReader, inputWriter: inputWriter, outputReader: outputReader, outputWriter: outputWriter, errorReader: errorReader, errorWriter: errorWriter}, nil
}

// configureChildPipes applies inheritance after creating the owned draining endpoints.
func configureChildPipes(command *exec.Cmd, config Config, pipes *childPipes) {
	command.Stdin, command.Stdout, command.Stderr = pipes.inputReader, pipes.outputWriter, pipes.errorWriter
	if config.InheritStdin {
		command.Stdin = os.Stdin
	}
	if config.InheritStdout {
		command.Stdout = os.Stdout
	}
	if config.InheritStderr {
		command.Stderr = os.Stderr
	}
}

// cleanupFailedStart kills and joins a partially assigned leader before parent endpoint closure.
func cleanupFailedStart(command *exec.Cmd, tree *nativeTree) {
	if command.Process != nil {
		if err := tree.kill(); err != nil {
			slog.Warn("cleanup.failed", "operation", "tree.kill", "error", err)
		}
		if err := command.Process.Kill(); err != nil {
			slog.Warn("cleanup.failed", "operation", "command.Process.Kill", "error", err)
		}
		reaped := make(chan struct{})
		go func() {
			if err := command.Wait(); err != nil {
				var exitError *exec.ExitError
				if !errors.As(err, &exitError) {
					slog.Warn("process.reap.failed", "error", err)
				}
			}
			close(reaped)
		}()
		cleanupContext, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		if err := tree.waitEmpty(cleanupContext); err != nil {
			slog.Warn("cleanup.failed", "operation", "tree.waitEmpty", "error", err)
		}
		if err := tree.close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "tree.close", "error", err)
		}
		select {
		case <-reaped:
		case <-cleanupContext.Done():
		}
		cancel()
	} else {
		if err := tree.close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "tree.close", "error", err)
		}
	}
}

// startDraining starts output readers before leader and cancellation observers.
func (child *Child) startDraining(config Config) {
	var draining sync.WaitGroup
	for index, reader := range child.readers {
		if index == 0 && config.StreamStdout {
			child.Stdout = reader
			continue
		}
		destination := &child.output
		if index == 1 {
			destination = &child.diagnostics
		}
		draining.Go(func() {
			_, _ = io.Copy(destination, reader)
			if err := reader.Close(); err != nil {
				slog.Warn("cleanup.failed", "operation", "reader.Close", "error", err)
			}
		})
	}
	go func() { draining.Wait(); close(child.drained) }()
}
