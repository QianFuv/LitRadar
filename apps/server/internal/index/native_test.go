package index

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/indexschema"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"path/filepath"
	"testing"
)

// TestIndexGoTokenizer works without discovering a shared library in the project or working directory.
func TestIndexGoTokenizer(t *testing.T) {
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
	if _, err := connection.ExecContext(context.Background(), indexschema.SearchTable); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(context.Background(), "INSERT INTO article_search(rowid,title) VALUES(1,?)", search.PrepareText("中文期刊 alpha")); err != nil {
		t.Fatal(err)
	}
	for query, expected := range map[string]int{"中文": 1, "alpha": 1, "zhongwen": 0} {
		var count int
		if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM article_search WHERE article_search MATCH ?", search.Phrase(query)).Scan(&count); err != nil || count != expected {
			t.Fatalf("Go-tokenized MATCH %q: %d %v", query, count, err)
		}
	}
}
