package delivery

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	storageconfig "github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func TestOriginalLegacyImportHistories(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/delivery/legacy-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct {
			Name  string
			Steps []struct {
				Files []struct{ Path, Body string }
				Now   *float64
				Sql   string
			}
			Output any
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, history := range corpus.Cases {
		t.Run(history.Name, func(t *testing.T) {
			root := t.TempDir()
			config := storageconfig.FromProjectRoot(root)
			ctx := context.Background()
			if _, err := migration.Migrate(ctx, config.AuthDbPath); err != nil {
				t.Fatal(err)
			}
			repository, err := Open(config.AuthDbPath)
			if err != nil {
				t.Fatal(err)
			}
			defer repository.Close()
			if _, err := repository.database.Exec(`INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'fixture','hash','salt',1,1),(2,'second','hash','salt',1,1)`); err != nil {
				t.Fatal(err)
			}
			observations := []any{}
			for _, step := range history.Steps {
				for _, file := range step.Files {
					path := filepath.Join(root, filepath.FromSlash(file.Path))
					if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
						t.Fatal(err)
					}
					if err := os.WriteFile(path, []byte(file.Body), 0600); err != nil {
						t.Fatal(err)
					}
				}
				if step.Sql != "" {
					if _, err := repository.database.Exec(step.Sql); err != nil {
						t.Fatal(err)
					}
				}
				now := 100.25
				if step.Now != nil {
					now = *step.Now
				}
				result, err := ImportLegacyFiles(ctx, config, now)
				var outcome any = result
				if err != nil {
					outcome = map[string]any{"error": err.Error()}
				}
				observations = append(observations, map[string]any{"outcome": outcome, "tables": snapshotDelivery(t, repository)})
				for _, file := range step.Files {
					body, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(file.Path)))
					if err != nil || string(body) != file.Body {
						t.Fatal("import changed original source bytes", err)
					}
				}
			}
			encoded, err := json.Marshal(observations)
			if err != nil {
				t.Fatal(err)
			}
			var actual any
			if err := json.Unmarshal(encoded, &actual); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(actual, history.Output) {
				expected, _ := json.Marshal(history.Output)
				t.Errorf("got %s\nwant %s", encoded, expected)
			}
		})
	}
}

func snapshotDelivery(t *testing.T, repository *Repository) map[string]any {
	t.Helper()
	result := map[string]any{}
	for _, table := range []string{"delivery_checkpoints", "delivery_runs", "delivery_run_items", "delivery_dedupe", "delivery_leases"} {
		rows, err := repository.database.Query("SELECT * FROM " + table + " ORDER BY id")
		if err != nil {
			t.Fatal(err)
		}
		columns, err := rows.Columns()
		if err != nil {
			t.Fatal(err)
		}
		values := []any{}
		for rows.Next() {
			raw := make([]any, len(columns))
			targets := make([]any, len(columns))
			for index := range targets {
				targets[index] = &raw[index]
			}
			if err := rows.Scan(targets...); err != nil {
				t.Fatal(err)
			}
			row := make([]any, len(columns))
			for index, value := range raw {
				switch value := value.(type) {
				case nil:
					row[index] = nil
				case string:
					row[index] = map[string]any{"text": value}
				case int64:
					row[index] = map[string]any{"integer": strconv.FormatInt(value, 10)}
				case float64:
					row[index] = map[string]any{"real": value}
				case []byte:
					row[index] = map[string]any{"blob": hex.EncodeToString(value)}
				default:
					t.Fatalf("unexpected SQLite class %T", value)
				}
			}
			values = append(values, row)
		}
		if err := rows.Err(); err != nil {
			t.Fatal(err)
		}
		rows.Close()
		result[table] = values
	}
	return result
}
