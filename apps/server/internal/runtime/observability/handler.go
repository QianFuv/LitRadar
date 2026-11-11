// Package observability owns process logging, bounded buffering and explicit span scopes.
package observability

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"maps"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/runtime/logfilter"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
)

type scopeKey struct{}

// Guard owns the one bounded lossy queue and flushes after all application work ends.
type Guard struct {
	mutex    sync.RWMutex
	queue    chan []byte
	done     chan struct{}
	isClosed bool
	dropped  atomic.Uint64
	writer   io.Writer
	format   string
	filter   *logfilter.Filter
}

type handler struct {
	guard      *Guard
	attributes []slog.Attr
	delegate   slog.Handler
}

// Initialize reads only startup logging settings and returns an independently owned logger.
func Initialize(ctx context.Context, filename string, writer io.Writer) (*Guard, *slog.Logger, error) {
	stored, err := settings.LoadLogging(ctx, filename)
	if err != nil {
		return nil, nil, errors.New("LitRadar runtime logging settings are unavailable")
	}
	return New(stored.LogFilter, stored.LogFormat, writer)
}

// New compiles persisted filtering before selecting an exact supported output format.
func New(filterText, format string, writer io.Writer) (*Guard, *slog.Logger, error) {
	filter, err := logfilter.Parse(filterText)
	if err != nil {
		return nil, nil, err
	}
	if format != "json" && format != "compact" {
		return nil, nil, errors.New("invalid LitRadar log format")
	}
	guard := &Guard{queue: make(chan []byte, 4096), done: make(chan struct{}), writer: writer, format: format, filter: filter}
	go func() {
		defer close(guard.done)
		for line := range guard.queue {
			_, _ = writer.Write(line)
		}
		guard.reportDropped()
		if flusher, exists := writer.(interface{ Flush() error }); exists {
			_ = flusher.Flush()
		}
	}()
	return guard, slog.New(&handler{guard: guard, delegate: standardHandler(queueWriter{guard}, format)}), nil
}

func (guard *Guard) enqueue(line []byte) {
	guard.mutex.RLock()
	defer guard.mutex.RUnlock()
	if guard.isClosed {
		return
	}
	select {
	case guard.queue <- line:
	default:
		for {
			previous := guard.dropped.Load()
			if previous == ^uint64(0) || guard.dropped.CompareAndSwap(previous, previous+1) {
				break
			}
		}
	}
}

// Shutdown closes admission and permits one second for the writer to drain and report overload loss.
func (guard *Guard) Shutdown() {
	guard.mutex.Lock()
	if guard.isClosed {
		guard.mutex.Unlock()
		return
	}
	guard.isClosed = true
	close(guard.queue)
	guard.mutex.Unlock()
	timer := time.NewTimer(time.Second)
	defer timer.Stop()
	select {
	case <-guard.done:
	case <-timer.C:
	}
}

type queueWriter struct{ guard *Guard }

func (writer queueWriter) Write(data []byte) (int, error) {
	writer.guard.enqueue(slices.Clone(data))
	return len(data), nil
}

func standardHandler(writer io.Writer, format string) slog.Handler {
	options := &slog.HandlerOptions{Level: slog.Level(-8), ReplaceAttr: func(_ []string, attribute slog.Attr) slog.Attr {
		switch attribute.Key {
		case "log_target":
			return slog.Attr{}
		case slog.TimeKey:
			if attribute.Value.Kind() == slog.KindTime {
				return slog.Time(attribute.Key, attribute.Value.Time().UTC())
			}
		case slog.LevelKey:
			if level, ok := attribute.Value.Any().(slog.Level); ok && level < slog.LevelDebug {
				return slog.String(attribute.Key, "TRACE")
			}
		}
		return attribute
	}}
	if format == "json" {
		return slog.NewJSONHandler(writer, options)
	}
	return slog.NewTextHandler(writer, options)
}

func (guard *Guard) reportDropped() {
	if dropped := guard.dropped.Load(); dropped > 0 {
		record := slog.NewRecord(time.Now(), slog.LevelWarn, "logging.events_dropped", 0)
		record.Add("target", "litradar", "event", "logging.events_dropped", "component", "logging", "dropped_count", dropped)
		_ = standardHandler(guard.writer, guard.format).Handle(context.Background(), record)
	}
}

