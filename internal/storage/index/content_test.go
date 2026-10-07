package index

import (
	"context"
	"database/sql"
	contentfixture "github.com/QianFuv/LitRadar/internal/testkit/content"

	"errors"

	"path/filepath"

	"strconv"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

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
		values := snapshotTableRows(t, connection, table)
		tables[table] = values
	}
	return tables
}

func TestContentRuntimeVersionBoundary(t *testing.T) {
	ctx := context.Background()
	for version := 4; version <= 9; version++ {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "content.sqlite")
			if err := contentfixture.Create(path, version); err != nil {
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

func snapshotTableRows(t *testing.T, connection *sql.Conn, table string) [][]any {
	t.Helper()
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
		normalizeSnapshotRecord(record)
		values = append(values, record)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	rows.Close()
	return values
}

func normalizeSnapshotRecord(record []any) {
	for position, value := range record {
		switch typed := value.(type) {
		case int64:
			record[position] = map[string]any{"integer": strconv.FormatInt(typed, 10)}
		case float64:
			record[position] = map[string]any{"real": strconv.FormatFloat(typed, 'g', -1, 64)}
		case []byte:
			record[position] = map[string]any{"blob": snapshotBlob(typed)}
		}
	}
}

func snapshotBlob(typed []byte) []int {
	bytes := []int{}
	for _, value := range typed {
		bytes = append(bytes, int(value))
	}
	return bytes
}
