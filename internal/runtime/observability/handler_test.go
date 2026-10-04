package observability

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/runtime/logfilter"
)

func TestCompactFormatMatchesOriginalObserver(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/runtime/log-compact-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct{ Output string }
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	guard, logger, err := New("trace", "compact", &output)
	if err != nil {
		t.Fatal(err)
	}
	previous := slog.Default()
	slog.SetDefault(logger)
	defer slog.SetDefault(previous)
	logger.Info("outside", "log_target", "litradar", "event", "outside", "component", "runtime")
	ctx := StartSpan(context.Background(), "litradar", "process", map[string]any{"component": "runtime", "command": "admin", "version": "0.1.0", "process_id": uint64(123), "parent_run_id": "parent-1"})
	logger.InfoContext(ctx, "process.completed", "log_target", "litradar", "event", "process.completed", "component", "runtime", "outcome", "success", "duration_ms", logfilter.DebugValue("7"))
	ctx = StartSpan(ctx, "litradar_cli", "cli.command", map[string]any{"component": "cli", "command": "admin"})
	logger.ErrorContext(ctx, "LitRadar process panicked", "log_target", "litradar::observability", "event", "process.panicked", "component", "runtime", "message", "LitRadar process panicked")
	guard.Shutdown()
	actual := regexp.MustCompile(`(?m)^\S+ `).ReplaceAllString(output.String(), "")
	if actual != corpus.Output {
		t.Fatalf("compact differs from original:\n%s\nexpected:\n%s", actual, corpus.Output)
	}
}

func TestContextScopeActivatesTypedFilterAndNeverLeaksToOtherRequests(t *testing.T) {
	var output bytes.Buffer
	guard, logger, err := New("off,[http.request{route=/api/private}]=debug", "json", &output)
	if err != nil {
		t.Fatal(err)
	}
	previous := slog.Default()
	slog.SetDefault(logger)
	defer slog.SetDefault(previous)
	fields := map[string]any{"component": "http", "request_id": "synthetic", "method": "GET", "route": "/api/private"}
	ctx := StartSpan(context.Background(), "litradar_api::http_observability", "http.request", fields)
	fields["route"] = "mutated caller map"
	var workers sync.WaitGroup
	for range 20 {
		workers.Go(func() { logger.DebugContext(ctx, "visible", "event", "visible", "log_target", "other") })
		workers.Go(func() { logger.ErrorContext(context.Background(), "hidden unrelated request", "log_target", "other") })
	}
	workers.Wait()
	guard.Shutdown()
	lines := bytes.Split(bytes.TrimSpace(output.Bytes()), []byte{'\n'})
	if len(lines) != 20 || bytes.Contains(output.Bytes(), []byte("hidden")) {
		t.Fatal("scope leaked", output.String())
	}
	for _, line := range lines {
		var value struct {
			Event, Target string
			Span          struct{ Name, Route string }
			Spans         []map[string]any
		}
		if err := json.Unmarshal(line, &value); err != nil {
			t.Fatal(err)
		}
		if value.Event != "visible" || value.Target != "other" || value.Span.Name != "http.request" || value.Span.Route != "/api/private" || len(value.Spans) != 1 {
			t.Fatal(string(line))
		}
	}
}

func TestCapturedWorkerRetainsAncestryWithoutEnteringAncestorFilter(t *testing.T) {
	var output bytes.Buffer
	guard, logger, err := New("off,[process]=trace,[scheduler.loop]=info", "json", &output)
	if err != nil {
		t.Fatal(err)
	}
	previous := slog.Default()
	slog.SetDefault(logger)
	defer slog.SetDefault(previous)
	ctx := StartSpan(context.Background(), "litradar", "process", map[string]any{"component": "runtime"})
	ctx = StartSpan(ctx, "litradar::runtime", "scheduler.loop", map[string]any{"component": "scheduler", "worker_id": "fixture"})
	logger.DebugContext(ctx, "loop-visible", "log_target", "other")
	worker := CaptureCurrent(ctx)
	logger.DebugContext(worker, "worker-hidden", "log_target", "other")
	logger.InfoContext(worker, "worker-visible", "log_target", "other")
	guard.Shutdown()
	if strings.Contains(output.String(), "worker-hidden") || !strings.Contains(output.String(), "loop-visible") {
		t.Fatal(output.String())
	}
	var event struct{ Spans []map[string]any }
	lines := bytes.Split(bytes.TrimSpace(output.Bytes()), []byte{'\n'})
	if len(lines) != 2 {
		t.Fatal(output.String())
	}
	if err := json.Unmarshal(lines[1], &event); err != nil {
		t.Fatal(err)
	}
	if len(event.Spans) != 2 || event.Spans[0]["name"] != "process" {
		t.Fatal("lost display ancestry", output.String())
	}
}

