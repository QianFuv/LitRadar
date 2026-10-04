package delivery

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/recommend"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func appendSyntheticMessage(filename string) error {
	file, err := os.OpenFile(filename, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	if _, err = file.WriteString("message\n"); err == nil {
		err = file.Sync()
	}
	closeError := file.Close()
	if err != nil {
		return err
	}
	return closeError
}

func TestDeliveryCrashChild(t *testing.T) {
	encoded := os.Getenv("LITRADAR_DELIVERY_CRASH_CONFIG")
	if encoded == "" {
		t.Skip("isolated crash-test child")
	}
	var input struct{ AuthPath, IndexPath, Barrier, Ready, Ledger string }
	if err := json.Unmarshal([]byte(encoded), &input); err != nil {
		t.Fatal(err)
	}
	pause := func(boundary string) {
		if boundary != input.Barrier {
			return
		}
		if err := os.WriteFile(input.Ready, []byte(boundary), 0600); err != nil {
			t.Fatal(err)
		}
		time.Sleep(30 * time.Second)
		t.Fatal("parent did not kill child at the selected boundary")
	}
	repository, err := store.Open(input.AuthPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	accounts, err := auth.Open(input.AuthPath)
	if err != nil {
		t.Fatal(err)
	}
	defer accounts.Close()
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	config := RunConfig{AuthDbPath: input.AuthPath, IndexDbPath: input.IndexPath, SecretCodec: codec, DbName: "fixture.sqlite", Workflow: store.WorkflowNotify, Mode: store.RunModeExecute, Trigger: store.TriggerKindScheduled}
	folders := favorites.New(accounts)
	engine := deliveryEngine{repository: repository, settings: settings.New(accounts, codec), selectArticles: func(context.Context, recommend.SelectionRequest) (recommend.SelectionOutcome, error) {
		return recommend.SelectionOutcome{Accepted: []domain.RankedSelection{{ArticleId: 7, Score: 1}}}, nil
	}, writeFavorites: func(ctx context.Context, writes []FavoriteWritePlan) error {
		pause("reserved")
		if err := executeFavoriteWrites(ctx, folders, writes); err != nil {
			return err
		}
		pause("favorites-written")
		return nil
	}, send: func(context.Context, PushplusMessage) (string, error) {
		pause("sending")
		if err := appendSyntheticMessage(input.Ledger); err != nil {
			return "", err
		}
		pause("message-written")
		return "message", nil
	}}
	if _, err = engine.execute(context.Background(), config, nil, manifestFor("crash-run", 7)); err != nil {
		t.Fatal(err)
	}
	pause("run-finalized")
}

func TestDeliveryProcessKillPreservesEffectsAndQuarantinesAmbiguity(t *testing.T) {
	for _, boundary := range []string{"reserved", "favorites-written", "sending", "message-written", "run-finalized"} {
		t.Run(boundary, func(t *testing.T) {
			engine, database, config := workflowFixture(t, store.WorkflowNotify)
			directory := t.TempDir()
			ready, ledger := filepath.Join(directory, "ready"), filepath.Join(directory, "messages")
			input, _ := json.Marshal(map[string]string{"AuthPath": config.AuthDbPath, "IndexPath": config.IndexDbPath, "Barrier": boundary, "Ready": ready, "Ledger": ledger})
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			command := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestDeliveryCrashChild$", "-test.timeout=35s")
			command.Env = append(os.Environ(), "LITRADAR_DELIVERY_CRASH_CONFIG="+string(input))
			output := filepath.Join(directory, "child.log")
			logFile, err := os.Create(output)
			if err != nil {
				t.Fatal(err)
			}
			defer logFile.Close()
			command.Stdout = logFile
			command.Stderr = logFile
			if err = command.Start(); err != nil {
				t.Fatal(err)
			}
			done := make(chan error, 1)
			go func() { done <- command.Wait() }()
			t.Cleanup(func() {
				if command.ProcessState == nil {
					command.Process.Kill()
					<-done
				}
			})
			deadline := time.Now().Add(15 * time.Second)
			for {
				if data, err := os.ReadFile(ready); err == nil && string(data) == boundary {
					break
				}
				select {
				case err := <-done:
					data, _ := os.ReadFile(output)
					t.Fatalf("child exited before boundary: %v %s", err, data)
				default:
				}
				if time.Now().After(deadline) {
					command.Process.Kill()
					<-done
					t.Fatal("child boundary deadline exceeded")
				}
				time.Sleep(10 * time.Millisecond)
			}
			if err = command.Process.Kill(); err != nil {
				t.Fatal(err)
			}
			if err = <-done; err == nil {
				t.Fatal("child was not forcibly interrupted")
			}
			if _, err = database.Exec("UPDATE delivery_runs SET lease_expires_at=1 WHERE status IN ('claimed','running','cancelling'); UPDATE delivery_leases SET expires_at=1 WHERE owner_id IS NOT NULL"); err != nil {
				t.Fatal(err)
			}
			engine.send = func(context.Context, PushplusMessage) (string, error) {
				return "message", appendSyntheticMessage(ledger)
			}
			outcome, err := engine.execute(context.Background(), config, nil, manifestFor("crash-run", 7))
			if err != nil {
				t.Fatal(err)
			}
			wantStatus, wantMessages := "completed", 1
			if boundary == "sending" {
				wantStatus, wantMessages = "unknown", 0
			}
			if boundary == "message-written" {
				wantStatus = "unknown"
			}
			data, readError := os.ReadFile(ledger)
			if readError != nil && !os.IsNotExist(readError) {
				t.Fatal(readError)
			}
			if outcome.Status != wantStatus || strings.Count(string(data), "message\n") != wantMessages || countRows(t, database, "favorites") != 1 {
				t.Fatalf("recovery status=%s messages=%q", outcome.Status, data)
			}
			record, err := engine.repository.LoadDedupe(context.Background(), config.Workflow, config.DbName, 1, 7)
			if err != nil {
				t.Fatal(err)
			}
			wantDedupe := store.DedupeStatusConfirmed
			if wantStatus == "unknown" {
				wantDedupe = store.DedupeStatusUnknown
			}
			if record == nil || record.Status != wantDedupe {
				t.Fatal("recovery dedupe state changed")
			}
		})
	}
}
