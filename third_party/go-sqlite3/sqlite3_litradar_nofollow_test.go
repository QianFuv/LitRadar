//go:build cgo

package sqlite3

import (
	"bytes"
	"database/sql/driver"
	"fmt"
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
			t.Fatalf("Windows native symlink open failed: %v", err)
		}
		rejected.Close()
		t.Log("Windows VFS accepts the final symlink even with NOFOLLOW; application path validation remains required")
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

// TestLitRadarDeferredSynchronous preserves version-first preflight on malformed schemas.
func TestLitRadarDeferredSynchronous(t *testing.T) {
	for _, version := range []int64{20, 21} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "preflight.sqlite")
			ordinary := &SQLiteDriver{}
			connection, err := ordinary.Open(filename)
			if err != nil {
				t.Fatal(err)
			}
			rows, err := connection.(driver.Queryer).Query("PRAGMA synchronous", nil)
			if err != nil {
				t.Fatal(err)
			}
			values := make([]driver.Value, 1)
			if err := rows.Next(values); err != nil || values[0] != int64(1) {
				t.Fatalf("default synchronous changed: %v %v", values, err)
			}
			rows.Close()
			statement := fmt.Sprintf("CREATE TABLE marker(value TEXT); PRAGMA user_version=%d; PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql='CREATE TABLE marker(' WHERE name='marker'", version)
			if _, err := connection.(driver.Execer).Exec(statement, nil); err != nil {
				t.Fatal(err)
			}
			if err := connection.Close(); err != nil {
				t.Fatal(err)
			}
			original, err := os.ReadFile(filename)
			if err != nil {
				t.Fatal(err)
			}
			if unexpected, err := ordinary.Open(filename); err == nil {
				unexpected.Close()
				t.Fatal("default schema validation changed")
			}
			preflight, err := (&SQLiteDriver{DeferSynchronous: true}).Open(filename)
			if err != nil {
				t.Fatal(err)
			}
			rows, err = preflight.(driver.Queryer).Query("PRAGMA user_version", nil)
			if err != nil {
				t.Fatal(err)
			}
			if err := rows.Next(values); err != nil || values[0] != version {
				t.Fatalf("preflight version: %v %v", values, err)
			}
			rows.Close()
			if err := preflight.Close(); err != nil {
				t.Fatal(err)
			}
			current, err := os.ReadFile(filename)
			if err != nil || !bytes.Equal(original, current) {
				t.Fatalf("preflight mutated bytes: %v", err)
			}
		})
	}
}
