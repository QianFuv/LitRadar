package cfp

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func decodeExact(t *testing.T, data []byte) any {
	t.Helper()
	var result any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := decoder.Decode(&result); err != nil {
		t.Fatal(err)
	}
	return result
}

func observation(t *testing.T, repository *Repository, value step) any {
	t.Helper()
	if query := value.text("sql", ""); query != "" {
		if _, err := repository.database.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	outcome, err := perform(context.Background(), repository, value)
	if err != nil {
		outcome = map[string]string{"error": classify(err)}
	}
	data, err := json.Marshal(map[string]any{"outcome": outcome, "tables": snapshot(t, repository.database)})
	if err != nil {
		t.Fatal(err)
	}
	return decodeExact(t, data)
}

func rustHistory(t *testing.T, binary, path string, steps []step, isResume bool) []any {
	t.Helper()
	input, _ := json.Marshal(map[string]any{"path": path, "steps": steps, "resume": isResume})
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, binary)
	command.Stdin = bytes.NewReader(append(input, '\n'))
	var diagnostic bytes.Buffer
	command.Stderr = &diagnostic
	output, err := command.Output()
	if err != nil {
		t.Fatalf("original Rust observer failed: %v %s", err, diagnostic.String())
	}
	result, ok := decodeExact(t, output).([]any)
	if !ok {
		t.Fatal("invalid observer history")
	}
	return result
}

func TestRustGoCfpDatabaseHandoffs(t *testing.T) {
	binary := os.Getenv("LITRADAR_CFP_ORACLE")
	if binary == "" {
		t.Skip("unified cfp runner supplies the verified original Rust executable")
	}
	data, err := os.ReadFile("../../../tests/migration/cfp/storage-vectors.json")
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
	if json.Unmarshal(data, &corpus) != nil {
		t.Fatal("invalid corpus")
	}
	selected := map[string]bool{"refresh-and-reimport": true, "active-generation-replaced": true, "full-partial-1": true, "expired-fail-retains-data": true, "verified-empty": true, "immutable-seed": true}
	for _, history := range corpus.Cases {
		if !selected[history.Name] {
			continue
		}
		for _, isRustFirst := range []bool{true, false} {
			direction := "go-to-rust"
			if isRustFirst {
				direction = "rust-to-go"
			}
			t.Run(history.Name+"/"+direction, func(t *testing.T) {
				filename := filepath.Join(t.TempDir(), "auth.sqlite")
				split := len(history.Steps) / 2
				expected := []any{}
				for _, raw := range history.Expected {
					expected = append(expected, decodeExact(t, raw))
				}
				if isRustFirst {
					if actual := rustHistory(t, binary, filename, history.Steps[:split], false); !reflect.DeepEqual(actual, expected[:split]) {
						t.Fatal("original Rust prefix changed")
					}
				} else {
					if _, err := migration.Migrate(context.Background(), filename); err != nil {
						t.Fatal(err)
					}
				}
				repository, err := Open(filename)
				if err != nil {
					t.Fatal(err)
				}
				defer repository.Close()
				first, last := 0, split
				if isRustFirst {
					first, last = split, len(history.Steps)
				}
				for index := first; index < last; index++ {
					if actual := observation(t, repository, history.Steps[index]); !reflect.DeepEqual(actual, expected[index]) {
						t.Fatalf("Go divergence at step %d", index)
					}
				}
				if err := repository.Close(); err != nil {
					t.Fatal(err)
				}
				if !isRustFirst {
					if actual := rustHistory(t, binary, filename, history.Steps[split:], true); !reflect.DeepEqual(actual, expected[split:]) {
						t.Fatal("Rust could not continue Go database")
					}
				}
			})
		}
	}
}
