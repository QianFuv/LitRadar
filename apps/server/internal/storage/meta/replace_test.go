package meta

import (
	"os"
	"path/filepath"
	"testing"
)

func TestPublicationNeverClobbersExistingTarget(t *testing.T) {
	directory := t.TempDir()
	source, target := filepath.Join(directory, "source"), filepath.Join(directory, "target")
	writeFixture(t, source, []byte("new"))
	writeFixture(t, target, []byte("operator"))
	if err := publishFile(source, target); err == nil {
		t.Fatal("existing target overwritten")
	}
	for filename, expected := range map[string]string{source: "new", target: "operator"} {
		data, err := os.ReadFile(filename)
		if err != nil || string(data) != expected {
			t.Fatalf("%s lost after failed publication", filename)
		}
	}
}
