//go:build cgo

package sqlite3

import (
	"bytes"
	"database/sql/driver"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// TestLitRadarNoFollowPreservesDefault tests the native flag at the actual driver open boundary.
func TestLitRadarNoFollowPreservesDefault(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "native-open.sqlite")
	strict := &SQLiteDriver{NoFollow: true}
	connection, err := strict.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := connection.(driver.Execer).Exec("CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('unchanged')", nil); err != nil {
		t.Fatal(err)
	}
	if err := connection.Close(); err != nil {
		t.Fatal(err)
	}
	original, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "alias.sqlite")
	if err := os.Symlink(filename, link); err != nil {
		t.Fatalf("Native symlink proof unavailable: %v", err)
	}
	ordinary, err := (&SQLiteDriver{}).Open(link)
	if err != nil {
		t.Fatalf("Default driver must preserve symlink behavior: %v", err)
	}
	if err := ordinary.Close(); err != nil {
		t.Fatal(err)
	}
	rejected, err := strict.Open(link)
	if runtime.GOOS == "windows" {
		if err != nil {
			t.Fatalf("Windows native behavior differs from frozen Rust SQLite: %v", err)
		}
		rejected.Close()
		t.Log("Windows VFS accepts the final symlink even with NOFOLLOW, matching Rust SQLite 3.50.2; application path validation remains required")
	} else if err == nil {
		rejected.Close()
		t.Fatal("NOFOLLOW unexpectedly accepted a symbolic database path")
	}
	if info, err := os.Stat(filename); err != nil || info.Size() == 0 {
		t.Fatalf("Original database lost: %v", err)
	}
	for attempt := 0; attempt < 2; attempt++ {
		reopened, err := strict.Open(filename)
		if err != nil {
			t.Fatalf("Regular file reconnect failed: %v", err)
		}
		if err := reopened.Close(); err != nil {
			t.Fatal(err)
		}
	}
	current, err := os.ReadFile(filename)
	if err != nil || !bytes.Equal(original, current) {
		t.Fatalf("external target changed: %v", err)
	}
}
