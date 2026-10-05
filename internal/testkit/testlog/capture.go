// Package testlog captures real application logging for application regression tests.
package testlog

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/runtime/observability"
)

// Capture installs the production handler and restores the previous logger after the test.
func Capture(t *testing.T) (context.Context, func() []map[string]any) {
	t.Helper()
	var output bytes.Buffer
	guard, logger, err := observability.New("trace", "json", &output)
	if err != nil {
		t.Fatal(err)
	}
	previous := slog.Default()
	slog.SetDefault(logger)
	t.Cleanup(func() { guard.Shutdown(); slog.SetDefault(previous) })
	ctx := observability.StartSpan(context.Background(), "litradar_index::live", "index.worker", map[string]any{"run_id": "run-source-correlation", "worker_id": uint64(7)})
	return ctx, func() []map[string]any {
		guard.Shutdown()
		var events []map[string]any
		for _, line := range strings.Split(strings.TrimSpace(output.String()), "\n") {
			if line == "" {
				continue
			}
			var event map[string]any
			if err := json.Unmarshal([]byte(line), &event); err != nil {
				t.Fatal(err)
			}
			events = append(events, event)
		}
		return events
	}
}

// Events selects exact event names without conflating nested span fields.
func Events(all []map[string]any, name string) []map[string]any {
	var selected []map[string]any
	for _, event := range all {
		if event["event"] == name {
			selected = append(selected, event)
		}
	}
	return selected
}

// Require checks exact event fields and rejects missing keys.
func Require(t *testing.T, event map[string]any, fields map[string]any) {
	t.Helper()
	for key, expected := range fields {
		if event[key] != expected {
			t.Fatalf("%s=%v expected=%v event=%v", key, event[key], expected, event)
		}
	}
}

// Private rejects synthetic secrets and locations anywhere in the serialized log records.
func Private(t *testing.T, events []map[string]any, forbidden ...string) {
	t.Helper()
	encoded, err := json.Marshal(events)
	if err != nil {
		t.Fatal(err)
	}
	for _, value := range forbidden {
		if strings.Contains(string(encoded), value) {
			t.Fatalf("logs disclosed %q", value)
		}
	}
}
