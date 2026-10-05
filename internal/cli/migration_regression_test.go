package cli

import (
	"bytes"
	"context"
	"io"
	"log/slog"
	"strings"
	"testing"
)

func TestIndexDefaultsRemainIndependentOfExecutionConcurrency(t *testing.T) {
	for _, values := range [][]string{nil, {"--workers", "5"}, {"--workers", "32", "--processes", "3"}, {"--stop-after", "catalog.csv"}} {
		args := arguments(values)
		configuration, explicit, err := parseIndex(&args)
		if err != nil || explicit || configuration.IssueBatchSize != 8 || configuration.TimeoutSeconds != 20 || !configuration.ShouldResume || configuration.File != nil || len(args) != 0 {
			t.Fatalf("%v: %+v explicit=%t err=%v", values, configuration, explicit, err)
		}
		if len(values) == 0 && (configuration.WorkerCount != nil || configuration.ProcessCount != nil) {
			t.Fatal("defaults forced concurrency")
		}
	}
}

func TestExplicitLegacyBatchWarnsOnceBeforeMissingKeyFailure(t *testing.T) {
	t.Setenv("LITRADAR_SECRET_KEY_FILE", "")
	previous := slog.Default()
	defer slog.SetDefault(previous)
	for _, explicit := range []bool{false, true} {
		var output bytes.Buffer
		slog.SetDefault(slog.New(slog.NewJSONHandler(&output, nil)))
		root := t.TempDir()
		args := []string{"--project-root", root}
		if explicit {
			args = append(args, "--issue-batch", "5")
		}
		if err := runIndex(context.Background(), args, "unused", io.Discard); err == nil {
			t.Fatal("missing key accepted")
		}
		expected := 0
		if explicit {
			expected = 1
		}
		if strings.Count(output.String(), `"event":"cli.index.legacy_issue_batch"`) != expected {
			t.Fatal(output.String())
		}
		if explicit && (!strings.Contains(output.String(), `"level":"WARN"`) || !strings.Contains(output.String(), `"behavior":"resume_compatibility_only"`)) {
			t.Fatal(output.String())
		}
		if strings.Contains(output.String(), root) {
			t.Fatal("warning exposed root")
		}
	}
}
