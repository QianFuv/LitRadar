// Package process owns child process trees, pipe draining and explicit cleanup.
package process

import (
	"context"
	"errors"
	"fmt"
	"io"
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
	InheritStderr bool
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

func startWithHook(ctx context.Context, config Config, hook startHook) (*Child, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if config.OutputLimit < 0 {
		return nil, fmt.Errorf("invalid output limit")
	}
	command := exec.Command(config.Path, config.Args...)
	command.Dir = config.Directory
	command.Env = config.Environment
	inputReader, inputWriter, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	outputReader, outputWriter, err := os.Pipe()
	if err != nil {
		inputReader.Close()
		inputWriter.Close()
		return nil, err
	}
	errorReader, errorWriter, err := os.Pipe()
	if err != nil {
		inputReader.Close()
		inputWriter.Close()
		outputReader.Close()
		outputWriter.Close()
		return nil, err
	}
	command.Stdin, command.Stdout, command.Stderr = inputReader, outputWriter, errorWriter
	if config.InheritStderr {
		command.Stderr = os.Stderr
	}
	tree, err := prepareTree(command)
	if err != nil {
		inputReader.Close()
		inputWriter.Close()
		outputReader.Close()
		outputWriter.Close()
		errorReader.Close()
		errorWriter.Close()
		return nil, fmt.Errorf("spawn_or_assign_failed")
	}
	if err = command.Start(); err == nil {
		err = tree.attach(command, hook)
	}
	inputReader.Close()
	outputWriter.Close()
	errorWriter.Close()
	if err != nil {
		if command.Process != nil {
			tree.kill()
			command.Process.Kill()
			reaped := make(chan struct{})
			go func() { command.Wait(); close(reaped) }()
			cleanupContext, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			tree.waitEmpty(cleanupContext)
			tree.close()
			select {
			case <-reaped:
			case <-cleanupContext.Done():
			}
			cancel()
		} else {
			tree.close()
		}
		inputWriter.Close()
		outputReader.Close()
		errorReader.Close()
		return nil, fmt.Errorf("spawn_or_assign_failed")
	}
	child := &Child{command: command, tree: tree, Stdin: inputWriter, readers: []*os.File{outputReader, errorReader},
		output: retainedOutput{limit: config.OutputLimit}, diagnostics: retainedOutput{limit: config.OutputLimit}, done: make(chan struct{}), drained: make(chan struct{}), closed: make(chan struct{})}
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
		draining.Go(func() { _, _ = io.Copy(destination, reader); reader.Close() })
	}
	go func() { draining.Wait(); close(child.drained) }()
	go func() { child.waitError = command.Wait(); close(child.done) }()
	go func() {
		select {
		case <-ctx.Done():
			child.Close()
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
		child.Stdin.Close()
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
		select {
		case <-child.drained:
		case <-ctx.Done():
			waitError = errors.Join(waitError, ctx.Err())
			for _, reader := range child.readers {
				reader.Close()
			}
			<-child.drained
		}
		child.closeError = errors.Join(killError, waitError, treeError, child.tree.close())
		if child.Stdout != nil {
			child.Stdout.Close()
		}
	})
	return child.termination, child.closeError
}
