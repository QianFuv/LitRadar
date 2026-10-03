package index

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type historical struct {
	Version   int
	File      string
	Canonical [][]string
	Search    [][]string
}

func fixtures(t *testing.T) []historical {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "tests", "migration", "storage", "index-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var result struct{ Cases []historical }
	if err := json.Unmarshal(data, &result); err != nil {
		t.Fatal(err)
	}
	return result.Cases
}

func copyFixture(t *testing.T, fixture historical) (string, []byte) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "tests", "migration", "storage", "fixtures", fixture.File))
	if err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(t.TempDir(), "content.sqlite")
	if err := os.WriteFile(filename, data, 0600); err != nil {
		t.Fatal(err)
	}
	return filename, data
}

func inspectConnection(t *testing.T, filename string, read func(*sql.Conn)) {
	t.Helper()
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	read(connection)
}

func TestActualRustHistoricalLayoutsPreserveCanonicalDataAndSearch(t *testing.T) {
	ctx := context.Background()
	for _, fixture := range fixtures(t) {
		for _, isPreflight := range []bool{false, true} {
			t.Run(strconv.Itoa(fixture.Version)+"/preflight="+strconv.FormatBool(isPreflight), func(t *testing.T) {
				filename, before := copyFixture(t, fixture)
				if isPreflight {
					scope, err := Preflight(ctx, filename)
					if err != nil {
						t.Fatal(err)
					}
					if fixture.Version >= 6 {
						after, err := os.ReadFile(filename)
						if err != nil || !bytes.Equal(before, after) || scope != "schema_only" {
							t.Fatal("ordinary startup changed a supported database", err)
						}
					} else if scope != "full_integrity" {
						t.Fatal(scope)
					}
				}
				summary, err := Migrate(ctx, filename)
				if err != nil || summary.ToVersion != 9 {
					t.Fatalf("%+v %v", summary, err)
				}
				inspectConnection(t, filename, func(connection *sql.Conn) {
					if err := indexschema.Validate(ctx, connection, 9); err != nil {
						t.Fatal(err)
					}
					for index, statement := range canonicalQueries {
						if actual := stringsFromQuery(t, connection, statement); !reflect.DeepEqual(actual, fixture.Canonical[index]) {
							t.Fatalf("canonical table%d changed: %v != %v", index, actual, fixture.Canonical[index])
						}
					}
					for index, query := range []string{"article", "abstract", "doi:article", "authors:Author", `journal_title:"Journal One"`} {
						if actual := stringsFromQuery(t, connection, "SELECT CAST(rowid AS TEXT) FROM article_search WHERE article_search MATCH ? ORDER BY rowid", query); !reflect.DeepEqual(actual, fixture.Search[index]) {
							t.Fatalf("search %s changed: %v", query, actual)
						}
					}
					var retractions int
					if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM article_retraction_dois").Scan(&retractions); err != nil || retractions != 0 {
						t.Fatal("legacy scalar retraction was invented as a plural relationship", err)
					}
				})
			})
		}
	}
}

func stringsFromQuery(t *testing.T, connection *sql.Conn, statement string, args ...any) []string {
	t.Helper()
	rows, err := connection.QueryContext(context.Background(), statement, args...)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	result := []string{}
	for rows.Next() {
		var value string
		if err := rows.Scan(&value); err != nil {
			t.Fatal(err)
		}
		result = append(result, value)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return result
}

func TestFreshEmptyAndLegacyDatabaseEntrances(t *testing.T) {
	ctx := context.Background()
	for _, isEmptyFile := range []bool{false, true} {
		filename := filepath.Join(t.TempDir(), "nested", "content.sqlite")
		if isEmptyFile {
			if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filename, nil, 0600); err != nil {
				t.Fatal(err)
			}
		}
		if result, err := Migrate(ctx, filename); err != nil || result != (Summary{0, 9}) {
			t.Fatalf("%+v %v", result, err)
		}
	}
	for _, version := range []int{0, 1, 2, 3, 10} {
		filename := filepath.Join(t.TempDir(), "legacy.sqlite")
		database, err := storage.OpenMigration(filename)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := database.Exec("CREATE TABLE legacy(value TEXT); PRAGMA user_version=" + strconv.Itoa(version)); err != nil {
			t.Fatal(err)
		}
		database.Close()
		before, err := os.ReadFile(filename)
		if err != nil {
			t.Fatal(err)
		}
		_, failure := Migrate(ctx, filename)
		if failure == nil {
			t.Fatal("unsupported database accepted")
		}
		var future UnsupportedVersion
		var rebuild RebuildRequired
		if version == 10 && !errors.As(failure, &future) || version < 10 && !errors.As(failure, &rebuild) {
			t.Fatalf("wrong classification: %v", failure)
		}
		after, err := os.ReadFile(filename)
		if err != nil || !bytes.Equal(before, after) {
			t.Fatal("rejected file changed")
		}
	}
}

var canonicalQueries = []string{
	"SELECT json_array(journal_id,catalog_id,title,title_aliases_json,issns_json,issn,eissn,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating) FROM journals ORDER BY journal_id",
	"SELECT json_array(issue_id,journal_id,publication_year,title,volume,number,date) FROM issues ORDER BY issue_id",
	"SELECT json_array(article_id,journal_id,issue_id,title,publication_year,date,authors_json,start_page,end_page,abstract_text,doi,pmid,open_access,in_press) FROM articles ORDER BY article_id",
	"SELECT json_array(identity_kind,identity_value,article_id) FROM article_identity_keys ORDER BY identity_kind,identity_value",
	"SELECT json_array(article_id,journal_id,issue_id,publication_year,date,open_access,in_press,doi,pmid,area) FROM article_listing ORDER BY article_id",
	"SELECT json_array(event_id,content_revision,article_id,change_kind,journal_id,issue_id,in_press,created_at) FROM article_change_events ORDER BY event_id",
}