type gatedWriter struct {
	started, release chan struct{}
	once             sync.Once
	buffer           bytes.Buffer
}

func (writer *gatedWriter) Write(value []byte) (int, error) {
	writer.once.Do(func() { close(writer.started); <-writer.release })
	return writer.buffer.Write(value)
}

func TestLossyQueueKeepsBoundAndReportsExactDropsAfterDrain(t *testing.T) {
	writer := &gatedWriter{started: make(chan struct{}), release: make(chan struct{})}
	guard, logger, err := New("info", "json", writer)
	if err != nil {
		t.Fatal(err)
	}
	logger.Info("first")
	<-writer.started
	for range 4113 {
		logger.Info("queued")
	}
	if guard.dropped.Load() != 17 || len(guard.queue) != 4096 {
		t.Fatal("unbounded or miscounted queue", guard.dropped.Load(), len(guard.queue))
	}
	close(writer.release)
	guard.Shutdown()
	guard.Shutdown()
	select {
	case <-guard.done:
	case <-time.After(10 * time.Second):
		t.Fatal("released writer did not drain")
	}
	lines := bytes.Split(bytes.TrimSpace(writer.buffer.Bytes()), []byte{'\n'})
	if len(lines) != 4098 {
		t.Fatal("buffer did not drain or report duplicated", len(lines))
	}
	if string(lines[len(lines)-1]) != "{\"component\":\"logging\",\"dropped_count\":17,\"event\":\"logging.events_dropped\",\"level\":\"WARN\",\"target\":\"litradar\"}" {
		t.Fatal(string(lines[len(lines)-1]))
	}
}

func TestShutdownRemainsBoundedWhenDroppedWarningWriterIsBlocked(t *testing.T) {
	writer := &gatedWriter{started: make(chan struct{}), release: make(chan struct{})}
	guard, logger, err := New("info", "json", writer)
	if err != nil {
		t.Fatal(err)
	}
	logger.Info("blocked")
	<-writer.started
	for range 4097 {
		logger.Info("queued")
	}
	finished := make(chan struct{})
	go func() { guard.Shutdown(); close(finished) }()
	select {
	case <-finished:
	case <-time.After(3 * time.Second):
		t.Error("shutdown blocked beyond the flush budget while reporting dropped events")
	}
	close(writer.release)
	<-finished
	<-guard.done
}

func TestStartupLoggingUsesReadOnlyDefaultsAndFixedErrors(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "missing", "auth.sqlite")
	t.Setenv("RUST_LOG", "off")
	guard, logger, err := Initialize(context.Background(), filename, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	defer guard.Shutdown()
	if _, err := os.Stat(filepath.Dir(filename)); !os.IsNotExist(err) {
		t.Fatal("logging created storage", err)
	}
	if !logger.Handler().(*handler).guard.filter.Enabled("litradar", 3, nil, nil) {
		t.Fatal("environment changed persisted logging")
	}
	for _, values := range [][3]string{{"[SECRET", "bad", "invalid LitRadar log filter"}, {"info", "json ", "invalid LitRadar log format"}} {
		_, _, err := New(values[0], values[1], io.Discard)
		if err == nil || err.Error() != values[2] || strings.Contains(err.Error(), "SECRET") {
			t.Fatal(err)
		}
	}
}
