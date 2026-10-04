package scheduler

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

type step map[string]json.RawMessage

func (value step) text(key, fallback string) string {
	var result *string
	json.Unmarshal(value[key], &result)
	if result != nil {
		return *result
	}
	return fallback
}
func (value step) integer(key string, fallback int64) int64 {
	var result *int64
	json.Unmarshal(value[key], &result)
	if result != nil {
		return *result
	}
	return fallback
}
func (value step) real(key string, fallback float64) float64 {
	var result *float64
	json.Unmarshal(value[key], &result)
	if result != nil {
		return *result
	}
	return fallback
}
func (value step) flag(key string, fallback bool) bool {
	var result *bool
	json.Unmarshal(value[key], &result)
	if result != nil {
		return *result
	}
	return fallback
}
func optional[T any](value step, key string) *T {
	var result *T
	json.Unmarshal(value[key], &result)
	return result
}

type exactNumber struct{ Number string }

func normalizeNumbers(value any) any {
	switch value := value.(type) {
	case json.Number:
		number, ok := new(big.Rat).SetString(string(value))
		if !ok {
			panic("invalid oracle number")
		}
		return exactNumber{number.RatString()}
	case []any:
		for index := range value {
			value[index] = normalizeNumbers(value[index])
		}
		return value
	case map[string]any:
		for key, entry := range value {
			value[key] = normalizeNumbers(entry)
		}
		return value
	default:
		return value
	}
}

func perform(ctx context.Context, repository *Repository, value step) (any, error) {
	id, run, now, lease, worker := value.integer("task", 1), value.integer("run", 1), value.real("now", 100.25), value.real("lease", 10), value.text("worker", "worker")
	switch value.text("op", "") {
	case "create":
		job := domain.Job{Kind: "index"}
		if raw, exists := value["job"]; exists {
			if err := json.Unmarshal(raw, &job); err != nil {
				return nil, err
			}
		}
		task, err := repository.Create(ctx, domain.Create{Name: value.text("name", "fixture"), Job: job, Cron: value.text("cron", "* * * * *"), Timezone: value.text("timezone", "UTC"), TimeoutSeconds: uint64(value.integer("timeout", 60)), Coalesce: value.flag("coalesce", true), Enabled: value.flag("enabled", true)}, nil, nil)
		if err != nil {
			return nil, err
		}
		err = repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
			_, err := connection.ExecContext(ctx, "UPDATE scheduled_tasks SET created_at=?1,updated_at=?1 WHERE id=?2", now, task.Id)
			return err
		})
		return task.Id, err
	case "update":
		task, err := repository.Update(ctx, domain.Update{TaskId: id, Name: optional[string](value, "name"), Job: optional[domain.Job](value, "job"), Cron: optional[string](value, "cron"), Timezone: optional[string](value, "timezone"), TimeoutSeconds: optional[uint64](value, "timeout"), Coalesce: optional[bool](value, "coalesce"), Enabled: optional[bool](value, "enabled")}, nil, nil)
		if err != nil || task == nil {
			return nil, err
		}
		err = repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
			_, err := connection.ExecContext(ctx, "UPDATE scheduled_tasks SET updated_at=? WHERE id=?", now, id)
			return err
		})
		return task.Id, err
	case "get":
		return repository.Get(ctx, id)
	case "list":
		return repository.List(ctx)
	case "delete":
		return repository.Delete(ctx, id, nil, nil)
	case "enqueue":
		task, err := repository.Get(ctx, id)
		if err != nil {
			return nil, err
		}
		var slots []int64
		if err := json.Unmarshal(value["slots"], &slots); err != nil {
			return nil, err
		}
		return repository.Enqueue(ctx, *task, slots)
	case "manual":
		admission, err := repository.ClaimManual(ctx, id, worker, now, lease)
		if err != nil {
			return nil, err
		}
		if admission.Claim == nil {
			return map[string]any{"status": admission.Status}, nil
		}
		claim := admission.Claim
		return map[string]any{"status": admission.Status, "run_id": claim.RunId, "scheduled_for": claim.ScheduledFor, "worker_id": claim.WorkerId, "task": claim.Task}, nil
	case "claim":
		capacity := ^uint64(0)
		if pointer := optional[uint64](value, "capacity"); pointer != nil {
			capacity = *pointer
		}
		return repository.ClaimReady(ctx, worker, now, lease, capacity)
	case "start":
		return repository.StartRun(ctx, run, worker, now, lease)
	case "renew":
		return repository.HeartbeatRun(ctx, run, worker, now, lease)
	case "finish":
		task, err := repository.Get(ctx, id)
		if err != nil {
			return nil, err
		}
		if task == nil {
			task = &domain.Task{Id: id}
		}
		return repository.FinishRun(ctx, Claim{RunId: run, WorkerId: worker, Task: *task}, domain.State(value.text("status", "success")), value.text("summary", ""), now)
	case "heartbeat":
		return "ok", repository.RecordHeartbeat(ctx, worker, now)
	case "check":
		return "ok", repository.RecordCheck(ctx, now)
	case "cursor":
		return repository.LastCheckedAt(ctx)
	case "status":
		return repository.Status(ctx, now, value.real("window", 90), uint64(value.integer("limit", 10)))
	case "noop":
		return "ok", nil
	}
	panic("invalid observer step")
}

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

func observation(t *testing.T, repository *Repository, value step) any {
	t.Helper()
	applyFixtureSql(t, repository, value.text("sql", ""))
	outcome, err := perform(context.Background(), repository, value)
	if err != nil {
		message := "storage"
		for _, prefix := range []string{"metadata file must", "database must", "max_candidates must", "timezone must", "timeout_seconds must", "Legacy task requires", "A legacy scheduled task", "Scheduled run result"} {
			if strings.HasPrefix(err.Error(), prefix) {
				message = err.Error()
			}
		}
		outcome = map[string]any{"error": message}
	}
	encoded, err := json.Marshal(map[string]any{"outcome": outcome, "tables": snapshot(t, repository)})
	if err != nil {
		t.Fatal(err)
	}
	return decodeExact(t, encoded)
}
func decodeExact(t *testing.T, data []byte) any {
	t.Helper()
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	var result any
	if err := decoder.Decode(&result); err != nil {
		t.Fatal(err)
	}
	return normalizeNumbers(result)
}

func TestOriginalRustSchedulerHistories(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/scheduler/storage-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct {
			Name     string
			Steps    []step
			Expected []json.RawMessage
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, history := range corpus.Cases {
		t.Run(history.Name, func(t *testing.T) {
			repository, _ := fixture(t)
			for index, value := range history.Steps {
				actual := observation(t, repository, value)
				expected := decodeExact(t, history.Expected[index])
				if !reflect.DeepEqual(actual, expected) {
					actualBytes, _ := json.Marshal(actual)
					t.Fatalf("step %d %s: got %s\nwant %s", index, value.text("op", ""), actualBytes, history.Expected[index])
				}
			}
		})
	}
}
