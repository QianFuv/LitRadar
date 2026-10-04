package index

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestOriginalRustControlWorkflows(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/index/control-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Name, Setup string
				Operations  []struct {
					Op, Sql, Checkpoint                               string
					Catalog, Provider, Journal, Batch, Run, Timestamp *string
					Mode                                              domain.IndexSyncMode
					Base, Anchor                                      *string
					Resume                                            *bool
					Now                                               int64
					Allow                                             bool
					Aliases                                           []string
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
			database, err := sqlite.OpenMigration(filepath.Join(t.TempDir(), "control.sqlite"))
			if err != nil {
				t.Fatal(err)
			}
			defer database.Close()
			connection, err := database.Conn(ctx)
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			if _, err := connection.ExecContext(ctx, item.Input.Setup); err != nil {
				t.Fatal(err)
			}
			var actual any
			if err := InitControl(ctx, connection); err != nil {
				actual = map[string]any{"error": err.Error()}
			} else {
				operations := []any{}
				for _, operation := range item.Input.Operations {
					defaulted := func(value *string, fallback string) string {
						if value != nil {
							return *value
						}
						return fallback
					}
					catalog := defaulted(operation.Catalog, "catalog")
					provider := defaulted(operation.Provider, "scholarly")
					journal := defaulted(operation.Journal, "catalog:example")
					batch := defaulted(operation.Batch, "batch-1")
					run := defaulted(operation.Run, "run-1")
					timestamp := defaulted(operation.Timestamp, "2026-10-04T00:00:00Z")
					mode := operation.Mode
					if mode == "" {
						mode = domain.Incremental
					}
					ref := SyncRun{SyncScope{catalog, provider, journal}, batch, run, mode, operation.Base}
					var value any
					var err error
					switch operation.Op {
					case "sql":
						_, err = connection.ExecContext(ctx, operation.Sql)
						value = map[string]any{"ok": true}
					case "prepare":
						value, err = PrepareJournalSync(ctx, connection, ref, operation.Resume == nil || *operation.Resume, timestamp)
					case "advance":
						err = AdvanceRunCheckpoint(ctx, connection, ref, operation.Checkpoint, timestamp)
						value = map[string]any{"ok": true}
					case "complete":
						err = CompleteSyncRun(ctx, connection, ref, operation.Anchor, timestamp)
						value = map[string]any{"ok": true}
					case "anchor":
						value, err = ReadSyncAnchor(ctx, connection, ref.Scope)
					case "checkpoint":
						value, err = ReadRunCheckpoint(ctx, connection, ref.Scope)
					case "acquire":
						err = AcquireLease(ctx, connection, catalog, provider, run, operation.Now)
						value = map[string]any{"ok": true}
					case "heartbeat":
						err = HeartbeatLease(ctx, connection, catalog, provider, run, operation.Now)
						value = map[string]any{"ok": true}
					case "release":
						err = ReleaseLease(ctx, connection, catalog, provider, run)
						value = map[string]any{"ok": true}
					case "counts":
						value, err = ReadBatchJournalState(ctx, connection, catalog, provider, batch)
					case "adopt":
						value, err = AdoptLegacyBatchState(ctx, connection, catalog, provider, batch, mode, operation.Allow)
					case "abandon":
						value, err = AbandonBatchCheckpoints(ctx, connection, batch)
					case "aliases":
						value, err = HasCatalogAliasSyncState(ctx, connection, catalog, operation.Aliases)
					default:
						t.Fatal(operation.Op)
					}
					if err != nil {
						value = map[string]any{"error": err.Error()}
					}
					operations = append(operations, value)
				}
				var version int
				if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
					t.Fatal(err)
				}
				actual = map[string]any{"operations": operations, "tables": selectedSnapshot(t, connection, []string{"provider_leases", "provider_sync_anchors", "provider_run_checkpoints"}), "version": version}
			}
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

func TestOrderedCommitErrorRetainsPhase(t *testing.T) {
	for _, phase := range []string{"content", "control"} {
		cause := errors.New("failure")
		err := &ContentCheckpointError{Phase: phase, Cause: cause}
		expected := "content commit failed: failure"
		if phase == "control" {
			expected = "sync progress commit failed: failure"
		}
		if err.Error() != expected || !errors.Is(err, cause) {
			t.Fatalf("phase=%s got=%s", phase, err)
		}
	}
}
