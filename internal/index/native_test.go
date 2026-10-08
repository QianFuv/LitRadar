package index

import (
	"context"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"path/filepath"
	"testing"
)

// TestIndexStaticTokenizer works without discovering a shared library in the project or working directory.
func TestIndexStaticTokenizer(t *testing.T) {
	t.Chdir(t.TempDir())
	database, err := sqlite.Open(filepath.Join(t.TempDir(), "static.sqlite"), false, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := sqlite.LoadSimple(connection); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(context.Background(), "CREATE VIRTUAL TABLE search USING fts5(text,tokenize='simple 0'); INSERT INTO search VALUES('中文期刊 alpha')"); err != nil {
		t.Fatal(err)
	}
	for query, expected := range map[string]int{"中文": 1, "alpha": 1, "zhongwen": 0} {
		var count int
		if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM search WHERE search MATCH ?", query).Scan(&count); err != nil || count != expected {
			t.Fatalf("static MATCH %q: %d %v", query, count, err)
		}
	}
}
