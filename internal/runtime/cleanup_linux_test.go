package runtime

import (
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

func TestCleanupRetainsNonUtf8BasenamesWithValidExtension(t *testing.T) {
	storage := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(storage.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(storage.IndexDir, string([]byte{0xff})+".sqlite")
	if err := os.WriteFile(filename, nil, 0600); err != nil {
		t.Fatal(err)
	}
	paths, err := cleanupPaths([]string{"--project-root", storage.ProjectRoot})
	if err != nil || !slices.Contains(paths, filename) {
		t.Fatal("valid extension was discarded", paths, err)
	}
}
