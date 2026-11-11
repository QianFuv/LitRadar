package scheduler

import (
	"context"
	"database/sql"
	"encoding/hex"

	"fmt"

	"path/filepath"

	"testing"

	migration "github.com/QianFuv/LitRadar/apps/server/internal/storage/migrations/auth"
)

// fixture creates a migrated scheduler repository for isolated tests and benchmarks.
func fixture(t testing.TB) (*Repository, string) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := repository.Close(); err != nil {
			t.Errorf("repository.Close: %v", err)
		}
	})
	return repository, filename
}

func snapshot(t *testing.T, repository *Repository) map[string]any {
	t.Helper()
	tables := map[string]any{}
	err := repository.auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
		for _, table := range []struct{ name, order string }{{"scheduled_tasks", "id"}, {"scheduled_task_runs", "id"}, {"scheduler_state", "id"}, {"scheduler_workers", "worker_id"}, {"service_heartbeats", "service,instance_id"}} {
			rows, err := connection.QueryContext(context.Background(), "SELECT * FROM "+table.name+" ORDER BY "+table.order)
			if err != nil {
				return err
			}
			columns, err := rows.Columns()
			if err != nil {
				if err := rows.Close(); err != nil {
					t.Errorf("rows.Close: %v", err)
				}
				return err
			}
			records := [][]any{}
			for rows.Next() {
				values := make([]any, len(columns))
				targets := make([]any, len(values))
				for index := range values {
					targets[index] = &values[index]
				}
				if err := rows.Scan(targets...); err != nil {
					if err := rows.Close(); err != nil {
						t.Errorf("rows.Close: %v", err)
					}
					return err
				}
				for index, value := range values {
					switch value := value.(type) {
					case nil:
					case int64:
						values[index] = map[string]any{"integer": fmt.Sprint(value)}
					case float64:
						values[index] = map[string]any{"real": value}
					case string:
						values[index] = map[string]any{"text": value}
					case []byte:
						values[index] = map[string]any{"blob": hex.EncodeToString(value)}
					default:
						t.Fatalf("unexpected SQLite value %T", value)
					}
				}
				records = append(records, values)
			}
			err = rows.Err()
			if err := rows.Close(); err != nil {
				t.Errorf("rows.Close: %v", err)
			}
			if err != nil {
				return err
			}
			tables[table.name] = records
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return tables
}

func applyFixtureSQL(t *testing.T, repository *Repository, statement string) {
	t.Helper()
	if statement == "" {
		return
	}
	if err := repository.auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
		defer func() {
			if _, err := connection.ExecContext(context.Background(), "PRAGMA ignore_check_constraints=OFF"); err != nil {
				t.Errorf("connection.ExecContext: %v", err)
			}
		}()
		_, err := connection.ExecContext(context.Background(), "PRAGMA ignore_check_constraints=ON;"+statement)
		return err
	}); err != nil {
		t.Fatal(err)
	}
}
