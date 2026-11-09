package sqlite

import (
	"context"
	"path/filepath"
	"testing"
)

// TestStaticSimpleWorksAcrossWorkingDirectories requires registration from an empty data root.
func TestStaticSimpleWorksAcrossWorkingDirectories(t *testing.T) {
	t.Chdir(t.TempDir())
	database, err := OpenPlain(filepath.Join(t.TempDir(), "static.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := LoadSimple(connection); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(context.Background(), "CREATE VIRTUAL TABLE search USING fts5(text,tokenize='simple 0'); INSERT INTO search VALUES('中文期刊');"); err != nil {
		t.Fatal(err)
	}
	var count int
	if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM search WHERE search MATCH '中文'").Scan(&count); err != nil || count != 1 {
		t.Fatalf("static search failed: %d %v", count, err)
	}
}

// TestSimpleLoaderPreservesExistingFunction checks repeat loading on the same physical connection.
func TestSimpleLoaderPreservesExistingFunction(t *testing.T) {
	ctx := context.Background()
	database, err := OpenPlain(filepath.Join(t.TempDir(), "native.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := LoadSimple(connection); err != nil {
		t.Fatal(err)
	}
	if err := LoadSimple(connection); err != nil {
		t.Fatal(err)
	}
	var hasFunction bool
	if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name='simple_highlight')").Scan(&hasFunction); err != nil || !hasFunction {
		t.Fatalf("native function unavailable: %t %v", hasFunction, err)
	}
}
