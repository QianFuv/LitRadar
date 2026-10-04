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

	apidomain "github.com/QianFuv/LitRadar/internal/domain/api"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/runtime/logfilter"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
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
	groups     []string
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
	return guard, slog.New(&handler{guard: guard}), nil
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

func (guard *Guard) reportDropped() {
	if dropped := guard.dropped.Load(); dropped > 0 {
		if guard.format == "compact" {
			_, _ = fmt.Fprintf(guard.writer, "WARN litradar logging.events_dropped component=logging dropped_count=%d\n", dropped)
		} else {
			encoded, _ := domain.EncodeJson(map[string]any{"level": "WARN", "target": "litradar", "event": "logging.events_dropped", "component": "logging", "dropped_count": dropped})
			_, _ = io.WriteString(guard.writer, encoded+"\n")
		}
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
	clone.attributes = append(slices.Clone(output.attributes), attributes...)
	return &clone
}
func (output *handler) WithGroup(name string) slog.Handler {
	clone := *output
	if name != "" {
		clone.groups = append(slices.Clone(output.groups), name)
	}
	return &clone
}

func (output *handler) Handle(ctx context.Context, record slog.Record) error {
	fields := map[string]any{}
	order := []string{}
	add := func(attribute slog.Attr) {
		if attribute.Key != "" {
			if _, exists := fields[attribute.Key]; !exists {
				order = append(order, attribute.Key)
			}
			fields[attribute.Key] = attribute.Value.Resolve().Any()
		}
	}
	for _, attribute := range output.attributes {
		add(attribute)
	}
	record.Attrs(func(attribute slog.Attr) bool { add(attribute); return true })
	if _, exists := fields["event"]; !exists && record.Message != "" {
		fields["event"] = record.Message
		order = append(order, "event")
	}
	target := originalTarget(record, fields)
	delete(fields, "log_target")
	scope := scopeFrom(ctx)
	if !output.guard.filter.Enabled(target, levelNumber(record.Level), fields, scope) {
		return nil
	}
	level := []string{"", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"}[levelNumber(record.Level)]
	if output.guard.format == "compact" {
		output.guard.enqueue([]byte(compact(record.Time, level, target, fields, order, scope)))
		return nil
	}
	payload := maps.Clone(fields)
	payload["timestamp"] = record.Time.UTC().Format("2006-01-02T15:04:05.000000Z")
	payload["level"] = level
	payload["target"] = target
	if len(scope) > 0 {
		spans := make([]map[string]any, 0, len(scope))
		for _, entry := range scope {
			if entry.Span != nil {
				spans = append(spans, entry.Span.Fields())
			}
		}
		if len(spans) > 0 {
			payload["span"] = spans[len(spans)-1]
			payload["spans"] = spans
		}
	}
	encoded, err := domain.EncodeJson(payload)
	if err != nil {
		return err
	}
	output.guard.enqueue([]byte(encoded + "\n"))
	return nil
}

func compact(timestamp time.Time, level, target string, fields map[string]any, order []string, scope []logfilter.Entry) string {
	var text strings.Builder
	fmt.Fprintf(&text, "%s %5s ", timestamp.UTC().Format("2006-01-02T15:04:05.000000Z"), level)
	hasSpan := false
	for _, entry := range scope {
		if entry.Span != nil {
			fmt.Fprintf(&text, "%s:", entry.Span.Fields()["name"])
			hasSpan = true
		}
	}
	if hasSpan {
		text.WriteByte(' ')
	}
	fmt.Fprintf(&text, "%s: ", target)
	writeFields(&text, fields, order)
	for _, entry := range scope {
		if entry.Span == nil {
			continue
		}
		values := entry.Span.Fields()
		name := values["name"].(string)
		delete(values, "name")
		if len(values) > 0 {
			text.WriteByte(' ')
			writeFields(&text, values, spanOrder(name, values))
		}
	}
	text.WriteByte('\n')
	return text.String()
}

func spanOrder(name string, fields map[string]any) []string {
	orders := map[string]string{
		"process":        "component command version process_id parent_run_id",
		"cli.command":    "component command",
		"http.request":   "component request_id method route",
		"scheduler.loop": "component worker_id", "scheduler.tick": "component worker_id",
		"scheduler.claim": "component worker_id task_id run_id job_id",
		"scheduler.run":   "component worker_id task_id run_id job_id job_kind",
		"scheduler.child": "component worker_id task_id run_id job_id command process_number",
		"delivery.manual": "component workflow mode user_id", "delivery.workflow": "component workflow mode user_id",
		"pushplus.delivery": "component provider endpoint", "ai.completion": "component provider endpoint operation",
	}
	order := strings.Fields(orders[name])
	for _, key := range slices.Sorted(maps.Keys(fields)) {
		if !slices.Contains(order, key) {
			order = append(order, key)
		}
	}
	return order
}

func writeFields(output *strings.Builder, fields map[string]any, order []string) {
	hasField := false
	for _, key := range order {
		value, exists := fields[key]
		if !exists || value == nil {
			continue
		}
		if hasField {
			output.WriteByte(' ')
		}
		hasField = true
		if key == "message" {
			fmt.Fprint(output, value)
			continue
		}
		output.WriteString(key)
		output.WriteByte('=')
		if text, isString := value.(string); isString {
			output.WriteString(apidomain.DebugString(text))
		} else {
			fmt.Fprint(output, value)
		}
	}
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
	for prefix, target := range map[string]string{"cli/": "litradar_cli", "scheduler/": "litradar_worker::scheduler", "sources/cnki_index": "litradar_sources::providers", "sources/scholarly/": "litradar_sources::scholarly", "sources/zjlib/": "litradar_sources::zjlib"} {
		if strings.HasPrefix(filename, prefix) {
			return target
		}
	}
	return "litradar"
}
