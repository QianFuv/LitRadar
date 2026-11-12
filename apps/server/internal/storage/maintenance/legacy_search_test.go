package maintenance

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/indexschema"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// legacySearchFixture reproduces installed search declarations absent from the current testkit fixtures.
func legacySearchFixture(t *testing.T, version int, tokenizer string) config.Config {
	t.Helper()
	configuration := fixture(t, version)
	filename := filepath.Join(configuration.IndexDir, "content.sqlite")
	database, err := storage.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	declaration := "CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED, title, abstract_text, doi, pmid, authors, journal_title, content = '', contentless_delete = 1, tokenize = '" + tokenizer + "');"
	if tokenizer == "simple 0" {
		_, err = database.Exec("PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=? WHERE name='article_search'; PRAGMA writable_schema=OFF", strings.TrimSuffix(strings.TrimSpace(declaration), ";"))
	} else {
		_, err = database.Exec("DROP TABLE article_search; " + declaration + `
			INSERT INTO article_search(rowid,article_id,title,abstract_text,journal_title)
			SELECT article_id,article_id,articles.title,abstract_text,journals.title FROM articles JOIN journals USING(journal_id)`)
	}
	closeError := database.Close()
	if err != nil || closeError != nil {
		t.Fatal(err, closeError)
	}
	old := time.Unix(1000, 0)
	if err := os.Chtimes(filename, old, old); err != nil {
		t.Fatal(err)
	}
	return configuration
}

// TestOptimizeMigratedSearchUsesPostMigrationSnapshot covers the release smoke's genuine v8 search schema.
func TestOptimizeMigratedSearchUsesPostMigrationSnapshot(t *testing.T) {
	for _, scenario := range []struct {
		name, tokenizer string
		version         int
	}{
		{"v8_unicode", "unicode61 remove_diacritics 2", 8},
		{"v10_unicode", "unicode61 remove_diacritics 2", 10},
		{"v10_simple", "simple 0", 10},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			configuration := legacySearchFixture(t, scenario.version, scenario.tokenizer)
			report, err := Optimize(context.Background(), Options{configuration, true})
			if err != nil {
				t.Fatal(err)
			}
			if report.Outcome != "optimized" || report.DatabaseCount != 1 || report.Databases[0].SourceSchemaVersion != int64(scenario.version) || report.Databases[0].TargetSchemaVersion != indexschema.Version || report.Databases[0].RowCounts["articles"] != 2 {
				t.Fatalf("unexpected optimization report: %+v", report)
			}
			if report.SourceBytes != report.Databases[0].Before.FileBytes || report.TemporaryBytesRequired != 2*report.SourceBytes+64*1024*1024 {
				t.Fatalf("estimate did not use the migrated source size: %+v", report)
			}
			if err := CheckInterrupted(configuration); err != nil {
				t.Fatal(err)
			}
			if err := readConnection(context.Background(), filepath.Join(configuration.IndexDir, "content.sqlite"), func(connection *sql.Conn) error {
				needsRebuild, err := search.NeedsRebuild(context.Background(), connection)
				if err == nil && needsRebuild {
					t.Error("optimized search still needs migration")
				}
				return err
			}); err != nil {
				t.Fatal(err)
			}
		})
	}
}

// TestOptimizeMigratedSearchStillRejectsLateChanges preserves the source guard after self-migration.
func TestOptimizeMigratedSearchStillRejectsLateChanges(t *testing.T) {
	configuration := legacySearchFixture(t, 8, "unicode61 remove_diacritics 2")
	hook := hooks{beforeValidation: func() error {
		changed := time.Unix(2000, 0)
		return os.Chtimes(filepath.Join(configuration.IndexDir, "content.sqlite"), changed, changed)
	}}
	_, err := optimize(context.Background(), Options{configuration, true}, time.Now().Unix(), hook)
	assertCode(t, err, "operation_failed")
	if !strings.Contains(err.Error(), "source_changed") {
		t.Fatal(err)
	}
}
