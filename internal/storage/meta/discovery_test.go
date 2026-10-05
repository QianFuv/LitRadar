package meta

import (
	"os"
	"path/filepath"
	"testing"
)

func TestPackagedDirectoryUsesPortableBundleWithoutOverridingSystemBundle(t *testing.T) {
	root := t.TempDir()
	system := filepath.Join(root, "system")
	portable := filepath.Join(root, "release", "assets", "meta")
	if err := os.MkdirAll(portable, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(portable, manifestFilename), []byte("{}"), 0644); err != nil {
		t.Fatal(err)
	}
	if got, err := findPackagedDirectory([]string{system, portable}); err != nil || got != portable {
		t.Fatalf("portable bundle: %q, %v", got, err)
	}
	if err := os.MkdirAll(system, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(system, manifestFilename), []byte("{}"), 0644); err != nil {
		t.Fatal(err)
	}
	if got, err := findPackagedDirectory([]string{system, portable}); err != nil || got != system {
		t.Fatalf("system bundle precedence: %q, %v", got, err)
	}
	if got, err := findPackagedDirectory([]string{filepath.Join(root, "missing")}); err != nil || got != "" {
		t.Fatalf("unpackaged development build: %q, %v", got, err)
	}
}
