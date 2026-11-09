package observability

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"reflect"
	"testing"
	"time"
)

// countedLogValue detects resolution of overwritten values and ignored empty keys.
type countedLogValue struct{ calls *int }

func (value countedLogValue) LogValue() slog.Value {
	*value.calls++
	return slog.IntValue(*value.calls)
}

// TestHandlerPreservesResolutionAndCompactAttributeOrder covers duplicate and reserved-field collisions.
func TestHandlerPreservesResolutionAndCompactAttributeOrder(t *testing.T) {
	var output bytes.Buffer
	guard, logger, err := New("trace", "compact", &output)
	if err != nil {
		t.Fatal(err)
	}
	calls := 0
	current := logger.Handler().WithAttrs([]slog.Attr{slog.Any("", countedLogValue{&calls}), slog.Any("first", countedLogValue{&calls}), slog.String("event", "explicit")}).WithGroup("ignored")
	record := slog.NewRecord(time.Unix(0, 0), slog.LevelInfo, "fallback", 0)
	record.AddAttrs(slog.Any("first", countedLogValue{&calls}), slog.String("log_target", ""), slog.String("timestamp", "caller"), slog.String("target", "caller"))
	if err := current.Handle(context.Background(), record); err != nil {
		t.Fatal(err)
	}
	guard.Shutdown()
	expected := "1970-01-01T00:00:00.000000Z  INFO : first=2 event=\"explicit\" timestamp=\"caller\" target=\"caller\"\n"
	if calls != 2 || output.String() != expected {
		t.Fatalf("calls=%d output=%q", calls, output.String())
	}
}

// TestHandlerPreservesJsonMetadataAndFilteredEncodingFailures checks admission before serialization.
func TestHandlerPreservesJsonMetadataAndFilteredEncodingFailures(t *testing.T) {
	for _, filter := range []string{"off", "trace"} {
		var output bytes.Buffer
		guard, logger, err := New(filter, "json", &output)
		if err != nil {
			t.Fatal(err)
		}
		record := slog.NewRecord(time.Unix(0, 0), slog.LevelInfo, "fallback", 0)
		record.AddAttrs(slog.Any("event", nil), slog.String("log_target", ""), slog.String("timestamp", "caller"), slog.String("level", "caller"), slog.String("target", "caller"))
		if err := logger.Handler().Handle(context.Background(), record); err != nil {
			t.Fatal(err)
		}
		invalid := slog.NewRecord(time.Unix(0, 0), slog.LevelInfo, "invalid", 0)
		invalid.AddAttrs(slog.Any("invalid", make(chan int)))
		err = logger.Handler().Handle(context.Background(), invalid)
		guard.Shutdown()
		assertJsonAdmission(t, filter, err, output.Bytes())
	}
}

// assertJsonAdmission verifies exact metadata replacement and no partial output after an encoding error.
func assertJsonAdmission(t *testing.T, filter string, err error, output []byte) {
	t.Helper()
	if filter == "off" {
		if err != nil || len(output) != 0 {
			t.Fatal(err, string(output))
		}
		return
	}
	if err == nil || err.Error() != "json: unsupported type: chan int" {
		t.Fatal(err)
	}
	var actual map[string]any
	if err := json.Unmarshal(output, &actual); err != nil {
		t.Fatal(err)
	}
	expected := map[string]any{"event": nil, "timestamp": "1970-01-01T00:00:00.000000Z", "level": "INFO", "target": ""}
	if !reflect.DeepEqual(actual, expected) {
		t.Fatal(actual)
	}
}
