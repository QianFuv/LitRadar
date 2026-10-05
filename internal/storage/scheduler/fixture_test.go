package scheduler

import (
	"context"
	"database/sql"
	"encoding/hex"

	"fmt"

	"path/filepath"

	"testing"

	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func fixture(t *testing.T) (*Repository, string) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
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
				rows.Close()
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
					rows.Close()
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
			rows.Close()
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

func applyFixtureSql(t *testing.T, repository *Repository, statement string) {
	t.Helper()
	if statement == "" {
		return
	}
	if err := repository.auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
		defer connection.ExecContext(context.Background(), "PRAGMA ignore_check_constraints=OFF")
		_, err := connection.ExecContext(context.Background(), "PRAGMA ignore_check_constraints=ON;"+statement)
		return err
	}); err != nil {
		t.Fatal(err)
	}
}