func levelNumber(level slog.Level) int {
	if level >= slog.LevelError {
		return logfilter.Error
	}
	if level >= slog.LevelWarn {
		return logfilter.Warn
	}
	if level >= slog.LevelInfo {
		return logfilter.Info
	}
	if level >= slog.LevelDebug {
		return logfilter.Debug
	}
	return logfilter.Trace
}
func scopeFrom(ctx context.Context) []logfilter.Entry {
	if ctx == nil {
		return nil
	}
	scope, _ := ctx.Value(scopeKey{}).([]logfilter.Entry)
	return scope
}

// StartSpan creates and enters an INFO span on this context without goroutine-local state.
func StartSpan(ctx context.Context, target, name string, fields map[string]any) context.Context {
	current, exists := slog.Default().Handler().(*handler)
	if !exists {
		return ctx
	}
	scope := scopeFrom(ctx)
	values := maps.Clone(fields)
	for key, value := range values {
		values[key] = scalarValue(value)
		if isDisplayField(name, key) && value != nil {
			values[key] = logfilter.DebugValue(fmt.Sprint(value))
		}
	}
	span := current.guard.filter.NewSpan(target, name, logfilter.Info, values, scope)
	if span == nil {
		return ctx
	}
	return context.WithValue(ctx, scopeKey{}, append(slices.Clone(scope), span.Enter()))
}

func isDisplayField(span, field string) bool {
	switch span {
	case "http.request":
		return field == "request_id" || field == "route"
	case "scheduler.loop":
		return field == "worker_id"
	case "scheduler.claim", "scheduler.run", "scheduler.child":
		return field == "worker_id" || field == "run_id" || field == "job_id"
	}
	return false
}

// CaptureCurrent preserves display ancestry while entering only the captured span on a worker.
func CaptureCurrent(ctx context.Context) context.Context {
	scope := slices.Clone(scopeFrom(ctx))
	for index := range scope {
		scope[index].Level = logfilter.Off
	}
	if len(scope) > 0 {
		scope[len(scope)-1] = scope[len(scope)-1].Span.Enter()
	}
	return context.WithValue(ctx, scopeKey{}, scope)
}

func scalarValue(value any) any {
	if value == nil {
		return nil
	}
	if _, isDebug := value.(logfilter.DebugValue); isDebug {
		return value
	}
	reflected := reflect.ValueOf(value)
	switch reflected.Kind() {
	case reflect.String:
		return reflected.String()
	case reflect.Bool:
		return reflected.Bool()
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return reflected.Int()
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return reflected.Uint()
	case reflect.Float32, reflect.Float64:
		return reflected.Float()
	}
	return value
}

// WithoutSpans preserves cancellation and values while starting an independent execution scope.
func WithoutSpans(ctx context.Context) context.Context {
	return context.WithValue(ctx, scopeKey{}, []logfilter.Entry(nil))
}

// RecordSpan updates one declared field without changing the current entry's filter level.
func RecordSpan(ctx context.Context, name string, value any) {
	scope := scopeFrom(ctx)
	if len(scope) > 0 {
		scope[len(scope)-1].Span.Record(name, value)
	}
}

func (output *handler) Enabled(context.Context, slog.Level) bool { return true }
func (output *handler) WithAttrs(attributes []slog.Attr) slog.Handler {
	clone := *output
	attributes = slices.Clone(attributes)
	for index := range attributes {
		attributes[index].Value = attributes[index].Value.Resolve()
	}
	clone.attributes = append(slices.Clone(output.attributes), attributes...)
	clone.delegate = output.delegate.WithAttrs(attributes)
	return &clone
}
func (output *handler) WithGroup(name string) slog.Handler {
	clone := *output
	clone.delegate = output.delegate.WithGroup(name)
	return &clone
}

func (output *handler) Handle(ctx context.Context, record slog.Record) error {
	resolved := slog.NewRecord(record.Time, record.Level, record.Message, record.PC)
	record.Attrs(func(attribute slog.Attr) bool {
		attribute.Value = attribute.Value.Resolve()
		resolved.AddAttrs(attribute)
		return true
	})
	record = resolved
	fields := recordFields(record, output.attributes)
	_, hasEvent := fields["event"]
	if !hasEvent && record.Message != "" {
		fields["event"] = record.Message
	}
	target := originalTarget(record, fields)
	scope := scopeFrom(ctx)
	if !output.guard.filter.Enabled(target, levelNumber(record.Level), fields, scope) {
		return nil
	}
	record = record.Clone()
	if !hasEvent && record.Message != "" {
		record.Add("event", record.Message)
	}
	record.Add("target", target)
	spans := make([]map[string]any, 0, len(scope))
	for _, entry := range scope {
		if entry.Span != nil {
			spans = append(spans, entry.Span.Fields())
		}
	}
	if len(spans) > 0 {
		record.Add("span", spans[len(spans)-1], "spans", spans)
	}
	return output.delegate.Handle(ctx, record)
}

