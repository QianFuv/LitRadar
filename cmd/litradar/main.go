// Command litradar runs the complete Go application and owns process-boundary cleanup.
package main

import (
	"context"
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/signal"
	goRuntime "runtime"
	"syscall"

	"github.com/QianFuv/LitRadar/internal/cli"
	"github.com/QianFuv/LitRadar/internal/runtime"
	"github.com/QianFuv/LitRadar/internal/runtime/observability"
)

func main() { os.Exit(run(os.Args[1:])) }

type terminationSignal struct{ signal os.Signal }

func (termination terminationSignal) Error() string { return "service termination requested" }
func (termination terminationSignal) SignalName() string {
	if goRuntime.GOOS == "windows" {
		return "interrupt"
	}
	if termination.signal == syscall.SIGTERM {
		return "sigterm"
	}
	return "sigint"
}

func run(args []string) (exitCode int) {
	slog.SetDefault(slog.New(slog.NewTextHandler(io.Discard, nil)))
	storage, err := runtime.ProcessStorage(args)
	if err != nil {
		fmt.Fprintln(os.Stderr, "LitRadar runtime logging settings are unavailable")
		return 1
	}
	guard, logger, err := observability.Initialize(context.Background(), storage.AuthDbPath, os.Stderr)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		runtime.CleanupAfterProcess(args)
		return 1
	}
	slog.SetDefault(logger)
	defer guard.Shutdown()
	defer runtime.CleanupAfterProcess(args)
	ctx, cancel := context.WithCancelCause(context.Background())
	defer cancel(nil)
	if cli.IsServiceCommand(args) {
		signals := make(chan os.Signal, 1)
		signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
		defer signal.Stop(signals)
		go func() {
			select {
			case received := <-signals:
				cancel(terminationSignal{received})
			case <-ctx.Done():
			}
		}()
	}
	defer func() {
		if recover() != nil {
			observability.ReportPanic(ctx)
			exitCode = 1
		}
	}()
	executable, err := os.Executable()
	if err != nil {
		return 1
	}
	if err := cli.Run(ctx, args, executable, os.Stdin, os.Stdout); err != nil {
		return 1
	}
	return 0
}
