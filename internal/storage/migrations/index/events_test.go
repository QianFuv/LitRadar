package index

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"os"
	"path/filepath"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

func TestBatchStopsAtFailureAndReportsCompletedDatabasesWithoutPaths(t *testing.T) {
	for _, isPreflight := range []bool{false, true} {
		configuration := config.FromProjectRoot(t.TempDir())
		if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
			t.Fatal(err)
		}
		_, raw := copyFixture(t, fixtures(t)[2])
		for _, name := range []string{"a.sqlite", "b.sqlite", "c.sqlite"} {
			if err := os.WriteFile(filepath.Join(configuration.IndexDir, name), raw, 0600); err != nil {
				t.Fatal(err)
			}
		}
		mutateFixture(t, filepath.Join(configuration.IndexDir, "b.sqlite"), "PRAGMA user_version=99")
		var output bytes.Buffer
		previous := slog.Default()
		slog.SetDefault(slog.New(slog.NewJSONHandler(&output, nil)))
		var err error
		if isPreflight {
			err = PreflightExisting(context.Background(), configuration)
		} else {
			err = MigrateExisting(context.Background(), configuration)
		}
		slog.SetDefault(previous)
		if err == nil {
			t.Fatal("future database accepted")
		}
		if bytes.Contains(output.Bytes(), []byte(configuration.ProjectRoot)) {
			t.Fatal("private database path leaked into event")
		}
		lines := bytes.Split(bytes.TrimSpace(output.Bytes()), []byte("\n"))
		var last map[string]any
		if err := json.Unmarshal(lines[len(lines)-1], &last); err != nil {
			t.Fatal(err)
		}
		prefix := "storage.migration.batch.failed"
		if isPreflight {
			prefix = "storage.index_preflight.batch.failed"
		}
		if last["event"] != prefix || last["discovered_count"] != float64(3) || last["completed_count"] != float64(1) || last["error_kind"] != "unsupported_schema_version" {
			t.Fatal(last)
		}
		first, err := inspect(context.Background(), filepath.Join(configuration.IndexDir, "a.sqlite"))
		if err != nil {
			t.Fatal(err)
		}
		expected := 9
		if isPreflight {
			expected = 6
		}
		if first.version != expected {
			t.Fatal(first)
		}
		lastDb, err := inspect(context.Background(), filepath.Join(configuration.IndexDir, "c.sqlite"))
		if err != nil || lastDb.version != 6 {
			t.Fatalf("later database touched: %+v %v", lastDb, err)
		}
	}
}
