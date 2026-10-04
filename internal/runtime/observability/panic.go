package observability

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// ReportPanic emits only fixed classification and source location, never the panic payload.
func ReportPanic(ctx context.Context) {
	file, line := "unknown", 0
	var callers [32]uintptr
	frames := runtime.CallersFrames(callers[:runtime.Callers(2, callers[:])])
	hasPanicFrame := false
	for {
		frame, more := frames.Next()
		if frame.Function == "runtime.gopanic" {
			hasPanicFrame = true
		} else if hasPanicFrame && !strings.HasPrefix(frame.Function, "runtime.") {
			file, line = filepath.Base(frame.File), frame.Line
			break
		}
		if !more {
			break
		}
	}
	fields := map[string]any{"event": "process.panicked", "component": "runtime", "panic_file": file, "panic_line": line, "panic_column": 0, "message": "LitRadar process panicked"}
	current, exists := slog.Default().Handler().(*handler)
	if !exists || !current.guard.filter.Enabled("litradar::observability", 1, fields, scopeFrom(ctx)) {
		fmt.Fprintln(os.Stderr, "LitRadar process panicked")
		return
	}
	slog.ErrorContext(ctx, "LitRadar process panicked", "log_target", "litradar::observability", "event", "process.panicked", "component", "runtime", "panic_file", file, "panic_line", line, "panic_column", 0, "message", "LitRadar process panicked")
}
