package content

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// TestCreateHistoricalFixtures preserves seeded records and every supported schema boundary.
func TestCreateHistoricalFixtures(t *testing.T) {
	for version := 4; version <= 9; version++ {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "nested", "content.sqlite")
			if err := Create(filename, version); err != nil {
				t.Fatal(err)
			}
			database := openFixtureDatabase(t, filename)
			assertFixtureRecords(t, database)
			assertFixtureSchema(t, database, version)
		})
	}
}

// openFixtureDatabase owns its independent reader until the test completes.
func openFixtureDatabase(t *testing.T, filename string) *sql.DB {
	t.Helper()
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := database.Close(); err != nil {
			t.Error(err)
		}
	})
	return database
}

// fixtureValues reads independent scalar expectations and closes the cursor before returning.
func fixtureValues(t *testing.T, database *sql.DB, statement string) []string {
	t.Helper()
	rows, err := database.QueryContext(context.Background(), statement)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	values := []string{}
	for rows.Next() {
		var value string
		if err := rows.Scan(&value); err != nil {
			t.Fatal(err)
		}
		values = append(values, value)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return values
}

// assertFixtureValue compares complete ordered rows rather than only counts.
func assertFixtureValue(t *testing.T, database *sql.DB, statement, expected string) {
	t.Helper()
	if actual := strings.Join(fixtureValues(t, database, statement), "|"); actual != expected {
		t.Fatalf("fixture rows changed: %q != %q", actual, expected)
	}
}

// assertFixtureRecords checks seeded journals, article identities, author JSON, listing and FTS.
func assertFixtureRecords(t *testing.T, database *sql.DB) {
	t.Helper()
	assertFixtureValue(t, database, "SELECT journal_id||':'||catalog_id||':'||title||':'||issns_json||':'||title_aliases_json FROM journals ORDER BY journal_id",
		`1:journal-one:Journal One:["1234-5679"]:[]|2:journal-two:Journal Two:["2049-3630"]:[]`)
	assertFixtureValue(t, database, "SELECT article_id||':'||journal_id||':'||title||':'||authors_json||':'||date FROM articles ORDER BY article_id",
		`1001:1:Synthetic first article:[{"display_name":"Author One"}]:2026-01-01|1012:2:Synthetic second article:[{"display_name":"Author Two"}]:2026-01-02`)
	assertFixtureValue(t, database, "SELECT article_id||':'||journal_id||':'||date FROM article_listing ORDER BY article_id",
		"1001:1:2026-01-01|1012:2:2026-01-02")
	assertFixtureValue(t, database, "SELECT rowid FROM article_search WHERE article_search MATCH 'Synthetic' ORDER BY rowid", "1001|1012")
}

// assertFixtureSchema checks versions, legacy fields and tokenizer storage without migrating the reader.
func assertFixtureSchema(t *testing.T, database *sql.DB, version int) {
	t.Helper()
	assertFixtureValue(t, database, "PRAGMA user_version", fmt.Sprint(version))
	assertFixtureSchemaObject(t, database, "table", "journal_identity_keys", version >= 5)
	assertFixtureSchemaObject(t, database, "table", "article_retraction_dois", version >= 6)
	assertFixtureSchemaObject(t, database, "index", "idx_article_change_events_order", version < 8)
	assertFixtureSchemaObject(t, database, "table", "article_search_content", version < 7)
	assertFixtureLegacyColumn(t, database, version)
	assertFixtureTokenizer(t, database, version)
	if version >= 5 {
		assertFixtureValue(t, database, "SELECT identity_kind||':'||identity_value||':'||canonical_catalog_id FROM journal_identity_keys ORDER BY identity_kind, identity_value",
			"catalog_id:journal-one:journal-one|catalog_id:journal-two:journal-two|issn:1234-5679:journal-one|issn:2049-3630:journal-two")
	}
}

// assertFixtureSchemaObject verifies presence and absence for independently declared historical boundaries.
func assertFixtureSchemaObject(t *testing.T, database *sql.DB, kind, name string, shouldExist bool) {
	t.Helper()
	var count int
	if err := database.QueryRow("SELECT count(*) FROM sqlite_schema WHERE type=? AND name=?", kind, name).Scan(&count); err != nil {
		t.Fatal(err)
	}
	expected := 0
	if shouldExist {
		expected = 1
	}
	if count != expected {
		t.Fatal("historical schema object changed", kind, name, count, expected)
	}
}

// assertFixtureLegacyColumn distinguishes old article fields from the current retraction table.
func assertFixtureLegacyColumn(t *testing.T, database *sql.DB, version int) {
	t.Helper()
	var count int
	if err := database.QueryRow("SELECT count(*) FROM pragma_table_info('articles') WHERE name='retraction_doi'").Scan(&count); err != nil {
		t.Fatal(err)
	}
	expected := 0
	if version < 6 {
		expected = 1
	}
	if count != expected {
		t.Fatal("historical retraction column changed", version, count)
	}
}

// assertFixtureTokenizer independently checks tokenizer selection and contentless deletion support.
func assertFixtureTokenizer(t *testing.T, database *sql.DB, version int) {
	t.Helper()
	var declaration string
	if err := database.QueryRow("SELECT sql FROM sqlite_schema WHERE name='article_search'").Scan(&declaration); err != nil {
		t.Fatal(err)
	}
	tokenizer := "tokenize = 'unicode61 remove_diacritics 2'"
	if version == 9 {
		tokenizer = "tokenize = 'simple 0'"
	}
	if !strings.Contains(declaration, tokenizer) || strings.Contains(declaration, "contentless_delete = 1") != (version >= 7) {
		t.Fatal("historical search options changed", version, declaration)
	}
}

// TestCreateRejectsUnsupportedBeforeWrites leaves the supplied root and operator data untouched.
func TestCreateRejectsUnsupportedBeforeWrites(t *testing.T) {
	for _, version := range []int{-1, 0, 3, 10, 100} {
		root := t.TempDir()
		filename := filepath.Join(root, "missing", "content.sqlite")
		if err := Create(filename, version); err == nil || err.Error() != fmt.Sprintf("unsupported test schema version %d", version) {
			t.Fatal("unsupported version accepted", version, err)
		}
		if _, err := os.Stat(filepath.Dir(filename)); !os.IsNotExist(err) {
			t.Fatal("unsupported fixture created its root", err)
		}
	}
}

// TestCreateFailureReleasesDatabase preserves existing records and closes owned handles on schema failure.
func TestCreateFailureReleasesDatabase(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "content.sqlite")
	if err := Create(filename, 9); err != nil {
		t.Fatal(err)
	}
	if err := Create(filename, 4); err == nil {
		t.Fatal("existing schema unexpectedly accepted")
	}
	moved := filename + ".moved"
	if err := os.Rename(filename, moved); err != nil {
		t.Fatal("failed fixture retained its database handle", err)
	}
	if err := os.Rename(moved, filename); err != nil {
		t.Fatal(err)
	}
	database := openFixtureDatabase(t, filename)
	assertFixtureRecords(t, database)
	assertFixtureValue(t, database, "PRAGMA user_version", "9")
}

