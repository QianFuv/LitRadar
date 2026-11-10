package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestSchemaInputsMatchProductionMigrations detects changes missing from the committed schema inputs.
func TestSchemaInputsMatchProductionMigrations(t *testing.T) {
	if err := run(context.Background(), filepath.Join("..", "..", "internal", "storage", "sqlc", "schema"), true); err != nil {
		t.Fatal(err)
	}
}

// TestGenerationIsRepeatableAndDetectsDrift checks repeatable exports and read-only drift detection.
func TestGenerationIsRepeatableAndDetectsDrift(t *testing.T) {
	directory := t.TempDir()
	ctx := context.Background()
	if err := run(ctx, directory, false); err != nil {
		t.Fatal(err)
	}
	if err := run(ctx, directory, true); err != nil {
		t.Fatal(err)
	}
	index, err := os.ReadFile(filepath.Join(directory, "index.sql"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(index), "CREATE TABLE articles") || strings.Contains(string(index), "article_search") {
		t.Fatal("schema input must include relational content and exclude virtual FTS tables and their shadow tables")
	}
	if err := os.WriteFile(filepath.Join(directory, "auth.sql"), []byte("-- stale schema\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := run(ctx, directory, true); err == nil || !strings.Contains(err.Error(), "differs from production migrations") {
		t.Fatalf("schema drift was not reported: %v", err)
	}
}
