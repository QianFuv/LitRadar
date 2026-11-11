package meta

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestRootPublicationDoesNotOverwriteExistingFile(t *testing.T) {
	directory := t.TempDir()
	source, target := filepath.Join(directory, "source"), filepath.Join(directory, "target")
	writeFixture(t, source, []byte("new"))
	writeFixture(t, target, []byte("original"))
	if err := publishFile(source, target); !errors.Is(err, os.ErrExist) {
		t.Fatal(err)
	}
	data, err := os.ReadFile(target)
	if err != nil || string(data) != "original" {
		t.Fatal(string(data), err)
	}
}
