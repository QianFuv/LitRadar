package sqlite

import (
	"database/sql"
	"errors"
	"net/url"
	"os"
	"path/filepath"
	"testing"

	native "modernc.org/sqlite"
	sqlite3 "modernc.org/sqlite/lib"
)

// TestNoFollowVfsRejectsLinks exercises SQLite's VFS directly, without the connector's Lstat preflight.
func TestNoFollowVfsRejectsLinks(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "target.sqlite")
	database, err := Open(Config{Filename: target, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("CREATE TABLE retained(value); INSERT INTO retained VALUES(42)"); err != nil {
		t.Fatal(err)
	}
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link.sqlite")
	if err := os.Symlink(target, link); err != nil {
		t.Skipf("symbolic links unavailable: %v", err)
	}
	parent := filepath.Join(t.TempDir(), "linked-directory")
	if err := os.Symlink(root, parent); err != nil {
		t.Fatal(err)
	}
	dangling := filepath.Join(root, "dangling.sqlite")
	missing := filepath.Join(root, "missing.sqlite")
	if err := os.Symlink(missing, dangling); err != nil {
		t.Fatal(err)
	}
	for _, filename := range []string{link, filepath.Join(parent, "target.sqlite"), dangling} {
		for _, mode := range []string{"ro", "rw", "rwc"} {
			t.Run(filepath.Base(filename)+"/"+mode, func(t *testing.T) {
				dsn, err := FileUri(filename, mode)
				if err != nil {
					t.Fatal(err)
				}
				location, err := url.Parse(dsn)
				if err != nil {
					t.Fatal(err)
				}
				query := location.Query()
				if err := UseNoFollow(query); err != nil {
					t.Fatal(err)
				}
				location.RawQuery = query.Encode()
				connection, err := (&native.Driver{}).Open(location.String())
				if connection != nil {
					if err := connection.Close(); err != nil {
						t.Errorf("connection.Close: %v", err)
					}
				}
				var failure *native.Error
				if !errors.As(err, &failure) || failure.Code()&255 != sqlite3.SQLITE_CANTOPEN {
					t.Fatalf("no-follow VFS admitted a link: %v", err)
				}
			})
		}
	}
	if _, err := os.Stat(missing); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("dangling link created its target: %v", err)
	}
	dsn, err := FileUri(link, "ro")
	if err != nil {
		t.Fatal(err)
	}
	reader, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := reader.Close(); err != nil {
			t.Errorf("reader.Close: %v", err)
		}
	}()
	var value int
	if err := reader.QueryRow("SELECT value FROM retained").Scan(&value); err != nil || value != 42 {
		t.Fatalf("ordinary driver or target changed: %d %v", value, err)
	}
}
