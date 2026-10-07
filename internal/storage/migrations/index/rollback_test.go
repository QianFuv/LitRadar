package index

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func mutateFixture(t *testing.T, filename, statement string) {
	t.Helper()
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec(statement); err != nil {
		t.Fatal(err)
	}
}

func databaseSnapshot(t *testing.T, filename string) string {
	t.Helper()
	result := map[string]any{}
	inspectConnection(t, filename, func(connection *sql.Conn) {
		ctx := context.Background()
		var version int
		if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
			t.Fatal(err)
		}
		result["version"] = version
		result["schema"] = stringsFromQuery(t, connection, "SELECT type||'|'||name||'|'||coalesce(sql,'') FROM sqlite_schema ORDER BY type,name")
		tables := stringsFromQuery(t, connection, "SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
		for _, table := range tables {
			rows, err := connection.QueryContext(ctx, `SELECT * FROM "`+strings.ReplaceAll(table, `"`, `""`)+`" ORDER BY 1`)
			if err != nil {
				t.Fatal(err)
			}
			columns, err := rows.Columns()
			if err != nil {
				t.Fatal(err)
			}
			values := [][]any{}
			for rows.Next() {
				row := make([]any, len(columns))
				destinations := make([]any, len(columns))
				for index := range row {
					destinations[index] = &row[index]
				}
				if err := rows.Scan(destinations...); err != nil {
					t.Fatal(err)
				}
				values = append(values, row)
			}
			if err := rows.Err(); err != nil {
				t.Fatal(err)
			}
			rows.Close()
			result[table] = values
		}
	})
	encoded, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	return string(encoded)
}

func TestIdentityFailuresRollBackHistoricalSchema(t *testing.T) {
	for _, test := range []struct {
		name, statement string
		expected        error
	}{
		{"catalog", "UPDATE journals SET catalog_id='A' WHERE journal_id=1", ErrIdentityState},
		{"issn-check", `UPDATE journals SET issns_json='["1234-5678"]' WHERE journal_id=1`, ErrIdentityState},
		{"issn-json", `UPDATE journals SET issns_json='[1]' WHERE journal_id=1`, ErrIdentityState},
		{"issn-null", `UPDATE journals SET issns_json='null' WHERE journal_id=1`, ErrIdentityState},
		{"conflict", `UPDATE journals SET issns_json='["1234-5679"]' WHERE journal_id=2`, ErrIdentityConflict},
		{"typed-before-semantic", `UPDATE journals SET catalog_id='A' WHERE journal_id=1; UPDATE journals SET issns_json=x'5b5d' WHERE journal_id=2`, nil},
	} {
		t.Run(test.name, func(t *testing.T) {
			filename, _ := copyFixture(t, fixtures(t)[0])
			mutateFixture(t, filename, test.statement)
			before := databaseSnapshot(t, filename)
			_, err := Migrate(context.Background(), filename)
			if test.expected != nil && !errors.Is(err, test.expected) {
				t.Fatalf("expected %v, got %v", test.expected, err)
			}
			if test.expected == nil && (err == nil || errorKind(err) != "sqlite") {
				t.Fatalf("typed read must precede semantic validation: %v", err)
			}
			if after := databaseSnapshot(t, filename); after != before {
				t.Fatal("failed identity migration changed schema or data")
			}
		})
	}
}

func TestLateMalformedAuthorsRollBackWholeMigration(t *testing.T) {
	for _, fixture := range fixtures(t) {
		if fixture.Version != 4 && fixture.Version != 6 && fixture.Version != 8 {
			continue
		}
		t.Run(fmt.Sprint(fixture.Version), func(t *testing.T) {
			filename, _ := copyFixture(t, fixture)
			mutateFixture(t, filename, `INSERT INTO articles(article_id,journal_id,title,authors_json) VALUES(999,1,'Late corrupt author','{');`)
			before := databaseSnapshot(t, filename)
			_, err := Migrate(context.Background(), filename)
			if !errors.Is(err, ErrInvalidQuery) {
				t.Fatalf("expected invalid authors, got %v", err)
			}
			if !reflect.DeepEqual(before, databaseSnapshot(t, filename)) {
				t.Fatal("late FTS failure committed part of the migration")
			}
		})
	}
}

// TestRepeatedJournalIssnsRetainOneOwner covers duplicate legacy identity sources.
func TestRepeatedJournalIssnsRetainOneOwner(t *testing.T) {
	filename, _ := copyFixture(t, fixtures(t)[0])
	mutateFixture(t, filename, `UPDATE journals SET issns_json='["1234-5679","0378-5955","1234-5679"]',issn='1234-5679',eissn='0378-5955' WHERE journal_id=1`)
	if summary, err := Migrate(context.Background(), filename); err != nil || summary != (Summary{4, 9}) {
		t.Fatalf("repeated identifiers rejected: %+v %v", summary, err)
	}
	inspectConnection(t, filename, func(connection *sql.Conn) {
		actual := stringsFromQuery(t, connection, "SELECT identity_kind||'|'||identity_value||'|'||canonical_catalog_id FROM journal_identity_keys ORDER BY identity_kind,identity_value")
		expected := []string{"catalog_id|journal-one|journal-one", "catalog_id|journal-two|journal-two", "issn|0378-5955|journal-one", "issn|1234-5679|journal-one", "issn|2049-3630|journal-two"}
		if !reflect.DeepEqual(actual, expected) {
			t.Fatalf("identity ownership: %v", actual)
		}
	})
}
