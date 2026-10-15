package index

import (
	"context"
	"database/sql"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
)

// TestVersionNineIdentifierUpgradePreservesStoredSearch verifies an index-only upgrade and idempotency.
func TestVersionNineIdentifierUpgradePreservesStoredSearch(t *testing.T) {
	filename, _ := copyFixture(t, historical{Version: 9})
	mutateFixture(t, filename, `DELETE FROM article_search;
INSERT INTO article_search(rowid,article_id,title) VALUES(1001,1001,'Frozen stored projection');`)
	var before []string
	inspectConnection(t, filename, func(connection *sql.Conn) {
		before = stringsFromQuery(t, connection, "SELECT printf('%d:%s',id,hex(block)) FROM article_search_data ORDER BY id")
	})
	ctx := context.Background()
	if summary, err := Migrate(ctx, filename); err != nil || summary != (Summary{9, indexschema.Version}) {
		t.Fatalf("upgrade: %+v, %v", summary, err)
	}
	inspectConnection(t, filename, func(connection *sql.Conn) {
		after := stringsFromQuery(t, connection, "SELECT printf('%d:%s',id,hex(block)) FROM article_search_data ORDER BY id")
		if !reflect.DeepEqual(before, after) {
			t.Fatal("identifier upgrade rebuilt the stored FTS publication")
		}
		matches := stringsFromQuery(t, connection, "SELECT rowid FROM article_search WHERE article_search MATCH 'Frozen'")
		if !reflect.DeepEqual(matches, []string{"1001"}) {
			t.Fatal("stored search terms changed", matches)
		}
		for _, field := range []string{"doi", "pmid"} {
			columns := stringsFromQuery(t, connection, "SELECT name FROM pragma_index_info('idx_article_listing_"+field+"') ORDER BY seqno")
			if !reflect.DeepEqual(columns, []string{field, "date", "article_id"}) {
				t.Fatal("lookup index does not cover ordered identifiers", field, columns)
			}
		}
	})
	if summary, err := Migrate(ctx, filename); err != nil || summary != (Summary{indexschema.Version, indexschema.Version}) {
		t.Fatalf("idempotent upgrade: %+v, %v", summary, err)
	}
}

// TestIdentifierIndexFailureRollsBackVersionAndFirstIndex rejects partial index publication.
func TestIdentifierIndexFailureRollsBackVersionAndFirstIndex(t *testing.T) {
	filename, _ := copyFixture(t, historical{Version: 9})
	before := databaseSnapshot(t, filename)
	database, err := storage.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	ctx := context.Background()
	connection, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	hasDeniedSecondIndex := false
	if err := connection.Raw(func(value any) error {
		value.(*native.SQLiteConn).RegisterAuthorizer(func(operation int, name, table, database string) int {
			if operation == native.SQLITE_CREATE_INDEX && name == "idx_article_listing_pmid" {
				hasDeniedSecondIndex = true
				return native.SQLITE_DENY
			}
			return native.SQLITE_OK
		})
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	summary, failure := migrateContentConnection(ctx, connection, 9, true)
	if err := connection.Raw(func(value any) error {
		value.(*native.SQLiteConn).RegisterAuthorizer(nil)
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if failure == nil || !hasDeniedSecondIndex || summary != (Summary{9, 9}) {
		t.Fatalf("second-index failure was not retained: %+v, %v, denied=%t", summary, failure, hasDeniedSecondIndex)
	}
	connection.Close()
	database.Close()
	if after := databaseSnapshot(t, filename); after != before {
		t.Fatal("failed index creation changed schema, version, or content")
	}
}
