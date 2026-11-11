package index

import (
	"os"
	"path/filepath"
	"testing"
)

func TestHistoryPublicationRejectsEscapingSymlink(t *testing.T) {
	outside := filepath.Join(t.TempDir(), "outside.json")
	if err := os.WriteFile(outside, []byte("original"), 0600); err != nil {
		t.Fatal(err)
	}
	destination := filepath.Join(t.TempDir(), "history.json")
	if err := os.Symlink(outside, destination); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}
	if err := PublishContentChangeHistory(destination, []byte("changed")); err == nil {
		t.Fatal("escaping symlink admitted")
	}
	body, err := os.ReadFile(outside)
	if err != nil || string(body) != "original" {
		t.Fatal(string(body), err)
	}
}
