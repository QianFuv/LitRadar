package cli

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/maintenance"
)

// TestMaintenanceFailureDiagnostics preserves the public JSON diagnostic in both stderr failure events.
func TestMaintenanceFailureDiagnostics(t *testing.T) {
	previous := slog.Default()
	t.Cleanup(func() { slog.SetDefault(previous) })
	var logs, output bytes.Buffer
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	err := Run(context.Background(), []string{"admin", "index", "optimize-storage", "--project-root", t.TempDir()}, "unused", strings.NewReader(""), &output)
	var failure maintenance.Failure
	if !errors.As(err, &failure) || failure.Code != "confirmation_required" {
		t.Fatal(err)
	}
	var result struct {
		Error struct{ Message string }
	}
	if err := json.Unmarshal(output.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	failures := 0
	decoder := json.NewDecoder(&logs)
	for {
		var event map[string]any
		if err := decoder.Decode(&event); err == io.EOF {
			break
		} else if err != nil {
			t.Fatal(err)
		}
		if event["event"] == "cli.command.failed" || event["event"] == "process.failed" {
			failures++
			if event["diagnostic"] != result.Error.Message || result.Error.Message == "" || event["error_kind"] != "command_failed" {
				t.Fatalf("missing actionable maintenance diagnostic: %+v", event)
			}
		}
	}
	if failures != 2 {
		t.Fatalf("got %d failure events, want 2", failures)
	}
}

// TestCommandFailureDiagnosticsKeepPrivateCausesOutOfLogs preserves existing CLI privacy boundaries.
func TestCommandFailureDiagnosticsKeepPrivateCausesOutOfLogs(t *testing.T) {
	previous := slog.Default()
	t.Cleanup(func() { slog.SetDefault(previous) })
	for _, scenario := range []struct {
		name       string
		failure    error
		diagnostic string
	}{
		{"public_usage", &diagnosticError{cause: errors.New("private-sentinel"), diagnostic: "safe usage guidance"}, "safe usage guidance"},
		{"private_runtime", errors.New("private-sentinel"), ""},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			var logs bytes.Buffer
			slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
			if err := runCommand(context.Background(), "admin", func(context.Context) error { return scenario.failure }); err != scenario.failure {
				t.Fatal(err)
			}
			if strings.Contains(logs.String(), "private-sentinel") {
				t.Fatal("private cause escaped into stderr")
			}
			for line := range strings.SplitSeq(strings.TrimSpace(logs.String()), "\n") {
				var event map[string]any
				if err := json.Unmarshal([]byte(line), &event); err != nil {
					t.Fatal(err)
				}
				if event["event"] == "cli.command.failed" {
					diagnostic, _ := event["diagnostic"].(string)
					if diagnostic != scenario.diagnostic {
						t.Fatalf("unexpected diagnostic: %+v", event)
					}
				}
			}
		})
	}
}