// TestCreateLateFailurePreservesPartialWrites retains seeded rows without publishing a schema version.
func TestCreateLateFailurePreservesPartialWrites(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "content.sqlite")
	database, err := storage.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	_, failure := database.Exec("CREATE TABLE operator_marker(value TEXT); CREATE INDEX idx_article_change_events_order ON operator_marker(value)")
	closeFailure := database.Close()
	if failure != nil || closeFailure != nil {
		t.Fatal(failure, closeFailure)
	}
	if err := Create(filename, 4); err == nil {
		t.Fatal("conflicting late index accepted")
	}
	moved := filename + ".moved"
	if err := os.Rename(filename, moved); err != nil {
		t.Fatal("late failure retained its database handle", err)
	}
	database = openFixtureDatabase(t, moved)
	assertFixtureRecords(t, database)
	assertFixtureValue(t, database, "PRAGMA user_version", "0")
	assertFixtureSchemaObject(t, database, "table", "journal_identity_keys", true)
	assertFixtureSchemaObject(t, database, "table", "article_retraction_dois", true)
	assertFixtureLegacyColumn(t, database, 6)
	assertFixtureValue(t, database, "SELECT tbl_name FROM sqlite_schema WHERE name='idx_article_change_events_order'", "operator_marker")
}

// TestCreateBlockedParentPreservesOperatorFile proves directory failure precedes opening a database.
func TestCreateBlockedParentPreservesOperatorFile(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "operator-data")
	if err := os.WriteFile(filename, []byte("preserve"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := Create(filepath.Join(filename, "content.sqlite"), 9); err == nil {
		t.Fatal("file parent accepted")
	}
	body, err := os.ReadFile(filename)
	if err != nil || string(body) != "preserve" {
		t.Fatal("operator file changed", err)
	}
}
