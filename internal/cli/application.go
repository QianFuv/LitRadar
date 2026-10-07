package cli

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"slices"
	"strconv"
	"time"

	"github.com/QianFuv/LitRadar"
	"github.com/QianFuv/LitRadar/internal/api"
	"github.com/QianFuv/LitRadar/internal/runtime"
	"github.com/QianFuv/LitRadar/internal/runtime/logfilter"
	"github.com/QianFuv/LitRadar/internal/runtime/observability"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
)

const applicationUsage = "Usage: litradar <COMMAND> [OPTIONS]\n\nCommands:\n  serve      Run HTTP and scheduling as one service\n  admin      Manage administrators, secrets, and backups\n  index      Build or update searchable article indexes\n  cfp        Import or refresh original journal calls for papers\n  notify     Deliver recommendation notifications\n  push       Push tracking updates\n  scheduler  Validate or run scheduled tasks manually\n  openapi    Emit the generated OpenAPI document"
const openapiUsage = "Usage: litradar openapi [--output PATH]"

// IsServiceCommand identifies the service after validating internal correlation arguments.
func IsServiceCommand(values []string) bool {
	args := arguments(slices.Clone(values))
	_, err := parentRunId(&args)
	return err == nil && len(args) > 0 && args[0] == "serve" && !hasHelp(args[1:])
}

// Run dispatches every public and authenticated internal command to Go implementations.
// Logging initialization and process-boundary cleanup belong to the executable host.
func Run(ctx context.Context, values []string, executable string, input io.Reader, output io.Writer) (result error) {
	args := arguments(slices.Clone(values))
	parent, parentError := parentRunId(&args)
	command := commandName(args)
	started := time.Now()
	fields := map[string]any{"component": "runtime", "command": command, "process_id": uint64(os.Getpid()), "version": litradar.Version(), "parent_run_id": nil}
	if parentError == nil && parent != "" {
		fields["parent_run_id"] = parent
	}
	ctx = observability.StartSpan(ctx, "litradar", "process", fields)
	slog.InfoContext(ctx, "process.started", "event", "process.started", "component", "runtime")
	defer func() {
		if recover() != nil {
			observability.ReportPanic(ctx)
			result = errors.New("LitRadar process panicked")
			return
		}
		duration := logfilter.DebugValue(strconv.FormatInt(time.Since(started).Milliseconds(), 10))
		if result != nil {
			slog.ErrorContext(ctx, "process.failed", "event", "process.failed", "component", "runtime", "outcome", "failure", "error_kind", "command_failed", "duration_ms", duration)
		} else {
			slog.InfoContext(ctx, "process.completed", "event", "process.completed", "component", "runtime", "outcome", "success", "duration_ms", duration)
		}
	}()
	if parentError != nil {
		return parentError
	}
	if len(args) == 0 || args[0] == "--help" || args[0] == "-h" {
		_, err := fmt.Fprintln(output, applicationUsage)
		return err
	}
	return dispatchApplicationCommand(ctx, args, parent, executable, input, output)
}

func commandName(args []string) string {
	if len(args) == 0 || args[0] == "--help" || args[0] == "-h" {
		return "help"
	}
	if args[0] == "delivery-run" {
		return "delivery_run"
	}
	if slices.Contains([]string{"serve", "admin", "cfp", "index", "notify", "push", "scheduler", "openapi"}, args[0]) {
		return args[0]
	}
	return "unknown"
}

func runCommand(ctx context.Context, command string, operation func(context.Context) error) error {
	ctx = observability.StartSpan(ctx, "litradar_cli", "cli.command", map[string]any{"component": "cli", "command": command})
	started := time.Now()
	slog.InfoContext(ctx, "cli.command.started", "event", "cli.command.started", "component", "cli", "command", command)
	err := operation(ctx)
	if err != nil {
		slog.ErrorContext(ctx, "cli.command.failed", "event", "cli.command.failed", "component", "cli", "command", command, "outcome", "failure", "error_kind", "command_failed", "duration_ms", time.Since(started).Milliseconds())
	} else {
		slog.InfoContext(ctx, "cli.command.completed", "event", "cli.command.completed", "component", "cli", "command", command, "outcome", "success", "duration_ms", time.Since(started).Milliseconds())
	}
	return err
}

func runOpenapi(values []string, output io.Writer) error {
	if hasHelp(values) {
		_, err := fmt.Fprintln(output, openapiUsage)
		return err
	}
	document, err := api.GenerateOpenAPI()
	if err != nil {
		return err
	}
	if len(values) == 0 {
		_, err := output.Write(document)
		return err
	}
	if len(values) == 2 && values[0] == "--output" {
		return os.WriteFile(values[1], document, 0666)
	}
	return fmt.Errorf("%s", openapiUsage)
}

// dispatchApplicationCommand retains help, service admission and authenticated internal command routing.
func dispatchApplicationCommand(ctx context.Context, args arguments, parent, executable string, input io.Reader, output io.Writer) error {
	tail := args[1:]
	switch args[0] {
	case "serve":
		if hasHelp(tail) {
			_, err := fmt.Fprintln(output, serveUsage)
			return err
		}
		configuration, err := parseServe(tail, executable)
		if err != nil {
			return err
		}
		return runtime.Serve(ctx, configuration)
	case "openapi":
		return runOpenapi(tail, output)
	case "admin", "cfp", "index", "notify", "push", "scheduler", "delivery-run":
		if args[0] == "delivery-run" && parent == "" {
			break
		}
		return runCommand(ctx, args[0], func(ctx context.Context) error {
			return dispatchNamedCommand(ctx, args[0], tail, executable, input, output)
		})
	}
	return fmt.Errorf("unknown LitRadar subcommand: %s\n%s", args[0], applicationUsage)
}

// dispatchNamedCommand invokes exactly one named CLI operation within its original command span.
func dispatchNamedCommand(ctx context.Context, command string, tail []string, executable string, input io.Reader, output io.Writer) error {
	switch command {
	case "admin":
		return runAdmin(ctx, tail, input, output)
	case "cfp":
		return runCfp(ctx, tail, output)
	case "index":
		return runIndex(ctx, tail, executable, output)
	case "notify":
		return runDelivery(ctx, delivery.WorkflowNotify, tail, output)
	case "push":
		return runDelivery(ctx, delivery.WorkflowPush, tail, output)
	case "scheduler":
		return runScheduler(ctx, tail, executable, output)
	default:
		return runManualDelivery(ctx, tail, output)
	}
}
