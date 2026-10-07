package sqlite

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

// TestSimpleLibraryRetainsSourceAnchorAcrossWorkingDirectories checks compiled-source discovery independence.
func TestSimpleLibraryRetainsSourceAnchorAcrossWorkingDirectories(t *testing.T) {
	original, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	library, err := SimpleLibrary()
	if err != nil {
		t.Fatal(err)
	}
	if !filepath.IsAbs(library) {
		t.Fatalf("library is not absolute: %s", library)
	}
	info, err := os.Stat(library)
	if err != nil || !info.Mode().IsRegular() {
		t.Fatalf("library is not regular: %s %v", library, err)
	}
	if err := os.Chdir(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := os.Chdir(original); err != nil {
			t.Fatal(err)
		}
	}()
	discovered, err := SimpleLibrary()
	if err != nil || discovered != library {
		t.Fatalf("working directory changed library: %s %v", discovered, err)
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
