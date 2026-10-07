package search

import (
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// TestProjectionTextPreservesFastPathAndUnicodeRules checks source-independent normalization boundaries.
func TestProjectionTextPreservesFastPathAndUnicodeRules(t *testing.T) {
	cases := []struct{ value, expected string }{
		{"ABC\tDEF", "ABC\tDEF"},
		{"ABC\tDEF!", "ABC DEF "},
		{"École Ö 中文!", "Ecole O 中文 "},
		{"\u0301\u0345", ""},
		{"a\vB", "a B"},
		{"\ue000\uf8ff\U000f0000\U000ffffd\U00100000\U0010fffd", "\ue000\uf8ff\U000f0000\U000ffffd\U00100000\U0010fffd"},
		{"\ufffd\xff", "  "},
	}
	for _, item := range cases {
		if actual := PrepareText(item.value, true); actual != item.expected {
			t.Fatalf("projection %q: %q expected %q", item.value, actual, item.expected)
		}
		if actual := PrepareText(item.value, false); actual != item.value {
			t.Fatalf("plain source changed: %q", actual)
		}
	}
}

// TestQueryProjectionPreservesGrammarAndWholeInputFallback checks advanced and simple mode boundaries.
func TestQueryProjectionPreservesGrammarAndWholeInputFallback(t *testing.T) {
	cases := []struct{ value, expected string }{
		{"Résumé AND title:Résumé", "\"Resume\" AND title:\"Resume\""},
		{"\"Résumé \"\"中文\"\"\"", "\"Resume  中文 \""},
		{"\"Résumé\":Résumé", "\"Résumé\":\"Resume\""},
		{"{Résumé title}:Résumé", "{Résumé title}:\"Resume\""},
		{"Résumé AND \"late", "Résumé AND \"late"},
		{"NEAR(Résumé* OR 中文, 2)", "NEAR(\"Resume\"* OR 中文, 2)"},
		{"_ \x1a", "\" \" \" \""},
	}
	for _, item := range cases {
		if actual := PrepareQuery(item.value, true, domain.SearchAdvanced); actual != item.expected {
			t.Fatalf("advanced %q: %q expected %q", item.value, actual, item.expected)
		}
		if actual := PrepareQuery(item.value, false, domain.SearchAdvanced); actual != item.value {
			t.Fatalf("plain query changed: %q", actual)
		}
	}
	if actual := PrepareQuery("!!!", true, domain.SearchSimple); actual != "!!!" {
		t.Fatalf("nonblank punctuation fallback changed: %q", actual)
	}
	if actual := PrepareQuery("École", true, domain.SearchMode("unknown")); actual != "Ecole" {
		t.Fatalf("nonadvanced mode changed: %q", actual)
	}
}

// TestAuthorDecodingPreservesCompleteRepresentations checks shape selection, nil failure and nonnil empty output.
func TestAuthorDecodingPreservesCompleteRepresentations(t *testing.T) {
	cases := []struct {
		payload  string
		expected []string
	}{
		{`[]`, []string{}},
		{`["A",""," A ","A"]`, []string{"A", "", " A ", "A"}},
		{`[{"display_name":"A"},["B"],{"display_name":""}]`, []string{"A", "B", ""}},
	}
	for _, item := range cases {
		actual, err := DecodeAuthorNames(item.payload)
		if err != nil || !reflect.DeepEqual(actual, item.expected) {
			t.Fatalf("authors %s: %#v %v", item.payload, actual, err)
		}
	}
	invalid := []string{
		`null`, `{}`, `[] []`, `["A",{"display_name":"B"}]`,
		`[{"display_name":"A"},"B"]`, `[[null]]`, `[[]]`,
		`[["A","B"]]`, `[{"display_name":null}]`,
		`[{"display_name":"A","other":"B"}]`,
		`[{"display_name":"A","display_name":"B"}]`,
		`["\ud800"]`, "[\"\xff\"]",
	}
	for _, payload := range invalid {
		actual, err := DecodeAuthorNames(payload)
		if actual != nil || !errors.Is(err, ErrInvalidAuthors) {
			t.Fatalf("invalid authors accepted: %q %#v %v", payload, actual, err)
		}
	}
}

// searchFixture keeps one caller-owned physical connection with seeded canonical and stale projected rows.
func searchFixture(t *testing.T, usesSimple bool) (*sql.Conn, context.Context) {
	t.Helper()
	ctx := context.Background()
	database, err := storage.OpenPlain(filepath.Join(t.TempDir(), "search.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	connection, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	if _, err := connection.ExecContext(ctx, `CREATE TABLE journals(journal_id INTEGER,title TEXT);
CREATE TABLE articles(article_id INTEGER,title TEXT,abstract_text TEXT,doi TEXT,pmid TEXT,authors_json TEXT,journal_id INTEGER);
INSERT INTO journals VALUES(1,'JÖURNAL');
INSERT INTO articles VALUES(1,'Résumé 中文!',NULL,NULL,NULL,'["Émile"]',1),(2,'SECOND','Abstract','DOI','PMID','[{"display_name":"B"}]',1);`); err != nil {
		t.Fatal(err)
	}
	createSearchFixture(t, ctx, connection, usesSimple)
	return connection, ctx
}

// createSearchFixture uses the actual bundled tokenizer when the native role is selected.
func createSearchFixture(t *testing.T, ctx context.Context, connection *sql.Conn, usesSimple bool) {
	t.Helper()
	declaration := "CREATE TABLE article_search(article_id,title,abstract_text,doi,pmid,authors,journal_title)"
	if usesSimple {
		if err := storage.LoadSimple(connection); err != nil {
			t.Fatal(err)
		}
		declaration = "CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,tokenize='simple 0')"
	}
	if _, err := connection.ExecContext(ctx, declaration+"; INSERT INTO article_search(rowid,article_id,title) VALUES(99,99,'stale')"); err != nil {
		t.Fatal(err)
	}
}

// assertProjectedRow checks projection values without weakening canonical storage class handling.
func assertProjectedRow(t *testing.T, ctx context.Context, connection *sql.Conn, expectedTitle, expectedAuthors, expectedJournal string) {
	t.Helper()
	var title, authors, journal, abstract, doi, pmid string
	if err := connection.QueryRowContext(ctx, "SELECT title,authors,journal_title,abstract_text,doi,pmid FROM article_search WHERE rowid=1").Scan(&title, &authors, &journal, &abstract, &doi, &pmid); err != nil {
		t.Fatal(err)
	}
	if title != expectedTitle || authors != expectedAuthors || journal != expectedJournal || abstract != "" || doi != "" || pmid != "" {
		t.Fatalf("projection changed: %q %q %q %q %q %q", title, authors, journal, abstract, doi, pmid)
	}
}

// TestRebuildPreservesCanonicalRowsAndNativeProjection checks both real native and plain roles.
func TestRebuildPreservesCanonicalRowsAndNativeProjection(t *testing.T) {
	for _, usesSimple := range []bool{false, true} {
		connection, ctx := searchFixture(t, usesSimple)
		if actual, err := UsesSimple(ctx, connection); err != nil || actual != usesSimple {
			t.Fatalf("actual declaration changed: %t %v", actual, err)
		}
		if err := Rebuild(ctx, connection); err != nil {
			t.Fatal(err)
		}
		title, authors, journal := "Résumé 中文!", "Émile", "JÖURNAL"
		if usesSimple {
			title, authors, journal = "Resume 中文 ", "Emile", "JOURNAL"
		}
		assertProjectedRow(t, ctx, connection, title, authors, journal)
		var canonical string
		if err := connection.QueryRowContext(ctx, "SELECT title FROM articles WHERE article_id=1").Scan(&canonical); err != nil || canonical != "Résumé 中文!" {
			t.Fatalf("canonical source changed: %q %v", canonical, err)
		}
	}
}

// TestRebuildLeavesPartialWritesForCallerRollback checks late author failure and prior projection recovery.
func TestRebuildLeavesPartialWritesForCallerRollback(t *testing.T) {
	connection, ctx := searchFixture(t, false)
	if _, err := connection.ExecContext(ctx, "UPDATE articles SET authors_json='null' WHERE article_id=2; BEGIN IMMEDIATE"); err != nil {
		t.Fatal(err)
	}
	defer connection.ExecContext(ctx, "ROLLBACK")
	if err := Rebuild(ctx, connection); !errors.Is(err, ErrInvalidAuthors) {
		t.Fatalf("late author error changed: %v", err)
	}
	assertProjectedRow(t, ctx, connection, "Résumé 中文!", "Émile", "JÖURNAL")
	if _, err := connection.ExecContext(ctx, "ROLLBACK"); err != nil {
		t.Fatal(err)
	}
	var identifier int64
	if err := connection.QueryRowContext(ctx, "SELECT rowid FROM article_search").Scan(&identifier); err != nil || identifier != 99 {
		t.Fatalf("caller rollback lost stale projection: %d %v", identifier, err)
	}
}

// TestRebuildRejectsNativeTextBeforeAuthorDecoding checks typed row validation precedence.
func TestRebuildRejectsNativeTextBeforeAuthorDecoding(t *testing.T) {
	connection, ctx := searchFixture(t, false)
	if _, err := connection.ExecContext(ctx, "UPDATE articles SET title=x'ff',authors_json='null' WHERE article_id=2; BEGIN IMMEDIATE"); err != nil {
		t.Fatal(err)
	}
	defer connection.ExecContext(ctx, "ROLLBACK")
	err := Rebuild(ctx, connection)
	if err == nil || errors.Is(err, ErrInvalidAuthors) {
		t.Fatalf("native text failure was bypassed: %v", err)
	}
	assertProjectedRow(t, ctx, connection, "Résumé 中文!", "Émile", "JÖURNAL")
}

// TestUsesSimpleMissingTableAndCanceledRebuildRetainExistingRows checks pre-deletion boundaries.
func TestUsesSimpleMissingTableAndCanceledRebuildRetainExistingRows(t *testing.T) {
	connection, ctx := searchFixture(t, false)
	canceled, cancel := context.WithCancel(ctx)
	cancel()
	if err := Rebuild(canceled, connection); !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled rebuild changed: %v", err)
	}
	var identifier int64
	if err := connection.QueryRowContext(ctx, "SELECT rowid FROM article_search").Scan(&identifier); err != nil || identifier != 99 {
		t.Fatalf("canceled rebuild deleted rows: %d %v", identifier, err)
	}
	if _, err := connection.ExecContext(ctx, "DROP TABLE article_search"); err != nil {
		t.Fatal(err)
	}
	if actual, err := UsesSimple(ctx, connection); err != nil || actual {
		t.Fatalf("missing declaration changed: %t %v", actual, err)
	}
}
