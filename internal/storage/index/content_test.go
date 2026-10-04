package index

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestOriginalRustContentTransactions(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/index/content-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Name       string
				Operations []struct {
					Op, Sql, Revision string
					Catalog           domain.JournalCatalogEntry
					Catalogs          []domain.JournalCatalogEntry
					Batch             domain.ProviderBatch
				}
			}
			Expected json.RawMessage
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, item := range corpus.Observations {
		t.Run(item.Input.Name, func(t *testing.T) {
			ctx := context.Background()
			connection, err := OpenContent(ctx, filepath.Join(t.TempDir(), "content.sqlite"))
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			operations := []any{}
			for _, operation := range item.Input.Operations {
				var value any
				var err error
				switch operation.Op {
				case "sql":
					_, err = connection.ExecContext(ctx, operation.Sql)
					value = map[string]any{"ok": true}
				case "reconcile":
					err = ReconcileCatalogIdentities(ctx, connection.Conn, operation.Catalogs)
					value = map[string]any{"ok": true}
				case "write":
					value, err = WriteContentBatch(ctx, connection.Conn, operation.Catalog, operation.Batch, operation.Revision, "2026-10-04T00:00:00Z")
				default:
					t.Fatal(operation.Op)
				}
				if err != nil {
					value = map[string]any{"error": err.Error()}
				}
				operations = append(operations, value)
			}
			actual := map[string]any{"operations": operations, "tables": contentSnapshot(t, connection.Conn)}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			if err := json.Unmarshal(encoded, &got); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(item.Expected, &want); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("actual=%s\nexpected=%s", encoded, item.Expected)
			}
		})
	}
}

func contentSnapshot(t *testing.T, connection *sql.Conn) map[string]any {
	return selectedSnapshot(t, connection, []string{"journals", "journal_identity_keys", "issues", "articles", "article_retraction_dois", "article_identity_keys", "article_listing", "article_search", "article_change_events"})
}

func selectedSnapshot(t *testing.T, connection *sql.Conn, selected []string) map[string]any {
	t.Helper()
	tables := map[string]any{}
	for _, table := range selected {
		var exists int
		if err := connection.QueryRowContext(context.Background(), "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)", table).Scan(&exists); err != nil {
			t.Fatal(err)
		}
		if exists == 0 {
			continue
		}
		rows, err := connection.QueryContext(context.Background(), "SELECT * FROM "+table+" ORDER BY 1,2")
		if err != nil {
			t.Fatal(err)
		}
		columns, err := rows.Columns()
		if err != nil {
			t.Fatal(err)
		}
		values := [][]any{}
		for rows.Next() {
			record := make([]any, len(columns))
			destinations := make([]any, len(columns))
			for position := range record {
				destinations[position] = &record[position]
			}
			if err := rows.Scan(destinations...); err != nil {
				t.Fatal(err)
			}
			for position, value := range record {
				switch typed := value.(type) {
				case int64:
					record[position] = map[string]any{"integer": strconv.FormatInt(typed, 10)}
				case float64:
					record[position] = map[string]any{"real": strconv.FormatFloat(typed, 'g', -1, 64)}
				case []byte:
					bytes := []int{}
					for _, value := range typed {
						bytes = append(bytes, int(value))
					}
					record[position] = map[string]any{"blob": bytes}
				}
			}
			values = append(values, record)
		}
		if err := rows.Err(); err != nil {
			t.Fatal(err)
		}
		rows.Close()
		tables[table] = values
	}
	return tables
}

func TestContentRuntimeVersionBoundary(t *testing.T) {
	ctx := context.Background()
	for version := 4; version <= 9; version++ {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "content.sqlite")
			fixture := "../../../tests/migration/storage/fixtures/content-v" + strconv.Itoa(version) + ".sqlite.fixture"
			body, err := os.ReadFile(fixture)
			if err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(path, body, 0600); err != nil {
				t.Fatal(err)
			}
			connection, err := OpenContent(ctx, path)
			if version < 6 {
				var rebuild *RebuildRequired
				if !errors.As(err, &rebuild) || rebuild.FoundVersion != int64(version) {
					t.Fatalf("error=%v", err)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			var stored sqlite.Integer
			if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&stored); err != nil || int(stored) != version {
				t.Fatalf("version=%v err=%v", stored, err)
			}
		})
	}
}
