package delivery

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

func runRustDurable(t *testing.T, binary, filename string, steps []durableStep, isResume bool) []any {
	t.Helper()
	input, err := json.Marshal(map[string]any{"path": filename, "steps": steps, "resume": isResume})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, binary)
	command.Stdin = bytes.NewReader(append(input, '\n'))
	var diagnostic bytes.Buffer
	command.Stderr = &diagnostic
	output, err := command.Output()
	if err != nil {
		t.Fatalf("Rust observer failed: %v %s", err, diagnostic.Bytes())
	}
	var observations []any
	if err = json.Unmarshal(output, &observations); err != nil {
		t.Fatal(err)
	}
	return observations
}

func TestRustGoDurableDatabaseHandoffs(t *testing.T) {
	binary := os.Getenv("LITRADAR_DELIVERY_ORACLE")
	if binary == "" {
		t.Skip("the unified delivery runner supplies the verified original Rust observer")
	}
	data, err := os.ReadFile("../../../tests/migration/delivery/durable-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct {
			Name   string
			Steps  []durableStep
			Output []any
		}
	}
	if err = json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	selected := map[string]bool{"atomic-success": true, "atomic-unknown": true, "recovery-mixed-items-and-reservations": true, "lease-monotonic-and-expiry": true, "reservation-release-atomic": true, "manual-busy-and-unknown-quarantine": true}
	for _, history := range corpus.Cases {
		if !selected[history.Name] {
			continue
		}
		for _, startsWithRust := range []bool{true, false} {
			direction := "go-to-rust"
			if startsWithRust {
				direction = "rust-to-go"
			}
			t.Run(history.Name+"/"+direction, func(t *testing.T) {
				filename := filepath.Join(t.TempDir(), "auth.sqlite")
				ctx := context.Background()
				split := len(history.Steps) / 2
				if startsWithRust {
					observed := runRustDurable(t, binary, filename, history.Steps[:split], false)
					if !reflect.DeepEqual(observed, history.Output[:split]) {
						t.Fatal("Rust prefix changed")
					}
				} else {
					if _, err := migration.Migrate(ctx, filename); err != nil {
						t.Fatal(err)
					}
				}
				repository, err := Open(filename)
				if err != nil {
					t.Fatal(err)
				}
				if !startsWithRust {
					if _, err = repository.database.Exec(`INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'fixture','hash','salt',1,1),(2,'second','hash','salt',1,1)`); err != nil {
						repository.Close()
						t.Fatal(err)
					}
				}
				start, end := 0, split
				if startsWithRust {
					start, end = split, len(history.Steps)
				}
				for index := start; index < end; index++ {
					step := history.Steps[index]
					if sql := step.text("sql", ""); sql != "" {
						if _, err = repository.database.Exec(sql); err != nil {
							t.Fatal(err)
						}
					}
					outcome, err := performDurableStep(ctx, repository, step)
					if err != nil {
						outcome = map[string]any{"error": err.Error()}
					}
					encoded, _ := json.Marshal(map[string]any{"outcome": outcome, "tables": snapshotDelivery(t, repository)})
					var actual any
					json.Unmarshal(encoded, &actual)
					if !reflect.DeepEqual(actual, history.Output[index]) {
						t.Fatalf("Go handoff diverged at step %d", index)
					}
				}
				if err = repository.Close(); err != nil {
					t.Fatal(err)
				}
				if !startsWithRust {
					observed := runRustDurable(t, binary, filename, history.Steps[split:], true)
					if !reflect.DeepEqual(observed, history.Output[split:]) {
						t.Fatal("Rust could not continue Go state")
					}
				}
			})
		}
	}
}
