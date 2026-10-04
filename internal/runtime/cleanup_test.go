package runtime

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestCleanupDiscoveryPreservesRawOptionsAndFileKinds(t *testing.T) {
	storage := config.FromProjectRoot(t.TempDir())
	for _, directory := range []string{storage.IndexDir, storage.IndexControlDir} {
		if err := os.MkdirAll(filepath.Join(directory, "directory.sqlite"), 0700); err != nil {
			t.Fatal(err)
		}
		for _, name := range []string{"english.sqlite", "ignored.txt", "upper.SQLITE", ".sqlite"} {
			if err := os.WriteFile(filepath.Join(directory, name), nil, 0600); err != nil {
				t.Fatal(err)
			}
		}
	}
	args := []string{"--project-root", storage.ProjectRoot, "--project-root", "ignored", "--auth-db", "relative.sqlite"}
	paths, err := cleanupPaths(args)
	expected := []string{"relative.sqlite", filepath.Join(storage.IndexDir, "english.sqlite"), filepath.Join(storage.IndexControlDir, "english.sqlite")}
	slices.Sort(expected)
	if err != nil || !slices.Equal(paths, expected) {
		t.Fatal(paths, err)
	}
	if _, err := cleanupPaths([]string{"--project-root", storage.ProjectRoot, "--auth-db"}); err == nil {
		t.Fatal("missing explicit database widened cleanup")
	}
}

func TestInternalChildLeavesIdleSidecarsForParent(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "database.sqlite")
	database, err := sqlite.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("CREATE TABLE item(id INTEGER PRIMARY KEY); INSERT INTO item DEFAULT VALUES"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	reader, err := sqlite.Open(filename, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	var count int
	if err := reader.QueryRow("SELECT count(*) FROM item").Scan(&count); err != nil {
		t.Fatal(err)
	}
	reader.Close()
	for _, marker := range []string{"--litradar-parent-run-id", "--live-worker-request"} {
		CleanupAfterProcess([]string{"--project-root", root, "--auth-db", filename, marker})
		if _, err := os.Stat(filename + "-wal"); err != nil {
			t.Fatal("child removed parent-owned sidecars", marker, err)
		}
	}
	CleanupAfterProcess([]string{"--project-root", root, "--auth-db", filename})
	if outcome, err := sqlite.CleanupSidecars(context.Background(), filename); err != nil || outcome != sqlite.SidecarNotPresent {
		t.Fatal("parent did not converge idle sidecars", outcome, err)
	}
}
