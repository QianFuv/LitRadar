package index

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"testing"

	runtimeindex "github.com/QianFuv/LitRadar/apps/server/internal/storage/index"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/indexschema"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// obsoleteSearchFixture emulates an installed Simple schema without requiring the deleted extension.
func obsoleteSearchFixture(t *testing.T) string {
	t.Helper()
	filename, _ := copyFixture(t, historical{Version: indexschema.Version})
	database, err := storage.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	}()
	if _, err := database.Exec("UPDATE articles SET title='研究 O''Brien AB12',abstract_text='😀' WHERE article_id=1001;PRAGMA writable_schema=ON"); err != nil {
		t.Fatal(err)
	}
	declaration := `CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,content='',contentless_delete=1,tokenize='simple 0')`
	if _, err := database.Exec("UPDATE sqlite_schema SET sql=? WHERE name='article_search'", declaration); err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("PRAGMA writable_schema=OFF"); err != nil {
		t.Fatal(err)
	}
	return filename
}

// TestObsoleteSimpleRebuildsOnEveryEntrance covers same-version startup, explicit migration and runtime opening.
func TestObsoleteSimpleRebuildsOnEveryEntrance(t *testing.T) {
	for _, entrance := range []string{"preflight", "migrate", "runtime"} {
		t.Run(entrance, func(t *testing.T) {
			filename := obsoleteSearchFixture(t)
			ctx := context.Background()
			switch entrance {
			case "preflight":
				if level, err := Preflight(ctx, filename); err != nil || level != "full_integrity" {
					t.Fatalf("%s %v", level, err)
				}
			case "migrate":
				if summary, err := Migrate(ctx, filename); err != nil || summary != (Summary{indexschema.Version, indexschema.Version}) {
					t.Fatalf("%+v %v", summary, err)
				}
			case "runtime":
				connection, err := runtimeindex.OpenContent(ctx, filename)
				if err != nil {
					t.Fatal(err)
				}
				if err := connection.Close(); err != nil {
					t.Errorf("connection.Close: %v", err)
				}
			}
			inspectConnection(t, filename, func(connection *sql.Conn) {
				if needed, err := search.NeedsRebuild(ctx, connection); err != nil || needed {
					t.Fatalf("obsolete schema retained: %t %v", needed, err)
				}
				var id int64
				if err := connection.QueryRowContext(ctx, "SELECT rowid FROM article_search WHERE article_search MATCH ?", search.Phrase("研究 O'Brien AB12")).Scan(&id); err != nil || id != 1001 {
					t.Fatalf("rebuilt phrase id=%d err=%v", id, err)
				}
				var title string
				if err := connection.QueryRowContext(ctx, "SELECT title FROM articles WHERE article_id=1001").Scan(&title); err != nil || title != "研究 O'Brien AB12" {
					t.Fatalf("canonical title changed: %s %v", title, err)
				}
			})
			if level, err := Preflight(ctx, filename); err != nil || level != "schema_only" {
				t.Fatalf("repeat startup %s %v", level, err)
			}
		})
	}
}

// TestObsoleteSimpleFailurePreservesDeclaration checks rollback after a late invalid canonical row.
func TestObsoleteSimpleFailurePreservesDeclaration(t *testing.T) {
	for _, entrance := range []string{"migrate", "runtime"} {
		t.Run(entrance, func(t *testing.T) {
			filename := obsoleteSearchFixture(t)
			mutateFixture(t, filename, "UPDATE articles SET authors_json='null' WHERE article_id=1012")
			var err error
			if entrance == "migrate" {
				_, err = Migrate(context.Background(), filename)
				if !errors.Is(err, ErrInvalidQuery) {
					t.Fatalf("unexpected failure: %v", err)
				}
			} else {
				_, err = runtimeindex.OpenContent(context.Background(), filename)
				if !errors.Is(err, search.ErrInvalidAuthors) {
					t.Fatalf("unexpected failure: %v", err)
				}
			}
			inspectConnection(t, filename, func(connection *sql.Conn) {
				var declaration string
				if err := connection.QueryRowContext(context.Background(), "SELECT sql FROM sqlite_schema WHERE name='article_search'").Scan(&declaration); err != nil || !strings.Contains(declaration, "simple 0") {
					t.Fatalf("old declaration lost: %s %v", declaration, err)
				}
			})
		})
	}
}