func recordFields(record slog.Record, attributes []slog.Attr) map[string]any {
	fields := map[string]any{}
	add := func(attribute slog.Attr) {
		if attribute.Key != "" {
			fields[attribute.Key] = attribute.Value.Resolve().Any()
		}
	}
	for _, attribute := range attributes {
		add(attribute)
	}
	record.Attrs(func(attribute slog.Attr) bool { add(attribute); return true })
	return fields
}

func originalTarget(record slog.Record, fields map[string]any) string {
	if target, exists := fields["log_target"].(string); exists {
		return target
	}
	frame, _ := runtime.CallersFrames([]uintptr{record.PC}).Next()
	filename := strings.ReplaceAll(frame.File, "\\", "/")
	position := strings.LastIndex(filename, "/internal/")
	if position >= 0 {
		filename = filename[position+10:]
	}
	if target := runtimeFileTarget(filename, fields); target != "" {
		return target
	}
	if target := apiAndIndexFileTarget(filename); target != "" {
		return target
	}
	if target := storageFileTarget(filename); target != "" {
		return target
	}
	if target := sourceAndDeliveryFileTarget(filename); target != "" {
		return target
	}
	for prefix, target := range map[string]string{"cli/": "litradar_cli", "scheduler/": "litradar_worker::scheduler", "sources/cnki_index": "litradar_sources::providers", "sources/scholarly/": "litradar_sources::scholarly", "sources/zjlib/": "litradar_sources::zjlib"} {
		if strings.HasPrefix(filename, prefix) {
			return target
		}
	}
	return "litradar"
}

// runtimeFileTarget retains exact source-file logging categories before prefix fallback.
func runtimeFileTarget(filename string, fields map[string]any) string {
	switch filename {
	case "cli/application.go":
		if strings.HasPrefix(fmt.Sprint(fields["event"]), "process.") {
			return "litradar"
		}
		return "litradar_cli"
	case "runtime/service.go", "runtime/background.go":
		return "litradar::runtime"
	case "runtime/manual_delivery.go":
		return "litradar::manual_delivery"
	case "runtime/cleanup.go":
		return "litradar::sqlite_cleanup"
	case "runtime/prepare.go":
		if fields["context"] == "index_startup" {
			return "litradar_cli"
		}
		return "litradar_api"
	}
	return ""
}

// apiAndIndexFileTarget retains exact source-file logging categories before prefix fallback.
func apiAndIndexFileTarget(filename string) string {
	switch filename {
	case "api/middleware.go":
		return "litradar_api::http_observability"
	case "api/auth.go", "api/auth_body.go":
		return "litradar_api::routes::auth"
	case "api/admin.go":
		return "litradar_api::routes::admin"
	case "api/article_access.go":
		return "litradar_api::article_access"
	case "index/live_execute.go":
		return "litradar_index::stats"
	case "index/live_manifest.go":
		return "litradar_index::live"
	case "cfp/full_text.go":
		return "litradar_worker::cfp::full_text"
	}
	return ""
}

// storageFileTarget retains exact source-file logging categories before prefix fallback.
func storageFileTarget(filename string) string {
	switch filename {
	case "storage/auth/audit.go":
		return "litradar_storage::business::security_audit"
	case "storage/favorites/metadata.go":
		return "litradar_storage::business::favorites"
	case "storage/delivery/legacy.go":
		return "litradar_storage::business::delivery"
	case "storage/maintenance/optimize.go":
		return "litradar_storage::index_maintenance"
	case "storage/migrations/index/events.go":
		return "litradar_storage::migrations"
	}
	return ""
}

// sourceAndDeliveryFileTarget retains exact source-file logging categories before prefix fallback.
func sourceAndDeliveryFileTarget(filename string) string {
	switch filename {
	case "sources/scholarly/crossref_workset_collect.go":
		return "litradar_sources::crossref_workset"
	case "sources/scholarly/index.go", "sources/article_access.go":
		return "litradar_sources::providers"
	case "recommend/ai.go":
		return "litradar_worker::ai"
	case "delivery/pushplus.go":
		return "litradar_worker::pushplus"
	case "delivery/manual.go", "delivery/orchestration.go":
		return "litradar_worker::delivery::orchestration"
	}
	return ""
}
