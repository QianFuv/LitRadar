package fullstack

import (
	"os"
	"path/filepath"
	"testing"
)

func TestRelativeTemporaryDirectoryUsesCanonicalContainment(t *testing.T) {
	parent := t.TempDir()
	t.Chdir(parent)
	t.Setenv("TMPDIR", "temporary")
	root := filepath.Join(parent, "temporary", "fixture")
	if err := os.MkdirAll(root, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, markerFile), []byte(markerContent), 0600); err != nil {
		t.Fatal(err)
	}
	actual, err := validateRoot(root)
	if err != nil || actual != root {
		t.Fatal("relative TMPDIR rejected its own descendant", actual, err)
	}
}
