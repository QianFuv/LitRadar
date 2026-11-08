package main

import (
	"bytes"
	"context"
	"encoding/json"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestExecutableReportsSafeUsageErrors checks actionable diagnostics without exposing argument values.
func TestExecutableReportsSafeUsageErrors(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	binary := buildProcessExecutable(t, ctx)
	const secret = "SyntheticCliSecret!\nforged-log-entry"
	for _, scenario := range []struct {
		name       string
		args       []string
		diagnostic string
	}{
		{"unknown_command", []string{"unknown-command"}, "unknown LitRadar subcommand; run litradar --help"},
		{"unknown_command_secret", []string{secret}, "unknown LitRadar subcommand; run litradar --help"},
		{"zero_retries", []string{"notify", "--retries", "0"}, "--retries must be between 1 and 10"},
		{"excessive_retries", []string{"push", "--retries", "11"}, "--retries must be between 1 and 10"},
		{"invalid_retries", []string{"notify", "--retries", secret}, "--retries must be an integer between 1 and 10"},
		{"missing_retries", []string{"notify", "--retries"}, "--retries must be an integer between 1 and 10"},
		{"runtime_error", []string{"openapi", "--output", filepath.Join(t.TempDir(), "SyntheticCliSecret!", "missing", "openapi.json")}, ""},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			command := exec.CommandContext(ctx, binary, scenario.args...)
			command.Dir = t.TempDir()
			var stdout, stderr bytes.Buffer
			command.Stdout, command.Stderr = &stdout, &stderr
			failure, ok := command.Run().(*exec.ExitError)
			if !ok || failure.ExitCode() != 1 || stdout.Len() != 0 {
				t.Fatalf("expected exit 1 and empty stdout: %v, %q", failure, stdout.String())
			}
			if strings.Contains(stderr.String(), "SyntheticCliSecret!") || strings.Contains(stderr.String(), "forged-log-entry") {
				t.Fatal("stderr exposed untrusted argument or runtime error text")
			}
			failures := 0
			for _, line := range strings.Split(strings.TrimSpace(stderr.String()), "\n") {
				var event map[string]any
				if err := json.Unmarshal([]byte(line), &event); err != nil {
					t.Fatalf("stderr is not JSON Lines: %q", line)
				}
				if event["event"] == "process.failed" {
					failures++
					diagnostic, _ := event["diagnostic"].(string)
					if diagnostic != scenario.diagnostic || event["error_kind"] != "command_failed" {
						t.Fatalf("unexpected failure diagnostic: %v", event)
					}
				}
			}
			if failures != 1 {
				t.Fatalf("expected one process failure, got %d", failures)
			}
		})
	}
}
