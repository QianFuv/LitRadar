package sqlite

import (
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestExtendedWindowsPathOpensExactTarget(t *testing.T) {
	ordinary := filepath.Join(t.TempDir(), "中文 #% &.sqlite")
	extended := `\\?\` + ordinary
	database, err := Open(Config{Filename: extended, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec("CREATE TABLE proof(value TEXT); INSERT INTO proof VALUES('exact target')"); err != nil {
		t.Fatal(err)
	}
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(ordinary); err != nil {
		t.Fatal(err)
	}
	reopened, err := Open(Config{Filename: ordinary, Mode: "ro", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	var value string
	if err := reopened.QueryRow("SELECT value FROM proof").Scan(&value); err != nil || value != "exact target" {
		t.Fatalf("wrong database %q %v", value, err)
	}
}

func TestUncAndExtendedUriKeepEmptyAuthority(t *testing.T) {
	for _, filename := range []string{`\\server\share\中文 #% &.sqlite`, `\\?\UNC\server\share\中文 #% &.sqlite`, `\\?\C:\中文 #% &.sqlite`} {
		encoded, err := FileUri(filename, "rw")
		if err != nil {
			t.Fatal(err)
		}
		parsed, err := url.Parse(encoded)
		if err != nil || parsed.Host != "" || parsed.Path != strings.ReplaceAll(filename, `\`, "/") {
			t.Fatalf("path lost: %s %v %v", encoded, parsed, err)
		}
	}
}
