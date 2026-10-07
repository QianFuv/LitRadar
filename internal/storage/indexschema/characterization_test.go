package indexschema

import (
	"context"
	"database/sql"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func schemaFixture(t *testing.T, version int) *sql.Conn {
	t.Helper()
	database, err := storage.OpenMigration(filepath.Join(t.TempDir(), "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	if err := storage.LoadSimple(connection); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(context.Background(), schemaFixtureSql(version)); err != nil {
		t.Fatal(err)
	}
	return connection
}

func schemaFixtureSql(version int) string {
	statement := ContentTables
	if version < 9 {
		statement = strings.ReplaceAll(statement, "tokenize = 'simple 0'", "tokenize = 'unicode61 remove_diacritics 2'")
	}
	if version < 7 {
		statement = strings.ReplaceAll(statement, "        content = '',\n        contentless_delete = 1,\n", "")
	}
	if version < 8 {
		statement += "CREATE INDEX idx_article_change_events_order ON article_change_events(event_id);"
	}
	if version < 6 {
		statement += "DROP TABLE article_retraction_dois; ALTER TABLE articles ADD COLUMN retraction_doi TEXT;"
	}
	if version < 5 {
		statement += "DROP TABLE journal_identity_keys;"
	}
	return statement
}

func TestStructureAdmissionRetainsAllSupportedVersions(t *testing.T) {
	for version := 4; version <= 9; version++ {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			connection := schemaFixture(t, version)
			if err := ValidateStructure(context.Background(), connection, version); err != nil {
				t.Fatal(err)
			}
			if err := Validate(context.Background(), connection, version); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestStructureValidationRetainsFirstMismatchPhase(t *testing.T) {
	for _, scenario := range []struct {
		name, mutation, message string
	}{
		{"unsupported", "CREATE TABLE unexpected(value TEXT)", "unsupported content schema version"},
		{"table", "CREATE TABLE unexpected(value TEXT); ALTER TABLE journals ADD COLUMN unexpected TEXT; DROP INDEX idx_articles_date_id", "table inventory mismatch"},
		{"column", "ALTER TABLE journals ADD COLUMN unexpected TEXT; ALTER TABLE issues ADD COLUMN unexpected TEXT; DROP INDEX idx_articles_date_id", "column inventory mismatch for journals"},
		{"index", "DROP INDEX idx_articles_date_id; DROP TABLE article_search; CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,tokenize='unicode61 remove_diacritics 2')", "index inventory mismatch"},
		{"search", "DROP TABLE article_search; CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,tokenize='unicode61 remove_diacritics 2')", "article_search storage options"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			connection := schemaFixture(t, 9)
			if _, err := connection.ExecContext(context.Background(), scenario.mutation); err != nil {
				t.Fatal(err)
			}
			version := 9
			if scenario.name == "unsupported" {
				version = 3
			}
			err := ValidateStructure(context.Background(), connection, version)
			if err == nil || !strings.HasPrefix(err.Error(), scenario.message) {
				t.Fatalf("expected %q, got %v", scenario.message, err)
			}
		})
	}
}
